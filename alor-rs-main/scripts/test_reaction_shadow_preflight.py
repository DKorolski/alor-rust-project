import datetime as dt
import fcntl
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

import reaction_shadow_preflight as probe


class PreflightTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name).resolve()
        self.cfg, self.source, self.owner = [self.base / p for p in ('cfg', 'source', 'owner')]
        for p in (self.cfg, self.source, self.owner): p.mkdir()
        self.stream = 'shadow.source.closed10m.USDRUBF.fixture'
        self.worker = self.base / 'worker'
        self.worker.write_text('fixture inspect-only binary')
        self.worker.chmod(0o755)
        examples = Path(__file__).resolve().parents[1] / 'configs/examples'
        raw = (examples / 'reaction-shadow.usdrubf-h4-w4.toml').read_text()
        raw = raw.replace('2026-11-02', '2026-11-06').replace('excluded_dates = []', 'excluded_dates = ["2026-11-04"]')
        self.prepared_raw = raw.replace('REPLACE_WITH_APPROVED_USDRUBF_M10_SOURCE', self.stream)
        self.prepared = self.cfg / 'prepared.toml'; self.prepared.write_text(self.prepared_raw)
        cfg = json.loads((examples / 'shadow-md-ws.usdrubf.json').read_text())
        cfg.update(stream=self.stream, first_live_session='2026-11-06')
        cfg_sha = probe.digest(json.dumps(cfg, sort_keys=True, separators=(',', ':')).encode())
        self.manifest = self.source / 'manifest.json'
        self.manifest.write_text(json.dumps(dict(version='shadow_gateway_ws_closed_m10_v3', config=cfg, config_sha256=cfg_sha)))
        self.lock = (self.source / 'writer.lock').open('a+b')
        self.addCleanup(self.lock.close)
        fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.health = dict(version='shadow_gateway_ws_closed_m10_v3', config_sha256=cfg_sha,
                           heartbeat_utc=int(time.time()), last_ws_rx_utc=int(time.time()), generation=1,
                           status='ready', history_complete=True, connected=True, subscribed=True,
                           order_emission_allowed=False, ordinary_delayed_rows=0,
                           model_freshness_policy='consumer_owned_unchanged', review_dates=[])
        (self.source / 'HEALTH.json').write_text(json.dumps(self.health))
        self.paths = dict(source=str(self.source), owner=str(self.owner), generation=str(self.owner / 'generation'),
                          approved_spec=str(self.cfg / 'approved.toml'), approval=str(self.cfg / 'ADMISSION.json'))
        self.plan = dict(version='reaction_shadow_preflight_v1', paths=self.paths,
                         files={k: dict(path=str(p), sha256=probe.service.file_hash(p)) for k, p in dict(
                             worker=self.worker, supervisor=Path(probe.service.__file__).resolve(),
                             prepared_spec=self.prepared, source_manifest=self.manifest).items()},
                         redis=dict(host='127.0.0.1', port=6379, username='shadow_ws_reader_fixture'))
        self.plan_path = self.cfg / 'plan.json'; self.plan_path.write_text(json.dumps(self.plan))
        self.plan_sha = probe.digest(self.plan_path.read_bytes())
        self.env = dict(REACTION_SHADOW_REDIS_URL='redis://shadow_ws_reader_fixture:secret-marker@127.0.0.1:6379/0')
        self.reader = mock.Mock()
        self.reader.command.return_value = b'PONG'
        self.reader.envelope.return_value = ('1-0', dict(source='shadow_gateway_ws_closed_m10_v3', payload=dict(symbol='USDRUBF')))
        # Pure policy fixtures do not claim OS ownership isolation. A native
        # preparation probe separately checks the actual dedicated reader UID.
        self.access = mock.patch.object(probe.os, 'access', side_effect=lambda p, mode: mode != os.W_OK or Path(p) == self.owner)
        self.access.start(); self.addCleanup(self.access.stop)
        self.uid = mock.patch.object(probe.os, 'geteuid', return_value=1000)
        self.uid.start(); self.addCleanup(self.uid.stop)

    @staticmethod
    def epoch(text):
        return int(dt.datetime.fromisoformat(text).replace(tzinfo=probe.MSK).timestamp())

    def check(self, stage='preparation', now=None):
        with mock.patch.object(probe, 'Reader', return_value=self.reader):
            return probe.preflight(self.plan_path, self.plan_sha, stage, self.env,
                                   self.epoch('2026-11-04T10:00:00') if now is None else now)

    def admission(self):
        self.now = self.epoch('2026-11-06T06:20:00')
        self.env['SHADOW_READY_BY'] = '2026-11-06T06:55:00+03:00'
        close = self.epoch('2026-11-05T23:50:00'); label = close - 600
        p = dict(reason='session_boundary_ws', generation=1, closed_at_utc=close, next_label_utc=None,
                 first_received_at_utc=label, last_received_at_utc=close - 1, observation_cutoff_utc=close + 10)
        records = []
        for ts in range(self.epoch('2026-11-05T07:00:00'), label + 1, 600):
            payload = dict(symbol='USDRUBF', close_time_utc=ts, o=100., h=101., l=99., c=100., v=1., origin='history')
            records.append(dict(id=str(ts * 1000) + '-0', envelope=dict(source='shadow_gateway_ws_closed_m10_v3',
                                ts_utc=ts + 610, payload=payload), release=None))
        self.terminal = records[-1]
        self.terminal['release'] = dict(id='synthetic-terminal', proof=p,
                                        bar={**self.terminal['envelope']['payload'], 'origin': 'live'})
        self.terminal['envelope']['payload']['source_proof'] = dict(release_id='synthetic-terminal', release=p)
        self.records = records
        self.save_records()
        self.bootstrap = dict(first_source_id=str(self.epoch('2026-11-03T07:00:00') * 1000) + '-0',
                              last_source_id=self.terminal['id'], rows=202, source_sha256='a' * 64,
                              first_session='2026-11-03', last_session='2026-11-05')
        approved = self.prepared_raw + '\n[runner.bootstrap]\n' + '\n'.join(f'{k} = {json.dumps(v)}' for k, v in self.bootstrap.items())
        Path(self.paths['approved_spec']).write_text(approved)
        evidence = self.cfg / 'evidence'; evidence.mkdir()
        reports = []
        for kind in ('natural_session', 'overnight_recovery'):
            path = evidence / (kind + '.json'); path.write_text('{"synthetic_policy_fixture":true}')
            reports.append(dict(kind=kind, path=str(path), sha256=probe.digest(path.read_bytes())))
        self.approval = dict(version='reaction_shadow_admission_v1', plan_sha256=self.plan_sha,
                             approved_spec_sha256=probe.digest(approved.encode()), reviewed_at_utc=self.now - 60,
                             ready_by=self.env['SHADOW_READY_BY'], gates=dict.fromkeys(probe.GATES, True), evidence=reports)
        self.save_approval()
        self.reader.envelope.side_effect = lambda first='-', last='+': (
            (self.terminal['id'], self.terminal['envelope']) if first != '-' else
            ('1-0', dict(source='shadow_gateway_ws_closed_m10_v3', payload=dict(symbol='USDRUBF'))))
        self.inspected = dict(event='bootstrap_inspected', order_emission_allowed=False,
                              preview=dict(order_emission_allowed=False, bootstrap=self.bootstrap, anchors=[[{}, {}], [{}, {}]]))
        self.completed = subprocess.CompletedProcess([], 0, json.dumps(self.inspected).encode(), b'')

    def save_records(self):
        (self.source / 'committed.jsonl').write_text(''.join(json.dumps(dict(record=r)) + '\n' for r in self.records))

    def save_approval(self):
        Path(self.paths['approval']).write_text(json.dumps(self.approval))

    def test_preparation_never_selects_bootstrap_starts_worker_or_writes_state(self):
        before = {str(p): p.read_bytes() for p in self.base.rglob('*') if p.is_file()}
        with mock.patch.object(probe.subprocess, 'run') as run:
            result = self.check()
        self.assertTrue(result['preparation_ready'])
        self.assertFalse(result['admission_allowed'] or result['bootstrap_pinned'] or result['worker_started'])
        self.assertEqual(set(result['pending_gates']), probe.GATES)
        self.assertEqual((result['ordinary_model_limit'], result['terminal_model_limit'], result['queue_limit']), (360, 120, 30))
        run.assert_not_called()
        self.assertEqual(before, {str(p): p.read_bytes() for p in self.base.rglob('*') if p.is_file()})
        self.assertEqual(list(self.owner.iterdir()), [])

    def test_admission_missing_and_expired_model_date_are_blocked(self):
        with self.assertRaisesRegex(ValueError, 'admission_missing'): self.check('admission')
        with self.assertRaisesRegex(ValueError, 'model_date_expired'): self.check(now=self.epoch('2026-11-06T07:00:00'))

    def test_changed_plan_and_pinned_file_are_blocked(self):
        original = self.plan_path.read_bytes(); self.plan_path.write_bytes(original + b' ')
        with self.assertRaisesRegex(ValueError, 'plan_changed'): self.check()
        self.plan_path.write_bytes(original); self.worker.write_text('changed')
        with self.assertRaisesRegex(ValueError, 'pinned_file_changed'): self.check()

    def test_existing_attempt_root_and_foreign_environment_are_blocked(self):
        self.env['ALOR_REFRESH_TOKEN'] = 'do-not-print'
        with self.assertRaisesRegex(ValueError, 'environment_forbidden'): self.check()
        del self.env['ALOR_REFRESH_TOKEN']
        Path(self.paths['generation'] + '.supervisor').mkdir()
        with self.assertRaisesRegex(ValueError, 'already_exists'): self.check()

    def test_symlink_and_writable_source_are_blocked(self):
        with mock.patch.object(probe.os, 'access', return_value=True):
            with self.assertRaisesRegex(ValueError, 'writable_by_worker'): self.check()
        self.prepared.unlink(); self.prepared.symlink_to(self.worker)
        with self.assertRaisesRegex(ValueError, 'canonical_regular_file'): self.check()

    def test_missing_or_stale_source_blocks_preparation(self):
        (self.source / 'FAILED.json').write_text('{}')
        with self.assertRaises(probe.service.SourceUnavailable): self.check()
        (self.source / 'FAILED.json').unlink()
        (self.source / 'HEALTH.json').write_text(json.dumps({**self.health, 'heartbeat_utc': 0}))
        with self.assertRaises(probe.service.SourceUnavailable): self.check()

    def test_approved_admission_calls_only_readonly_inspect_and_preserves_source(self):
        self.admission()
        before = (self.source / 'committed.jsonl').read_bytes()
        with mock.patch.object(probe.subprocess, 'run', return_value=self.completed) as run:
            result = self.check('admission', self.now)
        self.assertTrue(result['admission_allowed'])
        self.assertFalse(result['worker_started'] or result['generation_created'] or result['order_emission_allowed'])
        self.assertEqual(run.call_args.args[0][1], 'inspect-bootstrap')
        self.assertEqual(set(run.call_args.kwargs['env']), {'REACTION_SHADOW_REDIS_URL', 'TZ'})
        self.assertEqual(before, (self.source / 'committed.jsonl').read_bytes())
        self.assertEqual(list(self.owner.iterdir()), [])

    def test_unapproved_stale_and_short_readiness_window_block_admission(self):
        self.admission()
        self.approval['gates']['overnight_recovery'] = False; self.save_approval()
        with self.assertRaisesRegex(ValueError, 'not_approved'): self.check('admission', self.now)
        self.approval['gates']['overnight_recovery'] = True
        self.approval['reviewed_at_utc'] = self.now - 3601; self.save_approval()
        with self.assertRaisesRegex(ValueError, 'morning_review'): self.check('admission', self.now)
        self.approval['reviewed_at_utc'] = self.now
        self.approval['ready_by'] = '2026-11-06T06:25:00+03:00'; self.env['SHADOW_READY_BY'] = self.approval['ready_by']; self.save_approval()
        with self.assertRaisesRegex(ValueError, 'window_invalid'): self.check('admission', self.now)

    def test_model_or_reviewed_evidence_changes_block_admission(self):
        self.admission()
        path = Path(self.paths['approved_spec']); original = path.read_bytes(); path.write_bytes(original.replace(b'mr_horizon = 4', b'mr_horizon = 3'))
        self.approval['approved_spec_sha256'] = probe.digest(path.read_bytes()); self.save_approval()
        with self.assertRaisesRegex(ValueError, 'only_bootstrap'): self.check('admission', self.now)
        path.write_bytes(original); self.approval['approved_spec_sha256'] = probe.digest(original); self.save_approval()
        Path(self.approval['evidence'][0]['path']).write_text('changed')
        with self.assertRaisesRegex(ValueError, 'evidence_changed'): self.check('admission', self.now)

    def test_historical_terminal_holes_and_price_revisions_block_admission(self):
        self.admission()
        release = self.terminal['release']; self.terminal['release'] = None; self.save_records()
        with self.assertRaisesRegex(ValueError, 'historical_terminal'): self.check('admission', self.now)
        self.terminal['release'] = release
        removed = self.records.pop(2); self.save_records()
        with self.assertRaisesRegex(ValueError, 'session_not_complete'): self.check('admission', self.now)
        self.records.insert(2, removed); self.save_records()
        (self.source / 'HEALTH.json').write_text(json.dumps({**self.health, 'review_dates': ['2026-11-05']}))
        with self.assertRaisesRegex(ValueError, 'price_revision'): self.check('admission', self.now)

    def test_terminal_race_reasons_and_deadline_are_validated(self):
        self.admission()
        proof = self.terminal['release']['proof']
        proof.update(reason='next_timestamp_delayed', next_label_utc=proof['closed_at_utc']); self.save_records()
        with self.assertRaisesRegex(ValueError, 'release_reason'): self.check('admission', self.now)
        proof.update(reason='next_timestamp')
        self.save_records()
        with mock.patch.object(probe.subprocess, 'run', return_value=self.completed):
            self.assertTrue(self.check('admission', self.now)['admission_allowed'])
        proof['observation_cutoff_utc'] += 100; self.save_records()
        with self.assertRaisesRegex(ValueError, 'structural_proof'): self.check('admission', self.now)

    def test_bootstrap_failure_or_redis_terminal_mismatch_blocks_admission(self):
        self.admission()
        with mock.patch.object(probe.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, b'', b'secret-marker')):
            with self.assertRaisesRegex(ValueError, 'inspection_failed'): self.check('admission', self.now)
        self.reader.envelope.side_effect = None
        self.reader.envelope.return_value = ('wrong', dict(source='shadow_gateway_ws_closed_m10_v3', payload=dict(symbol='USDRUBF')))
        with self.assertRaisesRegex(ValueError, 'prefix_mismatch'): self.check('admission', self.now)

    def test_cli_redacts_credentials_and_creates_no_state(self):
        env = {**os.environ, 'ALOR_REFRESH_TOKEN': 'do-not-print', **self.env}
        result = subprocess.run([sys.executable, probe.__file__, '--plan', str(self.plan_path),
                                 '--plan-sha256', self.plan_sha, '--stage', 'admission'], capture_output=True, env=env)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(b'do-not-print', result.stdout + result.stderr)
        self.assertNotIn(b'secret-marker', result.stdout + result.stderr)
        self.assertFalse(json.loads(result.stdout)['worker_started'])
        self.assertEqual(list(self.owner.iterdir()), [])

    def test_price_revision_during_inspection_and_invalid_preview_block_admission(self):
        self.admission()
        self.inspected['preview']['anchors'] = [[{}], [{}]]
        invalid = subprocess.CompletedProcess([], 0, json.dumps(self.inspected).encode(), b'')
        with mock.patch.object(probe.subprocess, 'run', return_value=invalid):
            with self.assertRaisesRegex(ValueError, 'inspection_mismatch'): self.check('admission', self.now)
        def revised(*args, **kwargs):
            (self.source / 'HEALTH.json').write_text(json.dumps({**self.health, 'review_dates': ['2026-11-05']}))
            return self.completed
        with mock.patch.object(probe.subprocess, 'run', side_effect=revised):
            with self.assertRaisesRegex(ValueError, 'revision_during_inspection'): self.check('admission', self.now)
        self.assertEqual(list(self.owner.iterdir()), [])

    def test_root_and_shared_source_uid_cannot_pass_preparation(self):
        for uid, reason in ((0, 'nonroot_reader'), (self.source.stat().st_uid, 'uid_must_differ')):
            with mock.patch.object(probe.os, 'geteuid', return_value=uid):
                with self.assertRaisesRegex(ValueError, reason): self.check()

    def test_unit_template_keeps_all_admission_conditions_and_no_autostart(self):
        text = (Path(__file__).resolve().parents[1] / 'configs/examples/reaction-shadow-worker-prepared.service').read_text()
        for field in ('ADMISSION.json', 'approved-worker.toml', 'admission.env'):
            self.assertIn('ConditionPathExists=@CONFIG@/' + field, text)
        self.assertIn('ConditionPathExists=!@OWNER@/generation.supervisor', text)
        self.assertIn('--stage admission', text)
        self.assertIn('SupplementaryGroups=@SOURCE_GROUP@', text)
        self.assertIn('ReadOnlyPaths=@SOURCE_STATE@ @CONFIG@', text)
        self.assertIn('IPAddressAllow=@REDIS_IP@/32', text)
        self.assertIn('Restart=no', text)
        for forbidden in ('\n[Install]', '\nWants=', '\nRequires=', 'Restart=always'):
            self.assertNotIn(forbidden, text)


class ReaderTests(unittest.TestCase):
    def test_reader_binding_and_opcode_whitelist(self):
        binding = dict(host='127.0.0.1', port=1234, username='shadow_ws_reader_fixture')
        sock = mock.Mock(); sock.makefile.return_value = io.BytesIO(b'+OK\r\n+PONG\r\n')
        with mock.patch.object(probe.socket, 'create_connection', return_value=sock) as connect:
            with self.assertRaisesRegex(ValueError, 'binding_mismatch'):
                probe.Reader('redis://shadow_ws_reader_fixture:secret@127.0.0.2:1234/0', binding, 'fixture')
            connect.assert_not_called()
            reader = probe.Reader('redis://shadow_ws_reader_fixture:secret@127.0.0.1:1234/0', binding, 'fixture')
            self.assertEqual(reader.command('PING'), b'PONG')
            sent = sock.sendall.call_count
            for args in [('XADD', 'fixture', '*', 'payload', '{}'), ('XRANGE', 'other', '-', '+', 'COUNT', '1'), ('SELECT', '1')]:
                with self.assertRaisesRegex(ValueError, 'non_readonly'): reader.command(*args)
            self.assertEqual(sock.sendall.call_count, sent)
            reader.close()

    def test_reply_size_and_redis_error_do_not_escape(self):
        for raw, reason in [(b'-ERR credentials-secret\r\n', 'denied_or_failed'), (b'$65537\r\n', 'reply_size'), (b'*17\r\n', 'reply_array')]:
            reader = object.__new__(probe.Reader); reader.file = io.BytesIO(raw); reader.remaining = 128 * 1024
            with self.assertRaisesRegex(ValueError, reason) as error: reader.reply()
            self.assertNotIn('credentials-secret', str(error.exception))
            reader.file.close()


if __name__ == '__main__':
    unittest.main()
