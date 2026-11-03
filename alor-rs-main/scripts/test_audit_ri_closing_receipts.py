import datetime as dt
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import audit_ri_closing_receipts as audit


def epoch(text):
    return int(dt.datetime.fromisoformat(text).replace(tzinfo=audit.MSK).timestamp())


def checked(path, payload):
    raw = json.dumps(payload, ensure_ascii=False, separators=(',', ':'), allow_nan=False)
    sha = hashlib.sha256(raw.encode()).hexdigest()
    path.write_text('{"payload":'+raw+',"sha256":"'+sha+'"}')
    return sha


class ClosingAuditTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()/'generation'
        self.root.mkdir(); (self.root/'ingress').mkdir(); (self.root/'transactions').mkdir()

    def fixture(self, late=False, contrast=True, observed=True):
        identity = dict(generation='owned-ri', receipt_stream='owned.receipts', source_stream='owned.bars', symbol='RIZ6')
        spec = dict(generation=dict(id=identity['generation'], first_decision_session='2026-10-13'),
                    receipt_stream=identity['receipt_stream'], source_stream=identity['source_stream'],
                    strategy=dict(symbol='RIZ6', allow_order_emission=False, session_start_time='07:00:00', excluded_model_dates=[]))
        previous = checked(self.root/'transactions/manifest.json', dict(version='ri_paired_shadow_runner_v1', spec=spec))
        ingress = checked(self.root/'ingress/manifest.json', dict(version='ri_shadow_receipt_journal_v1',
                          identity=identity, created_at_utc=epoch('2026-10-12 07:00')))
        steps = []
        for seq, label in enumerate(['2026-10-12 23:30', '2026-10-12 23:40'], 1):
            received = epoch('2026-10-12 23:40') if seq == 1 else epoch('2026-10-13 07:00' if late else '2026-10-13 03:00')
            origin = 'history_gap' if seq == 2 and contrast else 'live'
            bar = dict(symbol='RIZ6', close_time_utc=epoch(label), o=90000., h=90100., l=89900., c=90050., v=1., origin=origin)
            raw = dict(sequence=seq, source_id=f'{seq}-0', raw_bar_json=json.dumps(bar),
                       received_at_utc=received, previous_commit_sha256=ingress)
            raw_sha = checked(self.root/f'ingress/{seq:08}.raw.json', raw)
            ingress = checked(self.root/f'ingress/{seq:08}.commit.json', dict(sequence=seq, raw_sha256=raw_sha, durable_at_utc=received))
            dispositions = (['context_only:Quarantined(AtOrAfterCutoff)']*2 if late and seq == 2 else
                            ['context_only:Quarantined(RecoveryDisabled)', 'context_only:ContextOnly'] if origin == 'history_gap' else
                            ['context_only:ContextOnly']*2)
            arms = [dict(sequence=seq, bar_ts_utc=epoch(label), origin=origin, received_at_utc=received,
                         durable_at_utc=received, disposition=d, records=[]) for d in dispositions]
            steps.append(dict(sequence=seq, arms=arms))
        out = dict(generation=identity['generation'], transaction=1, ingested_sequence=2,
                   observed_sequence=2 if observed else 0, steps=steps if observed else [])
        tx = dict(index=1, previous_sha256=previous, processed_at_utc=received, output=out)
        sha = checked(self.root/'transactions/00000001.json', tx)
        checked(self.root/'transactions/HEAD.json', dict(index=1, sha256=sha))

    def test_actual_contrast_and_snapshot_is_read_only(self):
        self.fixture()
        before = {p.relative_to(self.root): p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        report = audit.audit(self.root, ['2026-10-12'])
        day = report['days']['2026-10-12']
        self.assertEqual(day['recovery_exercised_sequences'], [2])
        self.assertEqual(day['coverage'], 'closing_recovery_exercised')
        self.assertEqual(before, {p.relative_to(self.root): p.read_bytes() for p in self.root.rglob('*') if p.is_file()})

    def test_exact_cutoff_is_late_not_success(self):
        self.fixture(late=True)
        day = audit.audit(self.root, ['2026-10-12'])['days']['2026-10-12']
        self.assertEqual(day['coverage'], 'no_timely_closing_receipt')
        self.assertFalse(day['receipts'][0]['durable_before_cutoff'])

    def test_live_closing_is_common_input_not_recovery_evidence(self):
        self.fixture(contrast=False)
        self.assertEqual(audit.audit(self.root, ['2026-10-12'])['days']['2026-10-12']['coverage'],
                         'timely_closing_without_recovery_contrast')

    def test_future_session_is_pending_and_weekend_is_not_a_model_day(self):
        self.fixture()
        report = audit.audit(self.root, ['2026-10-13', '2026-10-11'])
        self.assertEqual(report['days']['2026-10-13']['coverage'], 'closing_pending_before_cutoff')
        self.assertFalse(report['days']['2026-10-13']['cutoff_reached'])
        self.assertEqual(report['days']['2026-10-11']['coverage'], 'non_model_date')

    def test_missing_terminal_and_unobserved_committed_input_are_explicit(self):
        self.fixture(observed=False)
        report = audit.audit(self.root, ['2026-10-12', '2026-10-09'])
        self.assertEqual(report['unobserved_ingested_receipts'], 2)
        self.assertIsNone(report['days']['2026-10-12']['receipts'][0]['arm_dispositions'])
        self.assertEqual(report['days']['2026-10-09']['coverage'], 'no_timely_closing_receipt')
        (self.root/'ingress/00000003.raw.json').write_text('in-progress writer tail')
        self.assertEqual(audit.audit(self.root, ['2026-10-12'])['snapshot_head'], report['snapshot_head'])

    def test_corruption_and_symlink_are_not_a_valid_audit(self):
        self.fixture()
        path = self.root/'ingress/00000002.raw.json'
        old = path.read_text(); path.write_text(old.replace('history_gap', 'history_gap_modified'))
        with self.assertRaisesRegex(ValueError, 'checksum'):
            audit.audit(self.root, ['2026-10-12'])
        path.write_text(old)
        head = self.root/'transactions/HEAD.json'; target = self.root.parent/'head-copy'
        target.write_bytes(head.read_bytes()); head.unlink(); head.symlink_to(target)
        with self.assertRaises(OSError):
            audit.audit(self.root, ['2026-10-12'])

    def test_payload_hash_uses_original_float_encoding(self):
        raw = '{"value":1e-7}'
        path = self.root/'float.json'
        path.write_text('{"payload":'+raw+',"sha256":"'+hashlib.sha256(raw.encode()).hexdigest()+'"}')
        value, _ = audit.Reader().checked(path, 1000)
        self.assertEqual(value['value'], 1e-7)


if __name__ == '__main__':
    unittest.main()
