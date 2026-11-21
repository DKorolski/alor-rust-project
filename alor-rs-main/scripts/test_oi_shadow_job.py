import datetime as dt
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import oi_shadow_job as job


class JobTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / 'store').mkdir()
        (self.root / 'reports').mkdir()
        self.binary = self.root / 'binary'
        self.binary.write_bytes(b'controlled fixture; never executed')
        self.binary.chmod(0o555)
        self.calendar = self.write('calendar.json', dict(symbol='IMOEXF', schema_version='moex_shadow_calendar_v1'))
        config = dict(model_sha256=job.MODEL, store_root=str(self.root / 'store'),
                      calendar_path=self.calendar['path'], calendar_pin=dict(artifact_sha256=self.calendar['sha256']))
        self.config = self.write('collect.json', dict(config, decision_date='2026-11-23'))
        self.probe = self.write('probe.json', dict(config, decision_date='2026-11-20'))
        self.spec = dict(version='imoexf_oi_collect_only_schedule_v1',
                         binary=dict(path=str(self.binary), sha256=job.sha(self.binary.read_bytes())),
                         calendar=self.calendar, configs={'2026-11-23': self.config},
                         probe_config=self.probe, store_root=str(self.root / 'store'),
                         report_root=str(self.root / 'reports'))
        self.manifest = self.write('release.json', self.spec)
        self.calls = []
        self.now = dt.datetime(2026, 11, 23, 3, 45, tzinfo=dt.timezone.utc)

    def write(self, name, value):
        path = self.root / name
        raw = json.dumps(value).encode()
        path.write_bytes(raw)
        path.chmod(0o444)
        return dict(path=str(path), sha256=job.sha(raw))

    def fake_native(self, spec, mode, config, timeout):
        self.calls.append(mode)
        day = json.loads(config.read_text())['decision_date']
        plan = dict(decision_date=day, profile_id='IMO_AB_OI', model_sha256=job.MODEL)
        if mode == 'plan':
            return 0, plan, ''
        if mode == 'clock':
            return 0, dict(synchronized=True), ''
        if mode == 'probe':
            return 0, dict(version='imoexf_oi_non_pit_probe_v1', plan=plan,
                           diagnostic_only=True, qualified=False, allow_new_bo=False, passed=True), ''
        return 0, dict(schema_version='imoexf_oi_durable_qualification_v1', plan=plan,
                       status='accepted_snapshot', evidence_kind='linux_kernel_clock', allow_new_bo=False), ''

    def execute(self, mode='run'):
        with patch.object(job, 'utc_now', return_value=self.now), patch.object(job, 'native', self.fake_native):
            return job.execute(self.manifest['path'], self.manifest['sha256'], mode)

    def test_timely_collect_is_dated_and_not_model_connected(self):
        output, code = self.execute()
        self.assertEqual(code, 0)
        self.assertEqual(self.calls, ['plan', 'clock', 'collect'])
        self.assertFalse(output['models_connected'])
        self.assertFalse(output['order_emission_allowed'])
        self.assertFalse(output['data_gate_allow_only'])

    def test_diagnostic_outside_window_never_writes_production_store(self):
        self.now = dt.datetime(2026, 11, 20, 12, 0, tzinfo=dt.timezone.utc)
        output, code = self.execute('probe')
        self.assertEqual(code, 0)
        self.assertEqual(self.calls, ['plan', 'clock', 'probe'])
        self.assertEqual(list((self.root / 'store').iterdir()), [])
        stored = json.loads(Path(output['report']).read_text())
        self.assertFalse(stored['result']['qualified'])

    def test_no_catchup_or_request_after_deadline_or_without_commit_reserve(self):
        for hour, minute, second in [(3, 44, 59), (3, 54, 51), (3, 55, 0), (12, 0, 0)]:
            self.now = dt.datetime(2026, 11, 23, hour, minute, second, tzinfo=dt.timezone.utc)
            with self.assertRaisesRegex(ValueError, 'window'):
                self.execute()
        self.assertEqual(self.calls, [])

    def test_no_implicit_weekday_or_future_horizon_extension(self):
        self.now = dt.datetime(2026, 11, 24, 3, 45, tzinfo=dt.timezone.utc)
        output, code = self.execute()
        self.assertEqual(code, 0)
        self.assertEqual(output['event'], 'oi_capture_not_scheduled')
        self.assertEqual(self.calls, [])

    def test_tampered_calendar_and_binary_are_rejected_before_native_call(self):
        self.binary.chmod(0o755)
        self.binary.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'hash mismatch'):
            self.execute()
        self.assertEqual(self.calls, [])

    def test_duplicate_json_and_symlink_config_are_rejected(self):
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            job.parse(b'{"a":1,"a":2}')
        alias = self.root / 'alias.json'
        alias.symlink_to(self.config['path'])
        with self.assertRaisesRegex(ValueError, 'canonical'):
            job.read_file(alias)

    def test_native_clock_failure_never_calls_collect(self):
        original = self.fake_native

        def native(*args):
            if args[1] == 'clock':
                return 1, dict(synchronized=False), 'clock invalid'
            return original(*args)

        with patch.object(job, 'native', native), patch.object(job, 'utc_now', return_value=self.now):
            with self.assertRaisesRegex(ValueError, 'clock'):
                job.execute(self.manifest['path'], self.manifest['sha256'], 'run')
        self.assertNotIn('collect', self.calls)

    def test_probe_cannot_claim_a_qualified_or_enabled_result(self):
        original = self.fake_native

        def native(*args):
            code, data, error = original(*args)
            if args[1] == 'probe':
                data['qualified'] = True
            return code, data, error

        with patch.object(job, 'native', native), patch.object(job, 'utc_now', return_value=self.now):
            output, code = job.execute(self.manifest['path'], self.manifest['sha256'], 'probe')
        self.assertEqual(code, 1)
        self.assertFalse(output['okay'])


if __name__ == '__main__':
    unittest.main()
