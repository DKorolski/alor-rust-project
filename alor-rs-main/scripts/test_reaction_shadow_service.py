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

    def run_worker(self, mode, attempt, stop_on_status=True, source_guard=None, on_event=None):
        stop = service.StopRequest()
        events = []
        def emit(line):
            events.append(json.loads(line))
            if on_event:
                on_event(events[-1])
            if stop_on_status and events[-1]["event"] == "reaction_shadow_status":
                stop.request()
        try:
            result = service.supervise([sys.executable, "-u", "-c", WORKER, mode, str(self.root)],
                                      POLICY, time.time() + 10, attempt, {}, "fixture", stop, emit,
                                      source_guard=source_guard)
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

    def source(self):
        root = self.root.parent / "source"
        root.mkdir()
        cfg = dict(symbol="IMOEXF", stream="shadow.source.closed10m.IMOEXF.fixture",
                   bootstrap_start="2026-10-28", first_live_session="2026-11-02",
                   excluded_dates=[], revision_policy="imoexf_author41_short_bo053_volume_audit")
        manifest = dict(version="shadow_authenticated_rest_closed_m10_v3", config=cfg)
        (root / "manifest.json").write_text(json.dumps(manifest))
        lock = (root / "writer.lock").open("a+b")
        self.addCleanup(lock.close)
        service.fcntl.flock(lock, service.fcntl.LOCK_EX | service.fcntl.LOCK_NB)
        spec = b'''[runner]
first_decision_session = "2026-11-02"
excluded_dates = []
[runner.ingress]
symbol = "IMOEXF"
source_stream = "shadow.source.closed10m.IMOEXF.fixture"
[runner.config]
engine = "imoexf"
'''
        return root, lock, spec

    def test_source_guard_requires_running_writer_and_unchanged_identity(self):
        root, lock, spec = self.source()
        guard = service.ClosedSourceGuard(root, spec)
        guard.check()
        with (root / "FAILED.json").open("w") as handle:
            json.dump(dict(reason="price_revision"), handle)
        with self.assertRaises(service.SourceUnavailable):
            guard.check()

        (root / "FAILED.json").unlink()
        service.fcntl.flock(lock, service.fcntl.LOCK_UN)
        with self.assertRaisesRegex(service.SourceUnavailable, "not running"):
            guard.check()

    def ws_source(self, v2=False, v3=False):
        root, lock, spec = self.source()
        cfg = json.loads((root / "manifest.json").read_text())["config"]
        del cfg["revision_policy"]
        cfg.update(gateway_config="/fixture/gateway.toml", gateway_config_sha256="a" * 64,
                   terminal=dict(enabled=True, symbols=dict(IMOEXF={}), max_heartbeat_age_seconds=45))
        if v2 or v3:
            cfg["context_finalization"] = True
        if v3:
            cfg["ordinary_observation_max_lateness_seconds"] = 90
            cfg["terminal"]["max_lateness_seconds"] = 90
        sha = service.hashlib.sha256(json.dumps(cfg, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        version = "shadow_gateway_ws_closed_m10_v3" if v3 else "shadow_gateway_ws_closed_m10_v2" if v2 else "shadow_gateway_ws_closed_m10_v1"
        (root / "manifest.json").write_text(json.dumps(dict(version=version, config=cfg, config_sha256=sha)))
        health = dict(version=version, config_sha256=sha, status="ready", generation=1,
                      heartbeat_utc=int(time.time()), last_ws_rx_utc=int(time.time()),
                      connected=True, subscribed=True, history_complete=True, order_emission_allowed=False)
        if v3:
            health.update(ordinary_delayed_rows=0, model_freshness_policy="consumer_owned_unchanged")
        (root / "HEALTH.json").write_text(json.dumps(health))
        return root, lock, spec, health

    def test_ws_v3_checks_explicit_collection_policy_and_no_freshness_claim(self):
        root, _, spec, health = self.ws_source(v3=True)
        guard = service.ClosedSourceGuard(root, spec)
        delayed = {**health, "ordinary_delayed_rows": 1}
        (root / "HEALTH.json").write_text(json.dumps(delayed))
        guard.check()  # Collection liveness is not model freshness admission.
        for key, value in (("model_freshness_policy", "source_waives_model_age"), ("ordinary_delayed_rows", True)):
            (root / "HEALTH.json").write_text(json.dumps({**health, key: value}))
            with self.assertRaises(service.SourceUnavailable):
                guard.check()
        (root / "HEALTH.json").write_text(json.dumps(health))
        manifest = json.loads((root / "manifest.json").read_text())
        for value in (True, 0, 89, 301, None):
            manifest["config"]["ordinary_observation_max_lateness_seconds"] = value
            (root / "manifest.json").write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, "bounded ordinary"):
                service.ClosedSourceGuard(root, spec)

    def test_ws_v2_requires_opt_in_and_preserves_health_guard(self):
        root, _, spec, health = self.ws_source(v2=True)
        guard = service.ClosedSourceGuard(root, spec)
        (root / "HEALTH.json").write_text(json.dumps({**health, "generation": 0}))
        with self.assertRaises(service.SourceUnavailable):
            guard.check()
        (root / "HEALTH.json").write_text(json.dumps(health))
        manifest = json.loads((root / "manifest.json").read_text())
        for value in (False, None, 1):
            manifest["config"]["context_finalization"] = value
            (root / "manifest.json").write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, "explicit context"):
                service.ClosedSourceGuard(root, spec)
        del manifest["config"]["context_finalization"]
        (root / "manifest.json").write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "explicit context"):
            service.ClosedSourceGuard(root, spec)

    def test_ws_v1_does_not_accept_v2_context_policy(self):
        root, _, spec, _ = self.ws_source()
        manifest = json.loads((root / "manifest.json").read_text())
        manifest["config"]["context_finalization"] = True
        (root / "manifest.json").write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "unexpected WS source schema"):
            service.ClosedSourceGuard(root, spec)

    def test_ws_guard_rejects_stale_health_identity_and_unready_admission(self):
        root, _, spec, health = self.ws_source()
        guard = service.ClosedSourceGuard(root, spec)
        for key, value in (("heartbeat_utc", int(time.time()) - 20),
                           ("last_ws_rx_utc", int(time.time()) - 46),
                           ("config_sha256", "b" * 64), ("generation", 0),
                           ("order_emission_allowed", True), ("status", "failed"),
                           ("connected", False), ("subscribed", False), ("history_complete", False)):
            with self.subTest(key=key):
                (root / "HEALTH.json").write_text(json.dumps({**health, key: value}))
                with self.assertRaises(service.SourceUnavailable):
                    guard.check()
        reconnect = {**health, "status": "reconnecting", "history_complete": False, "connected": False}
        (root / "HEALTH.json").write_text(json.dumps(reconnect))
        guard.check()
        with self.assertRaises(service.SourceUnavailable):
            service.ClosedSourceGuard(root, spec)

    def test_ws_guard_rejects_generation_rollback_and_missing_health(self):
        root, _, spec, health = self.ws_source()
        guard = service.ClosedSourceGuard(root, spec)
        (root / "HEALTH.json").write_text(json.dumps({**health, "generation": 2}))
        guard.check()
        (root / "HEALTH.json").write_text(json.dumps(health))
        with self.assertRaises(service.SourceUnavailable):
            guard.check()
        (root / "HEALTH.json").unlink()
        with self.assertRaises(service.SourceUnavailable):
            guard.check()

    def test_source_guard_rejects_unreadable_or_changed_manifest(self):
        root, _, spec = self.source()
        guard = service.ClosedSourceGuard(root, spec)
        (root / "manifest.json").write_text('{}')
        with self.assertRaises(service.SourceUnavailable):
            guard.check()
        (root / "manifest.json").unlink()
        with self.assertRaises(service.SourceUnavailable):
            guard.check()

    def test_source_guard_binds_calendar_stream_and_model_volume_policy(self):
        root, _, spec = self.source()
        for old, new in ((b'2026-11-02', b'2026-11-03'),
                         (b'IMOEXF.fixture', b'IMOEXF.other'),
                         (b'excluded_dates = []', b'excluded_dates = ["2026-11-04"]'),
                         (b'engine = "imoexf"', b'engine = "usdrubf"')):
            with self.subTest(new=new), self.assertRaises(ValueError):
                service.ClosedSourceGuard(root, spec.replace(old, new))

    def test_source_failure_after_open_latches_attempt_and_reaps_worker(self):
        root, _, spec = self.source()
        guard = service.ClosedSourceGuard(root, spec)
        attempt = self.attempt()
        events = []
        def failed_after_open(event):
            events.append(event)
            if event['event'] == 'reaction_shadow_opened':
                (root / "FAILED.json").write_text('{"reason":"price_revision"}')
        try:
            with self.assertRaises(service.SourceUnavailable):
                self.run_worker('ok', attempt, source_guard=guard, on_event=failed_after_open)
            self.assertEqual([e['event'] for e in events], ['reaction_shadow_opened'])
            self.assertFalse((attempt.control / '0001.stopped.json').exists())
        finally:
            attempt.close()
        with self.assertRaisesRegex(ValueError, 'incomplete/failed'):
            self.attempt('resume')

    def test_source_failure_before_spawn_does_not_start_attempt(self):
        root, _, spec = self.source()
        guard = service.ClosedSourceGuard(root, spec)
        (root / 'FAILED.json').write_text('{}')
        attempt = self.attempt()
        try:
            with mock.patch.object(service.subprocess, 'Popen') as spawn:
                with self.assertRaises(service.SourceUnavailable):
                    self.run_worker('ok', attempt, source_guard=guard)
                spawn.assert_not_called()
            self.assertFalse((attempt.control / '0001.started.json').exists())
        finally:
            attempt.close()

    def test_guard_interrupts_wait_for_unresponsive_worker(self):
        attempt = self.attempt()
        guard = mock.Mock()
        # Startup check + first receive check pass; the next periodic receive
        # check fails while the worker is silent, before the worker timeout.
        guard.check.side_effect = [None, None, service.SourceUnavailable('source stopped')]
        try:
            with self.assertRaises(service.SourceUnavailable):
                self.run_worker('opening_hang', attempt, source_guard=guard)
            self.assertFalse((attempt.control / '0001.stopped.json').exists())
        finally:
            attempt.close()

    def test_symlinked_or_missing_source_lock_is_not_readiness(self):
        root, lock, spec = self.source()
        guard = service.ClosedSourceGuard(root, spec)
        lock.close()
        (root / 'writer.lock').unlink()
        with self.assertRaises(service.SourceUnavailable):
            guard.check()
        (root / 'writer.lock').symlink_to(root / 'manifest.json')
        with self.assertRaises(service.SourceUnavailable):
            guard.check()

    def test_closed_source_cli_cannot_omit_dependency_guard(self):
        _, _, raw = self.source()
        raw += b'\n[policy]\n' + '\n'.join(f'{k} = {v}' for k, v in POLICY.items()).encode()
        spec = self.root.parent / 'spec.toml'
        spec.write_bytes(raw)
        ready = service.dt.datetime.fromtimestamp(time.time() + 60, service.dt.timezone.utc).replace(microsecond=0).isoformat()
        argv = ['service', 'create', '--worker', sys.executable, '--spec', str(spec),
                '--generation', str(self.root), '--ready-by', ready]
        with mock.patch.object(sys, 'argv', argv), mock.patch.object(service.subprocess, 'Popen') as spawn:
            with self.assertRaisesRegex(ValueError, '--source-state'):
                service.main()
            spawn.assert_not_called()
        self.assertFalse(self.root.exists())
        self.assertFalse(self.root.with_name(self.root.name + '.supervisor').exists())

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
