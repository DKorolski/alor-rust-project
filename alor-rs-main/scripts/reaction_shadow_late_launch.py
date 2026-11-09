#!/usr/bin/env python3
"""Explicit diagnostic late create. Never backdates receipts or grants clean-day credit."""
import argparse
import datetime as dt
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import tomllib

import reaction_shadow_preflight as p
import reaction_shadow_service as service

GATES = {'complete_context', 'canonical_recovery_prefix', 'as_observed_audit',
         'diagnostic_day_excluded', 'no_order_emission', 'source_binding'}


def check(plan_path, plan_sha, environment, clock=None):
    now = int(clock()) if clock else int(time.time())
    p.require(os.geteuid() != 0, 'dedicated_nonroot_reader_required')
    p.require(not p.FORBIDDEN_ENV.intersection(environment), 'broker_or_generic_redis_environment_forbidden')
    raw_plan = p.small(plan_path)
    p.require(p.sha(plan_sha) and p.digest(raw_plan) == plan_sha, 'preflight_plan_changed')
    plan = json.loads(raw_plan)
    p.fields(plan, ('version', 'files', 'paths', 'redis'))
    p.require(plan['version'] == 'reaction_shadow_late_preflight_v1', 'explicit_diagnostic_plan_required')
    p.fields(plan['files'], ('worker', 'supervisor', 'spec', 'source_manifest'))
    files = {}
    for name, item in plan['files'].items():
        p.fields(item, ('path', 'sha256'))
        path = p.regular(item['path'])
        p.require(p.sha(item['sha256']) and service.file_hash(path) == item['sha256'], 'pinned_file_changed')
        p.require(not os.access(path, os.W_OK), 'pinned_file_writable_by_worker')
        files[name] = path
    p.require(files['supervisor'] == Path(service.__file__).resolve(), 'supervisor_import_mismatch')
    p.require(os.access(files['worker'], os.X_OK), 'worker_not_executable')
    p.fields(plan['paths'], ('source', 'owner', 'generation', 'approval'))
    paths = {key:Path(value) for key, value in plan['paths'].items()}
    for name in ('source', 'owner'):
        path = paths[name]
        p.require(path.is_absolute() and path.resolve(strict=True) == path and path.is_dir(), 'canonical_directory_required')
    p.require(files['source_manifest'] == paths['source']/'manifest.json', 'source_manifest_path_mismatch')
    p.require(paths['source'].stat().st_uid != os.geteuid(), 'source_and_worker_uid_must_differ')
    p.require(not os.access(paths['source'], os.W_OK) and os.access(paths['owner'], os.W_OK), 'worker_source_directory_permissions')
    root = paths['generation']
    p.require(root.is_absolute() and root.parent == paths['owner'] and root.name not in ('', '.', '..'), 'generation_parent_mismatch')
    p.require(not root.exists() and not root.is_symlink()
              and not root.with_name(root.name+'.supervisor').exists()
              and not root.with_name(root.name+'.supervisor').is_symlink(), 'generation_or_attempt_already_exists')
    cfg_dir = p.regular(plan_path).parent
    p.require(files['spec'].parent == cfg_dir and paths['approval'].parent == cfg_dir, 'admission_path_mismatch')
    raw_spec = p.small(files['spec'])
    spec = tomllib.loads(raw_spec.decode())
    policy = service.policy_from_spec(raw_spec)
    runner = spec['runner']
    p.fields(runner, ('ingress', 'config', 'first_decision_session', 'excluded_dates', 'bootstrap', 'late_start'))
    late = runner['late_start']
    p.fields(late, ('version', 'source_first_live_session', 'first_clean_session', 'recovery'))
    p.require(late['version'] == 'diagnostic_late_start_v1', 'explicit_diagnostic_mode_required')
    first = dt.date.fromisoformat(runner['first_decision_session'])
    local_now = dt.datetime.fromtimestamp(now, p.MSK)
    p.require(first == local_now.date() and first.weekday() < 5 and first.isoformat() not in runner['excluded_dates']
              and dt.time(7) <= local_now.time() < dt.time(23, 30), 'diagnostic_same_day_window_required')
    next_day = first+dt.timedelta(days=1)
    while next_day.weekday() >= 5 or next_day.isoformat() in runner['excluded_dates']:
        next_day += dt.timedelta(days=1)
    p.require(late['first_clean_session'] == next_day.isoformat(), 'diagnostic_day_cannot_be_clean')
    manifest = json.loads(p.small(files['source_manifest']))
    p.require(manifest['version'] == 'shadow_gateway_ws_closed_m10_v4', 'diagnostic_requires_v4_source')
    source_cfg = manifest['config']
    p.require(source_cfg['terminal']['grace_seconds'] == 10, 'diagnostic_terminal_grace_requires_review')
    symbol = runner['ingress']['symbol']
    p.require((symbol, runner['config']['engine']) in (('IMOEXF', 'imoexf'), ('USDRUBF', 'usdrubf')), 'symbol_profile_required')
    guard = service.ClosedSourceGuard(paths['source'], raw_spec)
    revisions = p.revision_snapshot(paths['source'], symbol, now)
    quarantine = p.late_context_snapshot(paths['source'], source_cfg, now)
    approval_raw = p.small(paths['approval'])
    approval = json.loads(approval_raw)
    p.require(not os.access(paths['approval'], os.W_OK), 'admission_files_writable_by_worker')
    p.fields(approval, ('version', 'plan_sha256', 'spec_sha256', 'reviewed_at_utc', 'ready_by',
                        'gates', 'revision_review', 'late_context_review', 'evidence'))
    p.require(approval['version'] == 'reaction_shadow_late_admission_v1'
              and approval['plan_sha256'] == plan_sha and approval['spec_sha256'] == p.digest(raw_spec), 'admission_identity_mismatch')
    p.require(type(approval['reviewed_at_utc']) is int and 0 <= now-approval['reviewed_at_utc'] <= 300, 'fresh_diagnostic_review_required')
    p.fields(approval['gates'], GATES)
    p.require(all(value is True for value in approval['gates'].values()), 'operator_gates_not_approved')
    p.require(approval['revision_review'] == revisions and approval['late_context_review'] == quarantine, 'reviewed_audit_changed')
    evidence = approval['evidence']
    p.require(type(evidence) is list and len(evidence) == 2, 'reviewed_evidence_required')
    kinds = set()
    for item in evidence:
        p.fields(item, ('kind', 'path', 'sha256'))
        path = p.regular(item['path'])
        p.require(path.parent == cfg_dir/'evidence' and not os.access(path, os.W_OK)
                  and p.sha(item['sha256']) and p.digest(p.small(path)) == item['sha256'], 'reviewed_evidence_changed')
        kinds.add(item['kind'])
    p.require(kinds == {'source_terminal', 'overnight_recovery'}, 'reviewed_evidence_kinds')
    ready = dt.datetime.fromisoformat(approval['ready_by'])
    end = dt.datetime.combine(first, dt.time(23, 40), p.MSK).timestamp()
    p.require(ready.tzinfo is not None and not ready.microsecond and approval['ready_by'] == environment.get('SHADOW_READY_BY')
              and now+policy['startup_timeout_seconds'] <= ready.timestamp() <= end, 'readiness_window_invalid')
    recovery = late['recovery']
    p.fields(recovery, ('last_source_id', 'rows', 'source_sha256', 'last_label_utc'))
    p.require(type(recovery['rows']) is int and runner['bootstrap']['rows'] < recovery['rows'] <= 1024
              and p.sha(recovery['source_sha256']) and type(recovery['last_label_utc']) is int, 'bounded_recovery_required')
    # A closed prefix remains current until the NEXT expected close is overdue.
    p.require(0 <= now-recovery['last_label_utc']-600 <= 600+policy.get(service.ORDINARY_LIMIT, policy['max_model_lateness_seconds']),
              'diagnostic_recovery_tail_stale')
    binding = plan['redis']
    p.fields(binding, ('host', 'port', 'username'))
    address = ipaddress.ip_address(binding['host'])
    p.require(address.version == 4 and address.is_private and type(binding['port']) is int and 1 <= binding['port'] <= 65535
              and binding['username'].startswith('shadow_ws_reader_'), 'invalid_reader_binding')
    with_reader = p.Reader(environment.get('REACTION_SHADOW_REDIS_URL', ''), binding, source_cfg['stream'])
    try:
        p.require(with_reader.command('PING') == b'PONG', 'reader_ping_failed')
        sid, envelope = with_reader.envelope(recovery['last_source_id'], recovery['last_source_id'])
        p.require(sid == recovery['last_source_id'] and envelope['source'] == manifest['version']
                  and envelope['payload']['symbol'] == symbol and envelope['payload']['close_time_utc'] == recovery['last_label_utc'],
                  'recovery_redis_boundary_mismatch')
    finally:
        with_reader.close()
    inspected = subprocess.run([str(files['worker']), 'inspect-late-start', str(files['spec']),
                                runner['bootstrap']['first_source_id'], recovery['last_source_id']],
                               env={'REACTION_SHADOW_REDIS_URL':environment['REACTION_SHADOW_REDIS_URL'], 'TZ':'UTC'},
                               capture_output=True, timeout=20)
    p.require(inspected.returncode == 0 and len(inspected.stdout) <= 65536, 'diagnostic_inspection_failed')
    preview = json.loads(inspected.stdout)
    p.require(preview['event'] == 'late_start_inspected' and preview['order_emission_allowed'] is False
              and preview['preview']['order_emission_allowed'] is False
              and preview['preview']['bootstrap'] == runner['bootstrap'] and preview['preview']['recovery'] == recovery
              and preview['preview']['first_clean_session'] == late['first_clean_session']
              and len(preview['preview']['anchors']) == 2 and all(len(a) >= 2 for a in preview['preview']['anchors']),
              'diagnostic_inspection_mismatch')
    finished = int(clock()) if clock else int(time.time())
    p.require(finished >= now and finished+policy['startup_timeout_seconds'] <= ready.timestamp(), 'grant_missed_startup_window')
    guard.check(require_ready=True)
    p.require(p.revision_snapshot(paths['source'], symbol, finished) == revisions
              and p.late_context_snapshot(paths['source'], source_cfg, finished) == quarantine, 'audit_changed_during_inspection')
    p.require(p.digest(p.small(paths['approval'])) == p.digest(approval_raw)
              and p.digest(p.small(files['spec'])) == p.digest(raw_spec), 'pinned_approval_or_spec_changed')
    grant = dict(version='reaction_shadow_late_grant_v1', policy='diagnostic_as_observed_reviewed_snapshot_v1',
                 checked_at_utc=finished, plan_sha256=plan_sha, spec_sha256=p.digest(raw_spec),
                 approval_sha256=p.digest(approval_raw), worker_sha256=plan['files']['worker']['sha256'],
                 supervisor_sha256=plan['files']['supervisor']['sha256'], source=guard.identity,
                 revision_review=revisions, late_context_review=quarantine, bootstrap=runner['bootstrap'],
                 late_start=late, diagnostic_session=first.isoformat(), first_clean_session=next_day.isoformat(),
                 ready_by=approval['ready_by'], reader_uid=os.geteuid(), order_emission_allowed=False)
    return plan, grant, policy, guard


def launch(plan_path, plan_sha, *, environ=None, clock=None, stop=None, emit=print, preflight_only=False):
    environment = dict(os.environ if environ is None else environ)
    plan, grant, policy, guard = check(plan_path, plan_sha, environment, clock)
    if preflight_only:
        return dict(event='reaction_shadow_late_preflight', grant=grant, worker_started=False, order_emission_allowed=False)
    p.require(not os.access(Path(__file__).resolve(), os.W_OK), 'grant_launcher_writable_by_worker')
    now = int(clock()) if clock else int(time.time())
    ready = dt.datetime.fromisoformat(grant['ready_by']).timestamp()
    p.require(grant['checked_at_utc'] <= now and now+policy['startup_timeout_seconds'] <= ready, 'grant_startup_window_invalid')
    for name, item in plan['files'].items():
        p.require(service.file_hash(p.regular(item['path'])) == item['sha256'], 'pinned_file_changed')
    p.require(p.digest(p.small(plan['paths']['approval'])) == grant['approval_sha256'], 'grant_approval_changed')
    guard.check(require_ready=True)
    if stop is not None and stop.at is not None:
        raise InterruptedError('operator stop before grant commit')
    manifest = dict(version='reaction_shadow_supervisor_v4_late_start', generation=plan['paths']['generation'],
                    binary_sha256=grant['worker_sha256'], spec_sha256=grant['spec_sha256'], closed_source=guard.identity,
                    admission_grant=grant, admission_grant_sha256=p.digest(json.dumps(grant, sort_keys=True, separators=(',', ':')).encode()),
                    launcher_sha256=service.file_hash(Path(__file__).resolve()), preflight_sha256=service.file_hash(Path(p.__file__).resolve()))
    attempt = service.Attempt(Path(plan['paths']['generation']), 'create', manifest)
    try:
        emit(json.dumps(dict(event='reaction_shadow_late_grant_persisted', diagnostic_session=grant['diagnostic_session'],
                             first_clean_session=grant['first_clean_session'], order_emission_allowed=False)))
        return service.supervise([plan['files']['worker']['path'], 'create', plan['files']['spec']['path'],
                                  plan['paths']['generation'], str(int(ready))], policy, ready, attempt,
                                 {'REACTION_SHADOW_REDIS_URL':environment['REACTION_SHADOW_REDIS_URL'], 'TZ':'UTC'},
                                 grant['spec_sha256'], stop, emit, source_guard=guard, require_full_startup_window=True)
    finally:
        attempt.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--plan-sha256', required=True)
    parser.add_argument('--preflight-only', action='store_true')
    args = parser.parse_args()
    stop = service.StopRequest()
    old = {sig:signal.signal(sig, stop.request) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        result = launch(args.plan, args.plan_sha256, stop=stop, preflight_only=args.preflight_only,
                        emit=lambda row:print(row, flush=True))
        if args.preflight_only:
            print(json.dumps(result), flush=True)
    finally:
        for sig, handler in old.items(): signal.signal(sig, handler)


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps(dict(event='reaction_shadow_late_launch_failed', error_class=type(error).__name__,
                              error_sha256=hashlib.sha256(str(error).encode()).hexdigest(),
                              order_emission_allowed=False, action='retain_and_audit_no_automatic_restart')), flush=True)
        sys.exit(1)
