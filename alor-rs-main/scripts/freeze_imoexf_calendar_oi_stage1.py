"""Read-only reference audit, or explicit first creation of a private E0 baseline."""
import argparse
import csv
import datetime as dt
import hashlib
import json
from pathlib import Path
import subprocess

PACKAGES = (
    'moex_calendar_context_2026_11_15',
    'moex_component_context_2026_11_15',
    'moex_oi_gate_overnight_2026_11_15',
)
PROTECTED = (
    'configs/examples/reaction-shadow.usdrubf-h4-w4.toml',
    'strategy-runtime/src/strategies/hybrid_intraday_runtime.rs',
    'strategy-runtime/src/strategies/alor_usdrubf_hybrid.rs',
    'strategy-runtime/src/strategies/ri_author41_42_live.rs',
)


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def scoped_file(workspace, path):
    resolved = path.resolve(strict=True)
    if workspace not in resolved.parents or not resolved.is_file() or path.is_symlink():
        raise ValueError('reference_outside_workspace_or_not_regular')
    return resolved


def pin(expected, relative, checksum):
    if len(checksum) != 64 or any(c not in '0123456789abcdef' for c in checksum):
        raise ValueError('invalid_reference_hash')
    if relative in expected and expected[relative] != checksum:
        raise ValueError('conflicting_frozen_references:' + relative)
    expected[relative] = checksum


def build(workspace, repo):
    expected, references, inventories = {}, set(), {}
    for name in PACKAGES:
        directory = workspace / 'analiz_alpha_si' / name
        for filename in ['PROTOCOL.md', 'protocol.json', 'frozen_inputs.json', 'artifact_manifest.csv']:
            references.add(scoped_file(workspace, directory / filename))
        frozen = json.loads((directory / 'frozen_inputs.json').read_text())
        for relative, checksum in frozen.items():
            path = scoped_file(workspace, directory / relative)
            pin(expected, str(path.relative_to(workspace)), checksum)
        with (directory / 'artifact_manifest.csv').open() as stream:
            entries = list(csv.DictReader(stream))
        for item in entries:
            path = scoped_file(workspace, directory / item['path'])
            if path.stat().st_size != int(item['bytes']) or digest(path) != item['sha256']:
                raise ValueError('artifact_inventory_drift:' + str(path.relative_to(workspace)))
        inventories[name] = len(entries)
    for relative, checksum in expected.items():
        path = workspace / relative
        if digest(path) != checksum:
            raise ValueError('frozen_input_drift:' + relative)
        references.add(path)
    references.update(workspace / 'analiz_alpha_si' / name / filename for name, filename in [
        (PACKAGES[0], 'exchange_calendar.py'), (PACKAGES[0], 'session_data.py'),
        (PACKAGES[1], 'component_models.py'), (PACKAGES[2], 'run_research.py'),
        (PACKAGES[2], 'capture_oi.py'), (PACKAGES[2], 'ENGINEERING_TZ.md'),
    ])
    protected = {}
    for relative in PROTECTED:
        path = repo / relative
        tracked = subprocess.check_output(['git', 'show', 'HEAD:alor-rs-main/' + relative], cwd=repo)
        if path.read_bytes() != tracked:
            raise ValueError('protected_existing_source_changed:' + relative)
        protected[relative] = digest(path)
    return dict(schema_version='imoexf_calendar_oi_stage1_baseline_v1',
        scope=dict(arms=['IMO_AA', 'IMO_AB', 'IMO_AB_OI'], quantity=1, max_sessions=20,
            contrasts=['IMO_AB-IMO_AA', 'IMO_AB_OI-IMO_AB'], broker_emission_allowed=False,
            deployed=False, full_model_parity_passed=False,
            existing_usdrubf=dict(profile='Previous-day07 H4/W4',
                policies=['weekday_only', 'weekend_context_no_trades'], changed=False),
            new_usdrubf_arms='DEFERRED_REQUIRES_SEPARATE_MATCHED_REFERENCE',
            mr_context='REGULAR', bo_control_context='REGULAR', bo_candidate_context='REGULAR_PLUS_ADMITTED_DSVD',
            oi_rule='prior_two_REGULAR_OI_ratio_minus_one_strictly_negative',
            costs='1_RUB_per_leg_plus_notional_0.000066_plus_one_tick_per_leg',
            no_overnight=True, mr_first_shared_slot=True),
        artifact_inventory_counts=inventories, frozen_dependency_count=len(expected),
        references={str(p.relative_to(workspace)):dict(sha256=digest(p), bytes=p.stat().st_size)
                    for p in sorted(references)}, protected_sources=protected)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--freeze', action='store_true')
    args = parser.parse_args()
    workspace = args.workspace.resolve(strict=True)
    repo = Path(__file__).resolve().parents[1]
    result = build(workspace, repo)
    if args.freeze:
        args.manifest.parent.mkdir(parents=True, exist_ok=True)
        value = dict(frozen_at_utc=dt.datetime.now(dt.timezone.utc).isoformat(), **result)
        with args.manifest.open('x') as stream:
            json.dump(value, stream, indent=2)
            stream.write('\n')
        action = 'BASELINE_CREATED_NOT_ADMITTED'
    else:
        value = json.loads(args.manifest.read_text())
        value.pop('frozen_at_utc')
        if value != result:
            raise ValueError('baseline_changed_requires_review_no_overwrite')
        action = 'BASELINE_VERIFIED_READ_ONLY'
    print(json.dumps(dict(status=action, references=len(result['references']),
                          dependencies=result['frozen_dependency_count'], protected_sources=len(PROTECTED))))


if __name__ == '__main__':
    main()
