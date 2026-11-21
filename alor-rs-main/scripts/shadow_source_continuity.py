"""Normal pre-open source reuse. Pure checks/read-only Linux identity, no grant."""
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import tomllib

VERSION = 'reaction_shadow_source_continuity_v1'
FIELDS = {'version', 'source_root', 'source_manifest_sha256', 'symbol', 'source_stream',
          'source_first_live_session', 'model_first_session', 'prepared_spec',
          'process', 'minimum_ws_generation'}


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def fingerprint(value):
    return digest(json.dumps(value, sort_keys=True, separators=(',', ':'),
                             ensure_ascii=False, allow_nan=False).encode())


def read_regular(path, limit=65536):
    path = Path(path)
    require(path.is_absolute() and path.resolve(strict=True) == path
            and path.is_file() and not path.is_symlink(), 'continuity_regular_canonical_file_required')
    with path.open('rb') as stream:
        data = stream.read(limit + 1)
    require(len(data) <= limit, 'continuity_file_too_large')
    return data


def is_sha(value):
    return (isinstance(value, str) and len(value) == 64
            and all(c in '0123456789abcdef' for c in value))


def inspect_process(pid):
    require(type(pid) is int and pid > 1 and os.uname().sysname == 'Linux',
            'continuity_linux_process_required')
    # Unlike /proc/PID/exe, stat/starttime is available to the distinct reader UID.
    raw = read_regular(f'/proc/{pid}/stat', 16384).decode()
    require(raw.split(' ', 1)[0] == str(pid) and ')' in raw, 'continuity_process_stat_invalid')
    fields = raw.rsplit(')', 1)[1].split()
    require(len(fields) >= 20 and fields[0] not in ('Z', 'X', 'x'), 'continuity_process_not_running')
    return dict(pid=pid, start_ticks=int(fields[19]),
                boot_id=read_regular('/proc/sys/kernel/random/boot_id', 128).decode().strip())


def check_process(binding):
    expected = binding['process']
    require(type(expected) is dict and set(expected) == {'pid', 'start_ticks', 'boot_id'}
            and type(expected['pid']) is int and expected['pid'] > 1
            and type(expected['start_ticks']) is int and expected['start_ticks'] > 0
            and isinstance(expected['boot_id'], str) and len(expected['boot_id']) == 36,
            'continuity_process_pin_invalid')
    try:
        actual = inspect_process(expected['pid'])
    except OSError as error:
        raise ValueError('continuity_process_unavailable') from error
    require(actual == expected, 'continuity_process_or_boot_changed')


def validate(binding, root, manifest, raw_spec):
    require(type(binding) is dict and set(binding) == FIELDS
            and binding['version'] == VERSION, 'continuity_schema_invalid')
    require(is_sha(binding['source_manifest_sha256'])
            and digest(manifest) == binding['source_manifest_sha256'], 'continuity_manifest_changed')
    root = Path(root)
    require(str(root) == binding['source_root'] and root.is_absolute()
            and root.resolve(strict=True) == root, 'continuity_source_root_changed')
    source = json.loads(manifest)
    runner = tomllib.loads(raw_spec.decode())['runner']
    require('late_start' not in runner and source['version'] == 'shadow_gateway_ws_closed_m10_v4',
            'continuity_requires_normal_strict_ws_v4')
    source_day = dt.date.fromisoformat(binding['source_first_live_session'])
    model_day = dt.date.fromisoformat(binding['model_first_session'])
    require(source_day.isoformat() == binding['source_first_live_session']
            and model_day.isoformat() == binding['model_first_session'] and source_day <= model_day
            and source['config']['first_live_session'] == source_day.isoformat()
            and runner['first_decision_session'] == model_day.isoformat(), 'continuity_activation_dates_invalid')
    require(binding['symbol'] in ('IMOEXF', 'USDRUBF')
            and binding['symbol'] == source['config']['symbol'] == runner['ingress']['symbol']
            and binding['source_stream'] == source['config']['stream'] == runner['ingress']['source_stream'],
            'continuity_symbol_stream_changed')
    prepared = binding['prepared_spec']
    require(type(prepared) is dict and set(prepared) == {'path', 'sha256'}
            and is_sha(prepared['sha256']), 'continuity_prepared_spec_pin_invalid')
    original = read_regular(prepared['path'])
    require(digest(original) == prepared['sha256'], 'continuity_prepared_spec_changed')
    expected = tomllib.loads(original.decode())
    actual = tomllib.loads(raw_spec.decode())
    require('bootstrap' not in expected['runner'] and 'late_start' not in expected['runner'],
            'continuity_prepared_spec_must_be_clean')
    actual['runner'].pop('bootstrap', None)
    require(actual == expected, 'continuity_model_configuration_changed')
    require(type(binding['minimum_ws_generation']) is int and binding['minimum_ws_generation'] > 0,
            'continuity_ws_generation_pin_invalid')
    check_process(binding)
    return json.loads(json.dumps(binding, sort_keys=True))
