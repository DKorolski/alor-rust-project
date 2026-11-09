"""Offline grant/launch protocol fixtures. No worker, Redis, broker or host mutation."""
import json
from pathlib import Path
import unittest
from unittest import mock

import reaction_shadow_launch as launch
import reaction_shadow_preflight as preflight
import reaction_shadow_service as service
import test_reaction_shadow_preflight as fixtures


class GrantedLaunchTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.LateContextTests('test_empty_late_context_review_is_still_required')
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.fixture.configure()
        self.env = {**self.fixture.env, 'SHADOW_READY_BY':self.fixture.approval['ready_by'], 'UNRELATED':'not-forwarded'}
        self.events = []
        self.reader = mock.patch.object(preflight, 'Reader', return_value=self.fixture.reader)
        self.inspector = mock.patch.object(preflight.subprocess, 'run', return_value=self.fixture.completed)
        self.supervisor = mock.patch.object(service, 'supervise', return_value={'fixture_only':True})
        for patch in [self.reader, self.inspector, self.supervisor]:
            value = patch.start(); self.addCleanup(patch.stop)
            if patch is self.supervisor: self.supervise = value

    def start(self, **kwargs):
        options = dict(environ=self.env, clock=lambda:self.fixture.now, emit=self.events.append)
        options.update(kwargs)
        return launch.launch(self.fixture.plan_path, self.fixture.plan_sha,
                             self.fixture.approval['ready_by'], **options)

    def manifest(self):
        root = Path(self.fixture.paths['generation'])
        return root.with_name(root.name+'.supervisor')/'manifest.json'

    def test_grant_is_durable_and_pinned_before_supervise(self):
        def inspect(*args, **kwargs):
            persisted = json.loads(self.manifest().read_text())
            self.assertEqual(persisted['version'], 'reaction_shadow_supervisor_v3')
            grant = persisted['admission_grant']
            self.assertEqual(grant['version'], 'reaction_shadow_admission_grant_v1')
            self.assertEqual(grant['policy'], 'as_observed_reviewed_snapshot_grant_v1')
            self.assertEqual(grant['plan_sha256'], self.fixture.plan_sha)
            self.assertEqual(grant['approval_sha256'], preflight.digest(Path(self.fixture.paths['approval']).read_bytes()))
            self.assertEqual(grant['bootstrap'], self.fixture.bootstrap)
            self.assertEqual(grant['late_context_review']['rows'], 0)
            self.assertFalse(grant['order_emission_allowed'])
            child_env = args[4]
            self.assertEqual(set(child_env), {'REACTION_SHADOW_REDIS_URL','TZ'})
            self.assertTrue(kwargs['require_full_startup_window'])
            return {'fixture_only':True}
        self.supervise.side_effect = inspect
        canonical = (self.fixture.source/'committed.jsonl').read_bytes()
        self.start()
        self.assertEqual(canonical, (self.fixture.source/'committed.jsonl').read_bytes())
        self.assertEqual(json.loads(self.events[0])['event'], 'reaction_shadow_admission_grant_persisted')

    def test_audit_append_between_check_and_commit_is_allowed_without_rewriting_grant(self):
        original = service.Attempt
        def after_grant(*args, **kwargs):
            self.fixture.late_row()
            return original(*args, **kwargs)
        with mock.patch.object(service, 'Attempt', side_effect=after_grant): self.start()
        grant = json.loads(self.manifest().read_text())['admission_grant']
        self.assertEqual(grant['late_context_review']['rows'], 0)
        self.assertEqual(self.fixture.health['late_context_rows'], 1)
        self.supervise.call_args.kwargs['source_guard'].check(require_ready=True)

    def test_audit_change_during_preflight_never_reserves_attempt(self):
        def changed(*args, **kwargs):
            self.fixture.late_row(); return self.fixture.completed
        with mock.patch.object(preflight.subprocess, 'run', side_effect=changed):
            with self.assertRaisesRegex(ValueError, 'late_context_changed'): self.start()
        self.assertEqual(list(self.fixture.owner.iterdir()), [])
        self.supervise.assert_not_called()

    def test_grant_fsync_failure_never_spawns_or_claims_success(self):
        with mock.patch.object(service, 'publish', side_effect=OSError('fixture fsync failure')):
            with self.assertRaises(OSError): self.start()
        self.supervise.assert_not_called()
        self.assertEqual(self.events, [])
        self.assertTrue(self.manifest().parent.is_dir())  # forensic attempt retained

    def test_pinned_spec_change_after_preflight_is_rejected(self):
        original = preflight.preflight
        def changed(*args, **kwargs):
            result = original(*args, **kwargs)
            self.fixture.prepared.with_name('approved.toml').write_text('changed fixture')
            return result
        with mock.patch.object(preflight, 'preflight', side_effect=changed):
            with self.assertRaisesRegex(ValueError, 'grant_pinned_file_changed'): self.start()
        self.supervise.assert_not_called()
        self.assertEqual(list(self.fixture.owner.iterdir()), [])

    def test_startup_budget_is_rechecked_before_grant_commit(self):
        with self.assertRaisesRegex(ValueError, 'grant_startup_window_invalid'):
            self.start(clock=iter([self.fixture.now,self.fixture.now+2000]).__next__)
        self.supervise.assert_not_called()
        self.assertEqual(list(self.fixture.owner.iterdir()), [])

    def test_approval_change_after_preflight_is_not_an_allowed_audit_append(self):
        original = preflight.preflight
        def changed(*args, **kwargs):
            result = original(*args, **kwargs)
            approval = Path(self.fixture.paths['approval'])
            approval.write_text(approval.read_text()+'\n')
            return result
        with mock.patch.object(preflight, 'preflight', side_effect=changed):
            with self.assertRaisesRegex(ValueError, 'grant_approval_changed'): self.start()
        self.supervise.assert_not_called()
        self.assertEqual(list(self.fixture.owner.iterdir()), [])

    def test_operator_stop_before_commit_does_not_reserve_attempt(self):
        stop = service.StopRequest(); stop.request()
        with self.assertRaises(InterruptedError): self.start(stop=stop)
        self.supervise.assert_not_called()
        self.assertEqual(list(self.fixture.owner.iterdir()), [])

    def test_a_second_create_does_not_overwrite_grant(self):
        self.start(); before = self.manifest().read_bytes()
        with self.assertRaisesRegex(ValueError, 'generation_or_attempt_already_exists'): self.start()
        self.assertEqual(self.manifest().read_bytes(), before)
        self.assertEqual(self.supervise.call_count, 1)

    def test_legacy_resume_cannot_drop_the_grant_from_manifest(self):
        self.start()
        root = Path(self.fixture.paths['generation'])
        manifest = json.loads(self.manifest().read_text())
        legacy = {k:manifest[k] for k in ['generation','binary_sha256','spec_sha256','closed_source']}
        legacy['version'] = 'reaction_shadow_supervisor_v2'
        with self.assertRaisesRegex(ValueError, 'binary/spec/generation changed'):
            service.Attempt(root, 'resume', legacy)

    def test_older_plan_is_not_silently_upgraded(self):
        plan = dict(self.fixture.plan, version='reaction_shadow_preflight_v2')
        self.fixture.plan_path.write_text(json.dumps(plan))
        self.fixture.plan_sha = preflight.digest(self.fixture.plan_path.read_bytes())
        with self.assertRaisesRegex(ValueError, 'grant_launcher_requires_v3'): self.start()
        self.supervise.assert_not_called()

    def test_template_uses_the_opt_in_launcher_and_no_autostart(self):
        path = Path(__file__).resolve().parents[1]/'configs/examples/reaction-shadow-worker-granted.service'
        text = path.read_text()
        self.assertIn('ExecStart=/usr/bin/python3 @SCRIPTS@/reaction_shadow_launch.py', text)
        self.assertNotIn('ExecStartPre=', text)
        self.assertNotIn('[Install]', text)
        for part in ['ADMISSION.json','approved-worker.toml','admission.env']:
            self.assertIn('ConditionPathExists=@CONFIG@/'+part, text)
        for setting in ['Restart=no','KillMode=mixed','IPAddressAllow=@REDIS_IP@/32',
                        'ReadOnlyPaths=@SOURCE_STATE@ @CONFIG@','ReadWritePaths=@OWNER@']:
            self.assertIn(setting, text)


if __name__=='__main__':unittest.main()
