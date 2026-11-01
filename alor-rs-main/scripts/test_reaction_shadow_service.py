import json
import os
from pathlib import Path
import sys
import subprocess
import tempfile
import time
import unittest
from unittest import mock

import reaction_shadow_service as service


POLICY = dict(startup_timeout_seconds=2, poll_timeout_seconds=1,
              poll_interval_seconds=1, shutdown_timeout_seconds=1,
              max_queue_age_seconds=10, max_model_lateness_seconds=60, max_sessions=2)

# Worker fixture has no Redis or strategy and can only echo a bounded protocol.
WORKER = r'''
import json, os, sys, time
from pathlib import Path
mode, root = sys.argv[1:]
Path(root).with_suffix(".pid").write_text(str(os.getpid()))
s = dict(runner=dict(failed=False, order_emission_allowed=False, committed_transactions=0,
                     source_caught_up=True), order_emission_allowed=False,
         armed=False, completed=False, resource_state="normal")
def emit(event):
    print(json.dumps(dict(event=event, spec_sha256="fixture", status=s,
                          order_emission_allowed=False)), flush=True)
if mode == "opening_hang": time.sleep(30)
if mode == "overflow":
    print("a" * 70000, flush=True)
    time.sleep(30)
if mode == "bad_spec":
    print(json.dumps(dict(event="opened", spec_sha256="wrong", status=s,
                         order_emission_allowed=False)), flush=True)
else: emit("opened")
for line in sys.stdin:
    if line == "poll\n":
        if mode == "poll_hang": time.sleep(30)
        if mode == "crash": sys.exit(7)
        if mode == "orders": s["runner"]["order_emission_allowed"] = True
        s["armed"] = mode != "never_ready"
        s["completed"] = mode == "complete"
        s["runner"]["committed_transactions"] += 1
        emit("polled")
    elif line == "stop\n":
        if mode == "stop_hang": time.sleep(30)
        emit("stopped")
        sys.exit(0)
sys.exit(8)
'''


class SupervisorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve() / "generation"
        self.manifest = dict(binary_sha256="fixture", spec_sha256="fixture", generation=str(self.root))

    def tearDown(self):
        self.tmp.cleanup()

    def attempt(self, mode="create"):
        return service.Attempt(self.root, mode, self.manifest)

    def run_worker(self, mode, attempt, stop_on_status=True):
        stop = service.StopRequest()
        events = []
        def emit(line):
            events.append(json.loads(line))
            if stop_on_status and events[-1]["event"] == "reaction_shadow_status":
                stop.request()
        try:
            result = service.supervise([sys.executable, "-u", "-c", WORKER, mode, str(self.root)],
                                      POLICY, time.time() + 10, attempt, {}, "fixture", stop, emit)
        finally:
            pid_path = self.root.with_suffix(".pid")
            if pid_path.exists():
                with self.assertRaises(ProcessLookupError):
                    os.kill(int(pid_path.read_text()), 0)
        return result, events

    def test_clean_stop_and_explicit_resume(self):
        first = self.attempt()
        try:
            final, events = self.run_worker("ok", first)
            self.assertTrue(final["armed"])
            self.assertEqual(events[-1]["reason"], "operator_stop")
        finally:
            first.close()
        again = self.attempt("resume")
        try:
            self.assertEqual(again.index, 2)
            self.run_worker("ok", again)
        finally:
            again.close()

    def test_faults_latch_and_reap_worker(self):
        for mode in ("opening_hang", "poll_hang", "stop_hang", "crash", "orders", "bad_spec", "overflow", "never_ready"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory(dir=self.tmp.name) as directory:
                self.root = Path(directory).resolve() / "generation"
                self.manifest["generation"] = str(self.root)
                attempt = self.attempt()
                try:
                    with self.assertRaises((TimeoutError, RuntimeError, ValueError)):
                        self.run_worker(mode, attempt, stop_on_status=mode != "never_ready")
                finally:
                    attempt.close()
                with self.assertRaisesRegex(ValueError, "incomplete/failed"):
                    self.attempt("resume")
                self.assertFalse((attempt.control / "0001.stopped.json").exists())

    def test_supervisor_death_marker_and_single_writer(self):
        attempt = self.attempt()
        try:
            with self.assertRaises(BlockingIOError):
                self.attempt("resume")
            attempt.start(time.time() + 10)
        finally:
            attempt.close()
        with self.assertRaisesRegex(ValueError, "incomplete/failed"):
            self.attempt("resume")

    def test_completed_generation_no_restart(self):
        attempt = self.attempt()
        try:
            final, _ = self.run_worker("complete", attempt)
            self.assertTrue(final["completed"])
        finally:
            attempt.close()
        with self.assertRaisesRegex(ValueError, "cannot resume"):
            self.attempt("resume")

    def test_no_configuration_substitution_or_unknown_files(self):
        attempt = self.attempt()
        try:
            self.run_worker("ok", attempt)
        finally:
            attempt.close()
        self.manifest["spec_sha256"] = "changed"
        with self.assertRaisesRegex(ValueError, "changed"):
            self.attempt("resume")
        self.manifest["spec_sha256"] = "fixture"
        (attempt.control / "stray").touch()
        with self.assertRaisesRegex(ValueError, "unknown"):
            self.attempt("resume")

    def test_bad_policy_and_deadline_create_no_attempt(self):
        for key in POLICY:
            for invalid in (0, True, float("nan"), service.LIMITS[key] + 1):
                policy = {**POLICY, key: invalid}
                raw = "[runner]\n[policy]\n" + "\n".join(f"{k} = {str(v).lower()}" for k, v in policy.items())
                with self.assertRaises(ValueError):
                    service.policy_from_spec(raw.encode())
        attempt = self.attempt()
        try:
            with self.assertRaises(ValueError):
                service.supervise(["not-executed"], POLICY, time.time()-1, attempt, {}, "fixture")
            self.assertFalse((attempt.control / "0001.started.json").exists())
        finally:
            attempt.close()

    def test_termination_exit_race_reaps_confirmed_child(self):
        child = mock.Mock(pid=123, poll=mock.Mock(return_value=None),
                          wait=mock.Mock(return_value=1))
        with mock.patch.object(service.os, "killpg", side_effect=PermissionError):
            service.terminate_owned(child)
        child.wait.assert_any_call(timeout=2)

    def test_signal_denial_for_running_child_is_not_swallowed(self):
        child = mock.Mock(pid=123, poll=mock.Mock(return_value=None),
                          wait=mock.Mock(side_effect=subprocess.TimeoutExpired("fixture", 2)))
        with mock.patch.object(service.os, "killpg", side_effect=PermissionError):
            with self.assertRaises(PermissionError):
                service.terminate_owned(child)

    def test_worker_failure_survives_exit_race_and_attempt_stays_failed(self):
        attempt = self.attempt()
        child = mock.Mock(pid=123, poll=mock.Mock(return_value=None),
                          wait=mock.Mock(return_value=1))
        channel = mock.Mock()
        channel.receive.return_value = dict(event="failed", reason="source_cursor_missing")
        try:
            with mock.patch.object(service.subprocess, "Popen", return_value=child), \
                    mock.patch.object(service, "Channel", return_value=channel), \
                    mock.patch.object(service.os, "killpg", side_effect=PermissionError):
                with self.assertRaisesRegex(service.WorkerFailure, "source_cursor_missing"):
                    service.supervise(["fixture"], POLICY, time.time()+10, attempt, {}, "fixture")
            channel.close.assert_called_once()
            child.stdin.close.assert_called_once()
            child.stdout.close.assert_called_once()
            self.assertFalse((attempt.control / "0001.stopped.json").exists())
        finally:
            attempt.close()
        with self.assertRaisesRegex(ValueError, "incomplete/failed"):
            self.attempt("resume")

    def test_channels_close_and_attempt_latches_on_termination_error(self):
        attempt = self.attempt()
        child, channel = mock.Mock(), mock.Mock()
        channel.receive.side_effect = RuntimeError("fixture protocol failure")
        try:
            with mock.patch.object(service.subprocess, "Popen", return_value=child), \
                    mock.patch.object(service, "Channel", return_value=channel), \
                    mock.patch.object(service, "terminate_owned", side_effect=PermissionError):
                with self.assertRaises(PermissionError):
                    service.supervise(["fixture"], POLICY, time.time()+10, attempt, {}, "fixture")
            channel.close.assert_called_once()
            child.stdin.close.assert_called_once()
            child.stdout.close.assert_called_once()
            self.assertFalse((attempt.control / "0001.stopped.json").exists())
        finally:
            attempt.close()
        with self.assertRaisesRegex(ValueError, "incomplete/failed"):
            self.attempt("resume")


if __name__ == "__main__":
    unittest.main()
