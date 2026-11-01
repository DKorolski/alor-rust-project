"""Explicit local real-worker/ACL smoke. Never uses REDIS_URL or a remote host."""
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import unittest

import reaction_shadow_service as service


@unittest.skipUnless(os.environ.get("REACTION_WORKER_BIN"), "set built REACTION_WORKER_BIN for owned local integration")
class RealWorkerSmoke(unittest.TestCase):
    def test_real_worker_readonly_create_stop_resume_and_cursor_failure(self):
        self._run_smoke(bootstrap=False)

    def test_real_worker_bootstrap_inspection_create_stop_resume(self):
        self._run_smoke(bootstrap=True)

    def _run_smoke(self, *, bootstrap):
        worker = Path(os.environ["REACTION_WORKER_BIN"]).resolve(strict=True)
        redis_server = os.environ.get("REDIS_SERVER_BIN") or shutil.which("redis-server")
        redis_cli = os.environ.get("REDIS_CLI_BIN") or shutil.which("redis-cli")
        self.assertTrue(redis_server and redis_cli, "local Redis tools required")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            with socket.socket() as reserved:
                reserved.bind(("127.0.0.1", 0))
                port = reserved.getsockname()[1]
            server = subprocess.Popen([redis_server, "--bind", "127.0.0.1", "--port", str(port),
                                       "--save", "", "--appendonly", "no", "--dir", str(root)],
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            def redis(*args, reader=False):
                env = {**os.environ, "REDISCLI_AUTH": "fixture-password"} if reader else os.environ
                result = subprocess.run([redis_cli, "-h", "127.0.0.1", "-p", str(port),
                                         *(["--user", "reaction_reader"] if reader else []),
                                         "--raw", *args], env=env, capture_output=True, text=True,
                                        timeout=3, check=True)
                return result.stdout.strip()
            try:
                for _ in range(100):
                    if server.poll() is not None:
                        self.fail("owned Redis exited")
                    try:
                        info = redis("INFO", "server")
                        pid = next(int(line.split(":")[1]) for line in info.splitlines() if line.startswith("process_id:"))
                        self.assertEqual(pid, server.pid, "refusing unrelated Redis process")
                        break
                    except (subprocess.CalledProcessError, StopIteration):
                        time.sleep(.02)
                else:
                    self.fail("owned Redis startup timed out")
                redis("ACL", "SETUSER", "reaction_reader", "on", ">fixture-password", "~fixture:bars",
                      "-@all", "+multi", "+exec", "+xrange", "+ping")
                self.assertIn("NOPERM", redis("XADD", "fixture:commands", "*", "payload", "forbidden", reader=True))
                msk = dt.timezone(dt.timedelta(hours=3))
                activation = dt.datetime.now(msk).date() + dt.timedelta(days=3)
                while activation.weekday() >= 5:
                    activation += dt.timedelta(days=1)
                first = (activation - dt.timedelta(days=8))
                while first.weekday() >= 5:
                    first -= dt.timedelta(days=1)
                label = int(dt.datetime.combine(first, dt.time(7), msk).timestamp())
                payload = dict(schema_version=1, ts_utc=label+600, source="owned-test", msg_type="bar",
                               payload=dict(symbol="IMOEXF", close_time_utc=label, o=100., h=102.,
                                            l=98., close=101., v=1., origin="history"))
                cursor = redis("XADD", "fixture:bars", "*", "payload", json.dumps(payload))
                raw = (Path(__file__).resolve().parents[1] / "configs/examples/reaction-shadow.imoexf-combo.toml").read_text()
                raw = raw.replace('"2026-11-02"', json.dumps(str(activation))).replace("REPLACE_WITH_APPROVED_IMOEXF_M10_SOURCE", "fixture:bars")
                spec = root / "launch.toml"
                expected_receipts = 1
                expected_stream_rows = 1
                env = {"REACTION_SHADOW_REDIS_URL": f"redis://reaction_reader:fixture-password@127.0.0.1:{port}/", "TZ": "UTC"}
                if bootstrap:
                    # A synthetic explicit calendar keeps this pre-cutoff fixture
                    # valid at any wall time without manufacturing today's close.
                    today = dt.datetime.now(msk).date()
                    activation = today + dt.timedelta(days=1)
                    while activation.weekday() >= 5:
                        activation += dt.timedelta(days=1)
                    days = []
                    day = today - dt.timedelta(days=1)
                    while len(days) < 2:
                        if day.weekday() < 5:
                            days.append(day)
                        day -= dt.timedelta(days=1)
                    raw = (Path(__file__).resolve().parents[1] / "configs/examples/reaction-shadow.imoexf-combo.toml").read_text()
                    raw = raw.replace('"2026-11-02"', json.dumps(str(activation))).replace("REPLACE_WITH_APPROVED_IMOEXF_M10_SOURCE", "fixture:bars")
                    raw = raw.replace("excluded_dates = []", "excluded_dates = " + json.dumps([str(today)]))
                    first_cursor = None
                    for day in reversed(days):
                        for slot in range(101):
                            label = int(dt.datetime.combine(day, dt.time(7), msk).timestamp()) + slot * 600
                            payload["ts_utc"] = label + 600
                            payload["payload"]["close_time_utc"] = label
                            cursor = redis("XADD", "fixture:bars", "*", "payload", json.dumps(payload))
                            first_cursor = first_cursor or cursor
                    spec.write_text(raw)
                    result = subprocess.run([str(worker), "inspect-bootstrap", str(spec), first_cursor, cursor],
                                            env=env, capture_output=True, text=True, timeout=15, check=True)
                    preview = json.loads(result.stdout)["preview"]
                    self.assertEqual(preview["bootstrap"]["rows"], 202)
                    self.assertTrue(all(len(anchors) == 2 for anchors in preview["anchors"]))
                    self.assertFalse(preview["order_emission_allowed"])
                    raw += "\n[runner.bootstrap]\n" + "\n".join(
                        f"{key} = {json.dumps(value)}" for key, value in preview["bootstrap"].items()) + "\n"
                    expected_receipts = 202
                    expected_stream_rows = 203
                spec.write_text(raw)
                policy = service.policy_from_spec(raw.encode())
                generation = root / "generation"
                spec_sha = hashlib.sha256(raw.encode()).hexdigest()
                manifest = dict(binary_sha256=service.file_hash(worker), spec_sha256=spec_sha, generation=str(generation))
                def run(mode, *, wait_until_ready=True):
                    attempt = service.Attempt(generation, mode, manifest)
                    stop = service.StopRequest()
                    def log(line):
                        event = json.loads(line)
                        if event["event"] == "reaction_shadow_status" and (
                                not wait_until_ready or event["status"]["armed"]):
                            stop.request()
                    ready_by = int(time.time()) + 60
                    try:
                        return service.supervise([str(worker), mode, str(spec), str(generation), str(ready_by)],
                                                 policy, ready_by, attempt, env, spec_sha, stop, log)
                    finally:
                        attempt.close()
                if bootstrap:
                    partial = run("create", wait_until_ready=False)
                    self.assertEqual(partial["runner"]["bootstrap_status"], "pending")
                    self.assertEqual(partial["runner"]["durable_receipts"], 128)
                    self.assertEqual(partial["runner"]["committed_transactions"], 0)
                    before = run("resume")
                else:
                    before = run("create")
                after = run("resume")
                self.assertEqual(before["head_sha256"], after["head_sha256"])
                self.assertEqual(before["runner"]["committed_transactions"], after["runner"]["committed_transactions"])
                self.assertEqual(after["runner"]["durable_receipts"], expected_receipts)
                self.assertEqual(after["runner"]["bootstrap_status"], "loaded" if bootstrap else "disabled")
                self.assertFalse(after["order_emission_allowed"])
                self.assertEqual(redis("XLEN", "fixture:bars"), str(expected_stream_rows))
                self.assertEqual(redis("XINFO", "GROUPS", "fixture:bars"), "")
                self.assertEqual(redis("EXISTS", "fixture:commands"), "0")
                redis("XDEL", "fixture:bars", cursor)
                with self.assertRaises(RuntimeError):
                    run("resume")
                with self.assertRaisesRegex(ValueError, "incomplete/failed"):
                    service.Attempt(generation, "resume", manifest)
            finally:
                server.terminate()
                server.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
