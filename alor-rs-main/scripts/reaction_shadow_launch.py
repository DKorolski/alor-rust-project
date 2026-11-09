#!/usr/bin/env python3
"""Opt-in v3 create launcher: check admission, durably record grant, then spawn."""
import argparse
import hashlib
import json
from pathlib import Path
import signal
import sys
import time

import reaction_shadow_preflight as preflight
import reaction_shadow_service as service


def launch(plan_path, plan_sha, ready_by, *, environ=None, clock=None, stop=None, emit=print):
    environment = dict(preflight.os.environ if environ is None else environ)
    raw_plan = preflight.small(plan_path)
    preflight.require(preflight.digest(raw_plan) == plan_sha, 'preflight_plan_changed')
    plan = json.loads(raw_plan)
    preflight.require(plan.get('version') == 'reaction_shadow_preflight_v3', 'grant_launcher_requires_v3')
    result = preflight.preflight(plan_path, plan_sha, 'admission', environment,
                                now=int(clock()) if clock else None)
    grant = result['admission_grant']
    preflight.require(result['admission_allowed'] and grant['order_emission_allowed'] is False,
                      'grant_admission_required')
    root = Path(plan['paths']['generation'])
    spec = preflight.regular(plan['paths']['approved_spec'])
    worker = preflight.regular(plan['files']['worker']['path'])
    raw_spec = preflight.small(spec)
    preflight.require(preflight.digest(raw_spec) == grant['approved_spec_sha256']
                      and service.file_hash(worker) == grant['worker_sha256'], 'grant_pinned_file_changed')
    preflight.require(preflight.digest(preflight.small(plan['paths']['approval'])) == grant['approval_sha256'],
                      'grant_approval_changed')
    policy = service.policy_from_spec(raw_spec)
    guard = service.ClosedSourceGuard(plan['paths']['source'], raw_spec)
    preflight.require(guard.identity['manifest_sha256'] == grant['source_manifest_sha256'],
                      'grant_source_identity_changed')
    guard.check(require_ready=True)
    current = int(clock()) if clock else int(time.time())
    ready = preflight.dt.datetime.fromisoformat(ready_by)
    preflight.require(ready_by == grant['ready_by'] and ready.tzinfo is not None
                      and current >= grant['checked_at_utc']
                      and current + policy['startup_timeout_seconds'] <= ready.timestamp(),
                      'grant_startup_window_invalid')
    grant_sha = preflight.digest(json.dumps(grant, sort_keys=True, separators=(',', ':')).encode())
    launcher = preflight.regular(Path(__file__).resolve())
    preflight.require(not preflight.os.access(launcher, preflight.os.W_OK), 'grant_launcher_writable_by_worker')
    if stop is not None and stop.at is not None:
        raise InterruptedError('operator stop before grant commit')
    manifest = dict(version='reaction_shadow_supervisor_v3', generation=str(root),
                    binary_sha256=grant['worker_sha256'], spec_sha256=grant['approved_spec_sha256'],
                    closed_source=guard.identity, admission_grant=grant,
                    admission_grant_sha256=grant_sha,
                    launcher_sha256=service.file_hash(launcher),
                    preflight_sha256=service.file_hash(Path(preflight.__file__).resolve()))
    # Attempt fsyncs its immutable manifest before supervise can spawn the child.
    attempt = service.Attempt(root, 'create', manifest)
    try:
        emit(json.dumps(dict(event='reaction_shadow_admission_grant_persisted',
                             grant_sha256=grant_sha, checked_at_utc=grant['checked_at_utc'],
                             order_emission_allowed=False)))
        child_env = {'REACTION_SHADOW_REDIS_URL':environment['REACTION_SHADOW_REDIS_URL'], 'TZ':'UTC'}
        return service.supervise([str(worker), 'create', str(spec), str(root), str(int(ready.timestamp()))],
                                 policy, ready.timestamp(), attempt, child_env,
                                 grant['approved_spec_sha256'], stop, emit, source_guard=guard,
                                 require_full_startup_window=True)
    finally:
        attempt.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--plan-sha256', required=True)
    parser.add_argument('--ready-by', required=True)
    args = parser.parse_args()
    stop = service.StopRequest()
    old = {sig:signal.signal(sig, stop.request) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        launch(args.plan, args.plan_sha256, args.ready_by, stop=stop,
               emit=lambda line:print(line, flush=True))
    finally:
        for sig, handler in old.items(): signal.signal(sig, handler)


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps(dict(event='reaction_shadow_launch_failed',
                              error_class=type(error).__name__,
                              reason=getattr(error, 'reason', 'admission_or_launch_error'),
                              error_sha256=hashlib.sha256(str(error).encode()).hexdigest(),
                              order_emission_allowed=False,
                              action='retain_and_audit_no_automatic_restart')), flush=True)
        sys.exit(1)
