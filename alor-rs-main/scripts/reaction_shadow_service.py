#!/usr/bin/env python3
"""Opt-in bounded Unix supervisor. Never restarts, repairs or deletes a generation."""
import argparse
import datetime as dt
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import time
import tomllib

MAX_MESSAGE = 64 * 1024
LIMITS = dict(startup_timeout_seconds=900, poll_timeout_seconds=120,
              poll_interval_seconds=30, shutdown_timeout_seconds=30,
              max_queue_age_seconds=120, max_model_lateness_seconds=300,
              max_sessions=20)
WORKER_REASONS = {"queue_age", "source_lateness", "missing_model_bar", "readiness_deadline",
                  "incomplete_session_resume", "anchors_not_ready", "storage_maintenance",
                  "source_cursor_missing", "source_cursor_changed", "supervisor_lost", "worker_error"}


class WorkerFailure(RuntimeError):
    def __init__(self, event):
        reason = event.get("reason")
        self.reason = reason if reason in WORKER_REASONS else "worker_error"
        super().__init__("worker failure: " + self.reason)


def read_small(path):
    if Path(path).is_symlink() or not Path(path).is_file():
        raise ValueError("regular supervisor input required")
    with Path(path).open("rb") as handle:
        data = handle.read(MAX_MESSAGE + 1)
    if len(data) > MAX_MESSAGE:
        raise ValueError("oversized supervisor input")
    return data


def file_hash(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as handle:
        for chunk in iter(lambda: handle.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def publish(path, value):
    # A torn file also blocks the next explicit resume. No overwrite or unlink.
    with path.open("x", encoding="utf-8") as handle:
        json.dump(value, handle, sort_keys=True)
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())
    sync_dir(path.parent)


def policy_from_spec(raw):
    spec = tomllib.loads(raw.decode())
    if set(spec) != {"runner", "policy"} or set(spec["policy"]) != set(LIMITS):
        raise ValueError("explicit runner and complete bounded service policy required")
    policy = spec["policy"]
    for key, maximum in LIMITS.items():
        if type(policy[key]) is not int or not 1 <= policy[key] <= maximum:
            raise ValueError("invalid bounded service policy")
    if not policy["poll_interval_seconds"] < policy["max_queue_age_seconds"] <= policy["max_model_lateness_seconds"]:
        raise ValueError("inconsistent poll/queue/model age budgets")
    return policy


class Attempt:
    """One clean STOP permits explicit resume, an incomplete attempt never does."""
    def __init__(self, root, mode, manifest):
        self.root = Path(root)
        if self.root.is_symlink() or self.root.parent.resolve() != self.root.parent:
            raise ValueError("canonical generation path required")
        self.control = self.root.with_name(self.root.name + ".supervisor")
        if mode == "create":
            if self.root.exists():
                raise ValueError("generation already exists")
            self.control.mkdir(mode=0o700)
            sync_dir(self.control.parent)
        elif mode != "resume" or self.control.is_symlink() or not self.control.is_dir():
            raise ValueError("explicit existing supervisor generation required")
        self.lock = os.fdopen(os.open(self.control / "lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600), "r+b")
        try:
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            if mode == "create":
                publish(self.control / "manifest.json", manifest)
            elif json.loads(read_small(self.control / "manifest.json")) != manifest:
                raise ValueError("binary/spec/generation changed; audit required")
            names = set()
            for i, entry in enumerate(self.control.iterdir()):
                if i >= 258 or entry.is_symlink() or not entry.is_file():
                    raise ValueError("oversized or invalid supervisor directory")
                names.add(entry.name)
            names -= {"lock", "manifest.json"}
            index = 1
            while f"{index:04d}.started.json" in names:
                start = f"{index:04d}.started.json"
                stop = f"{index:04d}.stopped.json"
                if stop not in names:
                    raise ValueError("incomplete/failed previous attempt; automatic recovery forbidden")
                before = json.loads(read_small(self.control / start))
                after = json.loads(read_small(self.control / stop))
                if before.get("attempt") != index or after.get("attempt") != index or after.get("reason") != "operator_stop":
                    raise ValueError("completed/invalid previous attempt cannot resume")
                names -= {start, stop}
                index += 1
            if names or index > 128:
                raise ValueError("unknown or excessive supervisor attempts")
            self.index = index
        except BaseException:
            self.lock.close()
            raise

    def start(self, ready_by):
        publish(self.control / f"{self.index:04d}.started.json",
                dict(attempt=self.index, ready_by_utc=ready_by, started_at_utc=time.time(),
                     supervisor_pid=os.getpid(),
                     order_emission_allowed=False))

    def stopped(self, reason, status):
        publish(self.control / f"{self.index:04d}.stopped.json",
                dict(attempt=self.index, reason=reason, stopped_at_utc=time.time(),
                     status=status, order_emission_allowed=False))

    def close(self):
        self.lock.close()


class StopRequest:
    def __init__(self):
        self.at = None

    def request(self, *_):
        if self.at is None:
            self.at = time.monotonic()


class Channel:
    def __init__(self, process):
        self.process = process
        self.buffer = b""
        self.selector = selectors.DefaultSelector()
        self.selector.register(process.stdout, selectors.EVENT_READ)

    def receive(self, deadline, stop, shutdown_seconds):
        while True:
            now = time.monotonic()
            if stop.at is not None:
                deadline = min(deadline, stop.at + shutdown_seconds)
            if now >= deadline:
                raise TimeoutError("worker deadline exceeded; generation requires audit")
            if b"\n" in self.buffer:
                line, self.buffer = self.buffer.split(b"\n", 1)
                if self.buffer or not line or len(line) > MAX_MESSAGE:
                    raise ValueError("unsolicited/oversized worker output")
                event = json.loads(line)
                if not isinstance(event, dict) or event.get("order_emission_allowed") is not False:
                    raise ValueError("worker protocol/order prohibition failed")
                return event
            if not self.selector.select(min(.1, deadline - now)):
                continue
            chunk = os.read(self.process.stdout.fileno(), 4096)
            if not chunk:
                raise RuntimeError("worker EOF before acknowledged boundary")
            self.buffer += chunk
            if len(self.buffer) > MAX_MESSAGE:
                raise ValueError("worker message exceeded bound")

    def send(self, request):
        self.process.stdin.write(request.encode() + b"\n")
        self.process.stdin.flush()

    def close(self):
        self.selector.close()


def status_of(event, expected):
    if event.get("event") == "failed":
        raise WorkerFailure(event)
    if event.get("event") != expected:
        raise RuntimeError("worker failed or unexpected protocol event")
    status = event.get("status", {})
    runner = status.get("runner", {})
    if (status.get("order_emission_allowed") is not False
            or runner.get("order_emission_allowed") is not False
            or runner.get("failed") is not False
            or type(status.get("armed")) is not bool
            or type(status.get("completed")) is not bool):
        raise RuntimeError("invalid/failed worker status")
    return status


def signal_owned_group(process, sig):
    try:
        os.killpg(process.pid, sig)
    except ProcessLookupError:
        pass
    except PermissionError as denied:
        # An exiting child can race poll/killpg (observed on macOS). Only accept
        # a confirmed reap; a still-running child with denied signals is fatal.
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            raise denied


def terminate_owned(process):
    if process.poll() is None:
        signal_owned_group(process, signal.SIGTERM)
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            signal_owned_group(process, signal.SIGKILL)
            process.wait()


def supervise(command, policy, ready_by, attempt, env, spec_sha, stop=None, emit=print):
    stop = stop or StopRequest()
    started = time.monotonic()
    remaining = ready_by - time.time()
    if not math.isfinite(remaining) or remaining <= 0:
        raise ValueError("readiness deadline expired before spawn")
    startup_deadline = started + min(policy["startup_timeout_seconds"], remaining)
    attempt.start(ready_by)  # survives timeout, crash, supervisor SIGKILL or host reboot
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, env=env, start_new_session=True)
    channel = Channel(process)
    try:
        opened = channel.receive(startup_deadline, stop, policy["shutdown_timeout_seconds"])
        status = status_of(opened, "opened")
        if opened.get("spec_sha256") != spec_sha:
            raise ValueError("worker read different launch spec")
        emit(json.dumps(dict(event="reaction_shadow_opened", status=status)))
        last_log = time.monotonic()
        last_progress = None
        while stop.at is None and not status["completed"]:
            deadline = (time.monotonic() + policy["poll_timeout_seconds"]
                        if status["armed"] else startup_deadline)
            channel.send("poll")
            status = status_of(channel.receive(deadline, stop, policy["shutdown_timeout_seconds"]), "polled")
            progress = (status["runner"].get("committed_transactions"),
                        status["runner"].get("source_caught_up"), status["armed"],
                        status.get("resource_state"))
            if progress != last_progress or time.monotonic() - last_log >= 60:
                emit(json.dumps(dict(event="reaction_shadow_status", status=status)))
                last_log, last_progress = time.monotonic(), progress
            if status.get("resource_state") == "maintenance_required":
                raise RuntimeError("generation reached maintenance threshold")
            # Catch up without poll sleeps; prospective normal polling is paced.
            delay = policy["poll_interval_seconds"] if status["armed"] else 0
            until = time.monotonic() + delay
            while stop.at is None and not status["completed"] and time.monotonic() < until:
                time.sleep(min(.1, max(0, until - time.monotonic())))
        reason = "session_budget_complete" if status["completed"] else "operator_stop"
        if stop.at is None:
            stop.request()
        channel.send("stop")
        final = status_of(channel.receive(stop.at + policy["shutdown_timeout_seconds"],
                                         stop, policy["shutdown_timeout_seconds"]), "stopped")
        if process.wait(timeout=max(.001, stop.at + policy["shutdown_timeout_seconds"] - time.monotonic())) != 0:
            raise RuntimeError("worker exit was not clean")
        attempt.stopped(reason, final)
        emit(json.dumps(dict(event="reaction_shadow_stopped", reason=reason, status=final)))
        return final
    finally:
        try:
            terminate_owned(process)
        finally:
            channel.close()
            process.stdin.close()
            process.stdout.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("create", "resume"))
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--spec", type=Path, required=True)
    parser.add_argument("--generation", type=Path, required=True)
    parser.add_argument("--ready-by", required=True, help="explicit timezone-aware ISO timestamp before next cutoff")
    args = parser.parse_args()
    ready = dt.datetime.fromisoformat(args.ready_by)
    if ready.tzinfo is None or ready.microsecond:
        raise ValueError("whole-second readiness timestamp with timezone required")
    ready_by = int(ready.timestamp())
    root = args.generation.absolute()
    worker, spec = args.worker.resolve(strict=True), args.spec.resolve(strict=True)
    raw = read_small(spec)
    policy = policy_from_spec(raw)
    spec_sha = hashlib.sha256(raw).hexdigest()
    if not os.environ.get("REACTION_SHADOW_REDIS_URL"):
        raise ValueError("dedicated read-only REACTION_SHADOW_REDIS_URL required")
    if ready_by <= time.time():
        raise ValueError("readiness deadline must be future")
    # Do not pass broker tokens, live config or generic REDIS_URL to the worker.
    env = {"REACTION_SHADOW_REDIS_URL": os.environ["REACTION_SHADOW_REDIS_URL"], "TZ": "UTC"}
    manifest = dict(version="reaction_shadow_supervisor_v1", generation=str(root),
                    binary_sha256=file_hash(worker), spec_sha256=spec_sha)
    attempt = Attempt(root, args.mode, manifest)
    stop = StopRequest()
    old = {sig: signal.signal(sig, stop.request) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        supervise([str(worker), args.mode, str(spec), str(root), str(ready_by)],
                  policy, ready_by, attempt, env, spec_sha, stop,
                  emit=lambda value: print(value, flush=True))
    finally:
        for sig, handler in old.items():
            signal.signal(sig, handler)
        attempt.close()


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # Never echo command args, Redis URLs, worker payloads or credential values.
        print(json.dumps(dict(event="reaction_shadow_supervisor_failed",
                              error_class=type(error).__name__,
                              reason=getattr(error, "reason", "supervisor_error"),
                              error_sha256=hashlib.sha256(str(error).encode()).hexdigest(),
                              action="retain_and_audit_no_automatic_restart")), flush=True)
        sys.exit(1)
