#!/usr/bin/env python3
"""Pinned, dated collect-only OI job. No model, Redis, credentials or orders."""
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import uuid

MSK = dt.timezone(dt.timedelta(hours=3))
MODEL = 'a469365d54844879ed63108bc99d45be18ace95d261627b57a94ceb1a4c18135'
MAX_CONFIG = 262144
MAX_OUTPUT = 6 * 1024 * 1024


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON key')
        result[key] = value
    return result


def parse(raw):
    return json.loads(raw, object_pairs_hook=unique_object)


def absolute(value):
    path = Path(value)
    if not path.is_absolute() or path.resolve(strict=True) != path:
        raise ValueError('absolute canonical existing path required')
    return path


def read_file(value, expected=None):
    path = absolute(value)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o022:
            raise ValueError('regular non-group-writable release file required')
        raw = stream.read(MAX_CONFIG + 1)
    if len(raw) > MAX_CONFIG or (expected is not None and sha(raw) != expected):
        raise ValueError('file bound/hash mismatch')
    return raw


def pin(value, *, binary=False):
    if set(value) != {'path', 'sha256'} or not re.fullmatch('[0-9a-f]{64}', value['sha256']):
        raise ValueError('invalid file pin')
    if binary:
        path = absolute(value['path'])
        if not path.is_file() or path.stat().st_mode & 0o022 or not os.access(path, os.X_OK):
            raise ValueError('unsafe binary')
        with path.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        if digest != value['sha256']:
            raise ValueError('binary hash mismatch')
        return path
    read_file(value['path'], value['sha256'])
    return absolute(value['path'])


def utc_now():
    return dt.datetime.now(dt.timezone.utc)


def load_release(path, expected):
    if not re.fullmatch('[0-9a-f]{64}', expected):
        raise ValueError('explicit release SHA256 required')
    spec = parse(read_file(path, expected))
    keys = {'version', 'binary', 'calendar', 'configs', 'probe_config', 'store_root', 'report_root'}
    if set(spec) != keys or spec['version'] != 'imoexf_oi_collect_only_schedule_v1':
        raise ValueError('unsupported collect-only schedule')
    pin(spec['binary'], binary=True)
    calendar = parse(read_file(pin(spec['calendar'])))
    if calendar.get('symbol') != 'IMOEXF' or calendar.get('schema_version') != 'moex_shadow_calendar_v1':
        raise ValueError('unexpected calendar')
    store, reports = absolute(spec['store_root']), absolute(spec['report_root'])
    if not store.is_dir() or not reports.is_dir() or store == reports:
        raise ValueError('separate existing store/report directories required')
    dates = spec['configs']
    if not isinstance(dates, dict) or not 1 <= len(dates) <= 20:
        raise ValueError('bounded reviewed dates required')
    for day, config_pin in [*dates.items(), (None, spec['probe_config'])]:
        config = parse(read_file(pin(config_pin)))
        if (config.get('model_sha256') != MODEL or config.get('store_root') != str(store)
                or config.get('calendar_path') != spec['calendar']['path']
                or config.get('calendar_pin', {}).get('artifact_sha256') != spec['calendar']['sha256']):
            raise ValueError('config identity mismatch')
        chosen = dt.date.fromisoformat(config['decision_date'])
        if chosen.isoformat() != config['decision_date'] or (day is not None and day != chosen.isoformat()):
            raise ValueError('dated config mismatch')
    return spec


def native(spec, mode, config, timeout):
    result = subprocess.run([spec['binary']['path'], mode, str(config)],
                            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=timeout,
                            env={'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}, check=False)
    if len(result.stdout) > MAX_OUTPUT or len(result.stderr) > MAX_CONFIG:
        raise ValueError('native output bound')
    data = parse(result.stdout) if result.stdout.strip() else None
    return result.returncode, data, result.stderr.decode(errors='replace')[:2048]


def report(spec, payload):
    root = absolute(spec['report_root'])
    existing = list(root.iterdir())
    if len(existing) >= 256 or any(p.is_symlink() or not p.is_file() for p in existing):
        raise ValueError('report-store entry bound/type')
    if sum(p.stat().st_size for p in existing) + MAX_OUTPUT > 32 * 1024 * 1024:
        raise ValueError('report-store size bound')
    raw = (json.dumps(payload, sort_keys=True) + '\n').encode()
    if len(raw) > MAX_OUTPUT:
        raise ValueError('report size bound')
    path = root / f'{utc_now().astimezone(MSK).date()}-{uuid.uuid4()}.json'
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o400)
    with os.fdopen(fd, 'wb') as stream:
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)
    return path


def in_window(now):
    local = now.astimezone(MSK)
    start = local.replace(hour=6, minute=45, second=0, microsecond=0)
    end = local.replace(hour=6, minute=55, second=0, microsecond=0)
    return start <= local and local + dt.timedelta(seconds=10) <= end


def execute(manifest, expected, mode):
    if mode not in ('run', 'probe'):
        raise ValueError('unsupported job mode')
    spec = load_release(manifest, expected)
    now = utc_now().astimezone(MSK)
    if mode == 'run':
        config = spec['configs'].get(now.date().isoformat())
        if config is None:
            return {'event': 'oi_capture_not_scheduled', 'date_msk': now.date().isoformat(),
                    'models_connected': False, 'order_emission_allowed': False}, 0
        if not in_window(now):
            raise ValueError('outside approved morning window/commit reserve; no request made')
    else:
        config = spec['probe_config']
    path = pin(config)
    code, plan, error = native(spec, 'plan', path, 10)
    config_data = parse(read_file(path, config['sha256']))
    if (code or not isinstance(plan, dict) or plan.get('decision_date') != config_data['decision_date']
            or plan.get('model_sha256') != MODEL or plan.get('profile_id') != 'IMO_AB_OI'):
        raise ValueError('native dated plan failed: ' + error)
    code, clock, error = native(spec, 'clock', path, 10)
    if code or not isinstance(clock, dict) or clock.get('synchronized') is not True:
        raise ValueError('native clock qualification failed: ' + error)
    if mode == 'run' and not in_window(utc_now()):
        raise ValueError('morning window exhausted before collection')
    command = 'collect' if mode == 'run' else 'probe'
    code, result, error = native(spec, command, path, 600 if mode == 'run' else 40)
    okay = code == 0 and isinstance(result, dict) and result.get('plan') == plan
    if mode == 'probe':
        okay = (okay and result.get('version') == 'imoexf_oi_non_pit_probe_v1'
                and result.get('diagnostic_only') is True and result.get('qualified') is False
                and result.get('allow_new_bo') is False and result.get('passed') is True)
    else:
        okay = (okay and result.get('schema_version') == 'imoexf_oi_durable_qualification_v1'
                and result.get('status') == 'accepted_snapshot'
                and result.get('evidence_kind') == 'linux_kernel_clock')
    payload = dict(version='imoexf_oi_job_report_v1', mode=mode, plan=plan,
                   captured_at_utc=utc_now().isoformat(), native_returncode=code,
                   models_connected=False, order_emission_allowed=False,
                   okay=okay, result=result, stderr=error)
    saved = report(spec, payload)
    values = result if isinstance(result, dict) else {}
    return dict(event='oi_capture_finished', mode=mode, date_msk=plan['decision_date'],
                okay=okay, report=str(saved), status=values.get('status'),
                data_gate_allow_only=values.get('allow_new_bo'),
                models_connected=False, order_emission_allowed=False), 0 if okay else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', required=True)
    parser.add_argument('--manifest-sha256', required=True)
    parser.add_argument('--mode', choices=('run', 'probe'), default='run')
    args = parser.parse_args()
    try:
        output, code = execute(args.manifest, args.manifest_sha256, args.mode)
    except Exception as error:
        output, code = dict(event='oi_capture_job_failed', reason=str(error),
                            models_connected=False, order_emission_allowed=False), 1
    print(json.dumps(output, sort_keys=True))
    return code


if __name__ == '__main__':
    sys.exit(main())
