#!/usr/bin/env python3
"""Opt-in granted create; v4 also permits explicit clean pre-open resume."""
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
    v4 = plan.get('version') == 'reaction_shadow_preflight_v4'
    preflight.require(plan.get('version') in ('reaction_shadow_preflight_v3','reaction_shadow_preflight_v4'),
                      'grant_launcher_requires_v3_or_v4')
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
    guard = service.ClosedSourceGuard(plan['paths']['source'], raw_spec,
                                      source_continuity=plan.get('source_continuity'))
    preflight.require(guard.identity['manifest_sha256'] == grant['source_manifest_sha256'],
                      'grant_source_identity_changed')
    guard.check(require_ready=True)
    if v4:
        preflight.require(grant['version'] == 'reaction_shadow_admission_grant_v2'
                          and grant['source_continuity'] == guard.source_continuity == plan['source_continuity']
                          and grant['source_continuity_sha256'] == guard.identity['source_continuity_sha256']
                          and service.file_hash(plan['files']['continuity_helper']['path']) == grant['continuity_helper_sha256'],
                          'grant_source_continuity_changed')
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
    manifest = dict(version='reaction_shadow_supervisor_v4' if v4 else 'reaction_shadow_supervisor_v3', generation=str(root),
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


def resume(plan_path, plan_sha, ready_by, *, environ=None, clock=None, stop=None, emit=print):
    """Resume the SAME admitted v4 generation, never renew/backdate its grant."""
    environment = dict(preflight.os.environ if environ is None else environ)
    preflight.require(preflight.os.geteuid() != 0 and not preflight.FORBIDDEN_ENV.intersection(environment),
                      'dedicated_nonroot_reader_and_clean_environment_required')
    raw_plan = preflight.small(plan_path)
    preflight.require(preflight.digest(raw_plan) == plan_sha, 'preflight_plan_changed')
    plan = json.loads(raw_plan)
    preflight.require(plan.get('version') == 'reaction_shadow_preflight_v4', 'continuity_resume_requires_v4')
    preflight.fields(plan, ('version','files','paths','redis','source_continuity'))
    preflight.fields(plan['files'], ('worker','supervisor','prepared_spec','source_manifest','continuity_helper'))
    preflight.fields(plan['paths'], ('source','owner','generation','approved_spec','approval'))
    preflight.fields(plan['redis'], ('host','port','username'))
    for item in plan['files'].values():
        preflight.fields(item, ('path','sha256'))
        path = preflight.regular(item['path'])
        preflight.require(service.file_hash(path) == item['sha256'] and not preflight.os.access(path, preflight.os.W_OK),
                          'resume_pinned_file_changed_or_writable')
    preflight.require(Path(plan['files']['supervisor']['path']) == Path(service.__file__).resolve()
                      and Path(plan['files']['continuity_helper']['path']) == Path(preflight.continuity_module().__file__).resolve()
                      and plan['source_continuity']['prepared_spec'] == plan['files']['prepared_spec']
                      and plan['source_continuity']['source_manifest_sha256'] == plan['files']['source_manifest']['sha256'],
                      'resume_continuity_import_or_plan_changed')
    root, owner, source = [Path(plan['paths'][key]) for key in ('generation','owner','source')]
    preflight.require(all(p.is_absolute() and p.resolve(strict=True)==p and p.is_dir() for p in (root,owner,source))
                      and root.parent==owner and source.stat().st_uid != preflight.os.geteuid()
                      and not preflight.os.access(source,preflight.os.W_OK) and preflight.os.access(owner,preflight.os.W_OK),
                      'resume_generation_or_source_permissions_invalid')
    recorded = json.loads(preflight.small(root.with_name(root.name+'.supervisor')/'manifest.json'))
    preflight.fields(recorded, ('version','generation','binary_sha256','spec_sha256','closed_source','admission_grant',
                               'admission_grant_sha256','launcher_sha256','preflight_sha256'))
    preflight.require(recorded['version']=='reaction_shadow_supervisor_v4' and recorded['generation']==str(root),
                      'resume_manifest_identity_mismatch')
    grant = recorded['admission_grant']
    preflight.require(grant['version']=='reaction_shadow_admission_grant_v2'
                      and grant['policy']=='normal_source_reuse_reviewed_snapshot_grant_v1'
                      and grant['plan_sha256']==plan_sha and grant['order_emission_allowed'] is False
                      and grant['reader_uid']==preflight.os.geteuid()
                      and recorded['admission_grant_sha256']==preflight.digest(json.dumps(grant,sort_keys=True,separators=(',',':')).encode()),
                      'resume_grant_changed')
    spec = preflight.regular(plan['paths']['approved_spec'])
    raw_spec = preflight.small(spec)
    worker = preflight.regular(plan['files']['worker']['path'])
    preflight.require(preflight.digest(raw_spec)==recorded['spec_sha256']==grant['approved_spec_sha256']
                      and service.file_hash(worker)==recorded['binary_sha256']==grant['worker_sha256']
                      and service.file_hash(Path(__file__).resolve())==recorded['launcher_sha256']
                      and service.file_hash(Path(preflight.__file__).resolve())==recorded['preflight_sha256']
                      and service.file_hash(Path(service.__file__).resolve())==grant['supervisor_sha256']
                      and service.file_hash(Path(preflight.continuity_module().__file__).resolve())==grant['continuity_helper_sha256']
                      and preflight.digest(preflight.small(plan['paths']['approval']))==grant['approval_sha256'],
                      'resume_code_spec_or_approval_changed')
    preflight.require(not preflight.os.access(spec,preflight.os.W_OK)
                      and not preflight.os.access(Path(__file__).resolve(),preflight.os.W_OK), 'resume_inputs_writable')
    guard = service.ClosedSourceGuard(source,raw_spec,source_continuity=plan['source_continuity'])
    preflight.require(guard.identity==recorded['closed_source']
                      and grant['source_continuity']==guard.source_continuity
                      and grant['source_continuity_sha256']==guard.identity['source_continuity_sha256']
                      and grant['source_manifest_sha256']==guard.identity['manifest_sha256'], 'resume_source_binding_changed')
    manifest = json.loads(preflight.small(Path(plan['files']['source_manifest']['path'])))
    preflight.require(Path(plan['files']['source_manifest']['path'])==source/'manifest.json', 'resume_manifest_path_changed')
    reader = preflight.Reader(environment.get('REACTION_SHADOW_REDIS_URL',''),plan['redis'],manifest['config']['stream'])
    try:
        preflight.require(reader.command('PING')==b'PONG','reader_ping_failed')
        preflight.verify_reader_prefix(reader,preflight.source_records(source))
    finally:
        reader.close()
    guard.check(require_ready=True)
    guard.check_full_session_window()
    policy = service.policy_from_spec(raw_spec)
    current = int(clock()) if clock else int(time.time())
    ready = preflight.dt.datetime.fromisoformat(ready_by)
    morning = preflight.dt.datetime.fromtimestamp(current,preflight.MSK)
    cutoff = preflight.dt.datetime.combine(morning.date(),preflight.dt.time(7),preflight.MSK)
    preflight.require(current >= grant['checked_at_utc'] and ready.tzinfo is not None and ready.microsecond==0
                      and current+policy['startup_timeout_seconds'] <= ready.timestamp() <= cutoff.timestamp(),
                      'resume_requires_full_preopen_window')
    if stop is not None and stop.at is not None:
        raise InterruptedError('operator stop before resume')
    attempt = service.Attempt(root,'resume',recorded)
    try:
        child_env = {'REACTION_SHADOW_REDIS_URL':environment['REACTION_SHADOW_REDIS_URL'],'TZ':'UTC'}
        return service.supervise([str(worker),'resume',str(spec),str(root),str(int(ready.timestamp()))],
            policy,ready.timestamp(),attempt,child_env,grant['approved_spec_sha256'],stop,emit,
            source_guard=guard,require_full_startup_window=True)
    finally:
        attempt.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--plan-sha256', required=True)
    parser.add_argument('--ready-by', required=True)
    parser.add_argument('--resume', action='store_true', help='same v4 generation after a clean operator stop; pre-open only')
    args = parser.parse_args()
    stop = service.StopRequest()
    old = {sig:signal.signal(sig, stop.request) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        action = resume if args.resume else launch
        action(args.plan, args.plan_sha256, args.ready_by, stop=stop,
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
