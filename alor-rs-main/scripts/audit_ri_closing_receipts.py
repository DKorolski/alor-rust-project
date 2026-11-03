#!/usr/bin/env python3
"""Read-only audit of a captured committed RI prefix, not a model/recovery replay."""
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import stat

MSK = dt.timezone(dt.timedelta(hours=3))
MAX_BYTES = 320 * 1024 * 1024


def require(ok, message):
    if not ok:
        raise ValueError(message)


def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, "duplicate JSON key")
        result[key] = value
    return result


def invalid_constant(_):
    raise ValueError("non-finite JSON constant")


DECODER = json.JSONDecoder(object_pairs_hook=pairs, parse_constant=invalid_constant)


class Reader:
    def __init__(self):
        self.bytes = 0

    def checked(self, path, limit):
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, 'rb') as handle:
            info = os.fstat(handle.fileno())
            require(stat.S_ISREG(info.st_mode) and info.st_size <= limit, "invalid/big audit file")
            raw = handle.read(limit + 1)
        self.bytes += len(raw)
        require(len(raw) <= limit and self.bytes <= MAX_BYTES, "audit byte budget")
        text = raw.decode('utf-8')
        wrapper = DECODER.decode(text)
        require(set(wrapper) == {'payload', 'sha256'} and text.startswith('{"payload":'), "unsupported checked encoding")
        # Hash the original stored payload bytes, not Python's float rendering.
        start = len('{"payload":')
        payload, end = DECODER.raw_decode(text, start)
        sha = hashlib.sha256(text[start:end].encode()).hexdigest()
        require(sha == wrapper['sha256'], "checked payload checksum mismatch")
        return payload, sha


def local(epoch):
    return dt.datetime.fromtimestamp(epoch, MSK).isoformat()


def cutoff_after(day, excluded):
    day += dt.timedelta(days=1)
    while day.weekday() >= 5 or day.isoformat() in excluded:
        day += dt.timedelta(days=1)
    hour = 9 if day < dt.date(2026, 7, 14) else 7
    return int(dt.datetime.combine(day, dt.time(hour), MSK).timestamp())


def audit(root, dates):
    root = Path(root).absolute()
    require(root.resolve(strict=True) == root and root.is_dir(), "canonical generation directory required")
    require(1 <= len(dates) <= 30 and len(set(dates)) == len(dates), "explicit unique audit dates required")
    dates = [dt.date.fromisoformat(d) for d in dates]
    targets = {int(dt.datetime.combine(d, dt.time(23, 40), MSK).timestamp()): d for d in dates}
    for name in ('ingress', 'transactions'):
        require(not (root/name).is_symlink() and (root/name).is_dir(), "invalid generation subdirectory")
    reader = Reader()
    head, _ = reader.checked(root/'transactions/HEAD.json', 4096)
    header, previous = reader.checked(root/'transactions/manifest.json', 65536)
    require(header['version'] == 'ri_paired_shadow_runner_v1', "unsupported runner version")
    spec = header['spec']; cfg = spec['strategy']
    require(cfg.get('allow_order_emission') is False and cfg['session_start_time'] == '07:00:00', "not an approved RI shadow")
    require(type(head['index']) is int and 0 <= head['index'] <= 20000, "transaction count budget")
    manifest, ingress_hash = reader.checked(root/'ingress/manifest.json', 65536)
    identity = manifest['identity']
    require(manifest['version'] == 'ri_shadow_receipt_journal_v1' and identity == dict(
        generation=spec['generation']['id'], receipt_stream=spec['receipt_stream'],
        source_stream=spec['source_stream'], symbol=cfg['symbol']), "ingress identity mismatch")
    selected_steps = {}; ingested = observed = step_sequence = 0
    processed_at = manifest['created_at_utc']
    for index in range(1, head['index'] + 1):
        tx, sha = reader.checked(root/f'transactions/{index:08}.json', 16*1024*1024)
        require(tx['index'] == index and tx['previous_sha256'] == previous, "transaction chain mismatch")
        require(type(tx['processed_at_utc']) is int and tx['processed_at_utc'] >= processed_at, "transaction clock regression")
        processed_at = tx['processed_at_utc']
        out = tx['output']
        require(out['transaction'] == index and out['generation'] == identity['generation'], "outbox identity mismatch")
        require(ingested <= out['ingested_sequence'] <= 10000
                and observed <= out['observed_sequence'] <= out['ingested_sequence'], "non-monotone output cursor")
        ingested, observed = out['ingested_sequence'], out['observed_sequence']
        for step in out['steps']:
            require(step['sequence'] == step_sequence + 1 and len(step['arms']) == 2, "paired observation order mismatch")
            step_sequence = step['sequence']
            if any(a['bar_ts_utc'] in targets for a in step['arms']):
                selected_steps[step_sequence] = step['arms']
        previous = sha
    require(previous == head['sha256'] and step_sequence == observed, "HEAD/observation coverage mismatch")
    reports = {d.isoformat(): dict(label_local=local(label), cutoff_local=local(cutoff_after(d, cfg['excluded_model_dates'])),
        receipts=[]) for label, d in targets.items()}
    source_ids = set(); last_durable = manifest['created_at_utc']
    for sequence in range(1, ingested + 1):
        raw, raw_hash = reader.checked(root/f'ingress/{sequence:08}.raw.json', 256*1024)
        commit, commit_hash = reader.checked(root/f'ingress/{sequence:08}.commit.json', 256*1024)
        require(raw['sequence'] == commit['sequence'] == sequence and commit['raw_sha256'] == raw_hash
                and raw['previous_commit_sha256'] == ingress_hash, "ingress chain mismatch")
        require(last_durable <= raw['received_at_utc'] <= commit['durable_at_utc'], "receipt/durability clock regression")
        require(raw['source_id'] not in source_ids, "duplicate source identity")
        source_ids.add(raw['source_id']); last_durable = commit['durable_at_utc']; ingress_hash = commit_hash
        bar = DECODER.decode(raw['raw_bar_json']); label = bar['close_time_utc']
        if label not in targets:
            continue
        day = targets[label]; arms = selected_steps.get(sequence)
        if sequence <= observed:
            require(arms is not None, "closing observation missing from outbox")
        for arm in arms or []:
            require(arm['sequence'] == sequence and arm['bar_ts_utc'] == label and arm['origin'] == bar['origin']
                    and arm['received_at_utc'] == raw['received_at_utc']
                    and arm['durable_at_utc'] == commit['durable_at_utc'], "closing outbox/receipt binding mismatch")
        cutoff = cutoff_after(day, cfg['excluded_model_dates'])
        reports[day.isoformat()]['receipts'].append(dict(sequence=sequence, source_id=raw['source_id'],
            symbol=bar['symbol'], origin=bar['origin'], received_local=local(raw['received_at_utc']),
            durable_local=local(commit['durable_at_utc']), closed_on_receipt=raw['received_at_utc'] >= label + 600,
            durable_before_cutoff=commit['durable_at_utc'] < cutoff,
            arm_dispositions=[a['disposition'] for a in arms] if arms else None,
            diagnostic_record_counts=[len(a['records']) for a in arms] if arms else None))
    for report in reports.values():
        receipts = report['receipts']
        timely = [r for r in receipts if r['symbol'] == cfg['symbol'] and r['closed_on_receipt'] and r['durable_before_cutoff']]
        exercised = [r['sequence'] for r in timely if r['origin'] == 'history_gap' and r['arm_dispositions'] == [
            'context_only:Quarantined(RecoveryDisabled)', 'context_only:ContextOnly']]
        day = dt.date.fromisoformat(report['label_local'][:10])
        cutoff = cutoff_after(day, cfg['excluded_model_dates'])
        regular = day.weekday() < 5 and day.isoformat() not in cfg['excluded_model_dates']
        coverage = ('non_model_date' if not regular else 'closing_recovery_exercised' if exercised else
                    'timely_closing_without_recovery_contrast' if timely else
                    'closing_pending_before_cutoff' if processed_at < cutoff else 'no_timely_closing_receipt')
        report.update(receipt_count=len(receipts), timely_receipt_count=len(timely), recovery_exercised_sequences=exercised,
                      model_date_eligible=regular, cutoff_reached=processed_at >= cutoff, coverage=coverage)
    return dict(version='ri_closing_receipt_audit_v1', generation=identity['generation'], symbol=cfg['symbol'],
        source_stream=identity['source_stream'], snapshot_head=head, ingested_sequence=ingested,
        snapshot_processed_at_utc=processed_at,
        observed_sequence=observed, unobserved_ingested_receipts=ingested-observed, bytes_read=reader.bytes,
        checked_payload_hashes_verified=True, chain_binding_verified=True, days=reports,
        limitations=['Committed-prefix read audit, not independent semantic Rust/state replay.',
                     'Context readiness does not prove 23:40 coverage; no automatic clean-session credit.',
                     'This tool never repairs/restarts a generation or accesses Redis/broker.'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--generation', required=True, type=Path)
    parser.add_argument('--date', required=True, action='append')
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    output = args.output.absolute()
    require(output.parent.resolve(strict=True) == output.parent
            and not output.resolve().is_relative_to(args.generation.resolve()), "new output outside generation required")
    result = audit(args.generation, args.date)
    with output.open('x') as handle:
        json.dump(result, handle, ensure_ascii=False, indent=2); handle.write('\n')
    print(json.dumps(dict(report=str(output), snapshot_head=result['snapshot_head'],
        coverage={d:v['coverage'] for d,v in result['days'].items()})))


if __name__ == '__main__':
    main()
