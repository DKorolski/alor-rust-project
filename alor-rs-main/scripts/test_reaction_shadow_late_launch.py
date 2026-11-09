"""Pure diagnostic admission fixtures; not real UID, Redis or worker qualification."""
import datetime as dt
import json
from pathlib import Path
import unittest
from unittest import mock

import reaction_shadow_late_launch as launch
import reaction_shadow_preflight as p
import reaction_shadow_service as service
import test_reaction_shadow_preflight as fixtures


class LateLaunchTests(unittest.TestCase):
    def setUp(self):
        self.f = fixtures.LateContextTests('test_empty_late_context_review_is_still_required')
        self.f.setUp(); self.addCleanup(self.f.doCleanups); self.f.configure()
        self.now = self.f.epoch('2026-11-09T11:10:00')
        self.ready = '2026-11-09T11:30:00+03:00'
        cfg = json.loads(self.f.manifest.read_text())['config']
        cfg['first_live_session'] = '2026-11-10'
        cfg_sha = p.digest(json.dumps(cfg, sort_keys=True, separators=(',', ':')).encode())
        self.f.manifest.write_text(json.dumps(dict(version='shadow_gateway_ws_closed_m10_v4', config=cfg, config_sha256=cfg_sha)))
        self.f.health['config_sha256'] = cfg_sha; self.f.write_health()
        self.recovery = dict(last_source_id=str((self.now-600)*1000)+'-0', rows=self.f.bootstrap['rows']+25,
                             source_sha256='b'*64, last_label_utc=self.now-600)
        self.late = dict(version='diagnostic_late_start_v1', source_first_live_session='2026-11-10',
                         first_clean_session='2026-11-10', recovery=self.recovery)
        self.spec = self.f.cfg/'late.toml'
        self.save_spec()
        self.plan = dict(version='reaction_shadow_late_preflight_v1', paths={k:self.f.paths[k] for k in ('source','owner','generation','approval')},
                         files={key:dict(path=str(path), sha256=service.file_hash(path)) for key,path in
                                dict(worker=self.f.worker, supervisor=Path(service.__file__).resolve(),
                                     spec=self.spec, source_manifest=self.f.manifest).items()},
                         redis=self.f.plan['redis'])
        self.plan_path = self.f.cfg/'late-plan.json'
        self.plan_path.write_text(json.dumps(self.plan)); self.plan_sha = p.digest(self.plan_path.read_bytes())
        reports = [dict(item, kind='source_terminal' if item['kind']=='natural_session' else item['kind'])
                   for item in self.f.approval['evidence'] if item['kind'] != 'source_revisions']
        self.approval = dict(version='reaction_shadow_late_admission_v1', plan_sha256=self.plan_sha,
                             spec_sha256=p.digest(self.spec.read_bytes()), reviewed_at_utc=self.now-60,
                             ready_by=self.ready, gates=dict.fromkeys(launch.GATES,True),
                             revision_review=p.revision_snapshot(self.f.source,'USDRUBF',self.now),
                             late_context_review=p.late_context_snapshot(self.f.source,cfg,self.now), evidence=reports)
        self.save_approval()
        self.env = dict(REACTION_SHADOW_REDIS_URL=self.f.env['REACTION_SHADOW_REDIS_URL'], SHADOW_READY_BY=self.ready)
        self.preview = dict(event='late_start_inspected', order_emission_allowed=False,
                            preview=dict(bootstrap=self.f.bootstrap, recovery=self.recovery,
                                         first_clean_session=self.late['first_clean_session'],
                                         anchors=[[{},{}],[{},{}]], order_emission_allowed=False))
        self.reader = mock.Mock()
        self.reader.command.return_value = b'PONG'
        self.reader.envelope.return_value = (self.recovery['last_source_id'], dict(source='shadow_gateway_ws_closed_m10_v4',
             payload=dict(symbol='USDRUBF',close_time_utc=self.recovery['last_label_utc'])))
        self.inspected = mock.Mock(returncode=0,stdout=json.dumps(self.preview).encode())
        for patch in (mock.patch.object(p,'Reader',return_value=self.reader),
                      mock.patch.object(launch.subprocess,'run',return_value=self.inspected)):
            patch.start(); self.addCleanup(patch.stop)
        patch = mock.patch.object(service,'supervise',return_value={'fixture_only':True})
        self.supervise = patch.start(); self.addCleanup(patch.stop)
        self.events=[]

    def save_spec(self):
        raw = self.f.prepared_raw+'\n[runner.bootstrap]\n'+'\n'.join(f'{key} = {json.dumps(value)}' for key,value in self.f.bootstrap.items())
        raw += '\n[runner.late_start]\n'+'\n'.join(f'{key} = {json.dumps(value)}' for key,value in self.late.items() if key!='recovery')
        raw += '\n[runner.late_start.recovery]\n'+'\n'.join(f'{key} = {json.dumps(value)}' for key,value in self.recovery.items())+'\n'
        self.spec.write_text(raw)

    def save_approval(self):
        Path(self.f.paths['approval']).write_text(json.dumps(self.approval))

    def start(self, **kwargs):
        return launch.launch(self.plan_path,self.plan_sha,environ=self.env,clock=lambda:self.now,emit=self.events.append,**kwargs)

    def manifest(self):
        root=Path(self.f.paths['generation'])
        return root.with_name(root.name+'.supervisor')/'manifest.json'

    def test_readonly_preflight_does_not_reserve_or_start(self):
        result=self.start(preflight_only=True)
        self.assertFalse(result['worker_started'] or result['order_emission_allowed'])
        self.assertEqual(list(self.f.owner.iterdir()),[])
        self.supervise.assert_not_called()

    def test_grant_is_persisted_before_spawn_and_excludes_diagnostic_day(self):
        def inspect(*args,**kwargs):
            manifest=json.loads(self.manifest().read_text())
            self.assertEqual(manifest['version'],'reaction_shadow_supervisor_v4_late_start')
            grant=manifest['admission_grant']
            self.assertEqual(grant['diagnostic_session'],'2026-11-09')
            self.assertEqual(grant['first_clean_session'],'2026-11-10')
            self.assertFalse(grant['order_emission_allowed'])
            self.assertEqual(set(args[4]),{'REACTION_SHADOW_REDIS_URL','TZ'})
            self.assertTrue(kwargs['require_full_startup_window'])
            return {'fixture_only':True}
        self.supervise.side_effect=inspect
        canonical=(self.f.source/'committed.jsonl').read_bytes()
        self.start()
        self.assertEqual((self.f.source/'committed.jsonl').read_bytes(),canonical)
        self.assertEqual(json.loads(self.events[0])['event'],'reaction_shadow_late_grant_persisted')

    def test_same_day_cannot_be_counted_clean(self):
        self.late['first_clean_session']='2026-11-09'; self.save_spec()
        self.plan['files']['spec']['sha256']=service.file_hash(self.spec)
        self.plan_path.write_text(json.dumps(self.plan)); self.plan_sha=p.digest(self.plan_path.read_bytes())
        with self.assertRaisesRegex(ValueError,'diagnostic_day_cannot_be_clean'):self.start()
        self.supervise.assert_not_called()

    def test_stale_replay_tail_is_not_a_freshness_waiver(self):
        self.now+=1000; self.approval['reviewed_at_utc']=self.now-10
        self.approval['ready_by']='2026-11-09T11:50:00+03:00'
        self.env['SHADOW_READY_BY']=self.approval['ready_by'];self.save_approval()
        with self.assertRaisesRegex(ValueError,'diagnostic_recovery_tail_stale'):self.start()
        self.supervise.assert_not_called()

    def test_last_closed_prefix_is_current_between_ten_minute_closes(self):
        self.now+=480
        self.approval['reviewed_at_utc']=self.now
        self.approval['ready_by']='2026-11-09T11:40:00+03:00'
        self.env['SHADOW_READY_BY']=self.approval['ready_by'];self.save_approval()
        self.start(preflight_only=True)
        self.supervise.assert_not_called()

    def test_old_approval_and_short_startup_margin_are_blocked(self):
        self.approval['reviewed_at_utc']=self.now-301;self.save_approval()
        with self.assertRaisesRegex(ValueError,'fresh_diagnostic_review_required'):self.start()
        self.approval['reviewed_at_utc']=self.now; self.approval['ready_by']='2026-11-09T11:15:00+03:00'
        self.env['SHADOW_READY_BY']=self.approval['ready_by']; self.save_approval()
        with self.assertRaisesRegex(ValueError,'readiness_window_invalid'):self.start()

    def test_before_open_and_other_date_are_not_diagnostic_late_starts(self):
        for ts in (self.f.epoch('2026-11-09T06:20:00'),self.f.epoch('2026-11-10T11:10:00')):
            self.now=ts
            with self.assertRaisesRegex(ValueError,'diagnostic_same_day_window_required'):self.start()

    def test_new_audit_during_inspection_blocks_without_reservation(self):
        def changed(*args,**kwargs):
            self.f.late_row(); return self.inspected
        with mock.patch.object(launch.subprocess,'run',side_effect=changed):
            with self.assertRaisesRegex(ValueError,'audit_changed_during_inspection'):self.start()
        self.assertEqual(list(self.f.owner.iterdir()),[])

    def test_false_preview_and_missing_reader_boundary_are_blocked(self):
        self.reader.envelope.return_value=('wrong',{})
        with self.assertRaisesRegex(ValueError,'recovery_redis_boundary_mismatch'):self.start()
        self.supervise.assert_not_called()

    def test_inspector_cannot_change_recovery_or_enable_order_emission(self):
        for field in ('recovery', 'order_emission_allowed'):
            preview=json.loads(json.dumps(self.preview))
            preview['preview'][field] = {} if field=='recovery' else True
            inspected=mock.Mock(returncode=0,stdout=json.dumps(preview).encode())
            with mock.patch.object(launch.subprocess,'run',return_value=inspected):
                with self.assertRaisesRegex(ValueError,'diagnostic_inspection_mismatch'):self.start()
        self.assertEqual(list(self.f.owner.iterdir()),[])

    def test_changed_approval_after_check_is_not_a_permitted_audit_append(self):
        original=launch.check
        def changed(*args,**kwargs):
            result=original(*args,**kwargs)
            approval=Path(self.f.paths['approval'])
            approval.write_text(approval.read_text()+'\n')
            return result
        with mock.patch.object(launch,'check',side_effect=changed):
            with self.assertRaisesRegex(ValueError,'grant_approval_changed'):self.start()
        self.supervise.assert_not_called()

    def test_existing_strict_preflight_rejects_diagnostic_preparation(self):
        self.f.prepared.write_text(self.f.prepared_raw+'\n[runner.late_start]\nversion="diagnostic_late_start_v1"\n')
        self.f.plan['files']['prepared_spec']['sha256']=service.file_hash(self.f.prepared)
        self.f.plan['files']['source_manifest']['sha256']=service.file_hash(self.f.manifest)
        self.f.plan_path.write_text(json.dumps(self.f.plan)); self.f.plan_sha=p.digest(self.f.plan_path.read_bytes())
        with self.assertRaisesRegex(ValueError,'diagnostic_late_start_requires_dedicated_launcher'):
            self.f.check(now=self.now)

    def test_template_has_no_autostart_or_legacy_execstart(self):
        text=(Path(__file__).resolve().parents[1]/'configs/examples/reaction-shadow-worker-late.service').read_text()
        self.assertIn('ExecStart=/usr/bin/python3 @SCRIPTS@/reaction_shadow_late_launch.py',text)
        for setting in ('Restart=no','KillMode=mixed','ReadOnlyPaths=@SOURCE_STATE@ @CONFIG@','IPAddressDeny=any','CapabilityBoundingSet='):
            self.assertIn(setting,text)
        self.assertNotIn('[Install]',text)

    def test_broker_environment_and_operator_stop_are_blocked(self):
        self.env['REDIS_URL']='forbidden'
        with self.assertRaisesRegex(ValueError,'broker_or_generic_redis_environment_forbidden'):self.start()
        del self.env['REDIS_URL']
        stop=service.StopRequest(); stop.request()
        with self.assertRaises(InterruptedError):self.start(stop=stop)
        self.assertEqual(list(self.f.owner.iterdir()),[])

    def test_fsync_failure_never_spawns_and_retains_forensic_attempt(self):
        with mock.patch.object(service,'publish',side_effect=OSError('fixture fsync failure')):
            with self.assertRaises(OSError):self.start()
        self.supervise.assert_not_called();self.assertEqual(self.events,[])
        self.assertTrue(self.manifest().parent.exists())

    def test_duplicate_create_keeps_original_grant(self):
        self.start(); before=self.manifest().read_bytes()
        with self.assertRaisesRegex(ValueError,'generation_or_attempt_already_exists'):self.start()
        self.assertEqual(self.manifest().read_bytes(),before);self.assertEqual(self.supervise.call_count,1)


if __name__=='__main__':unittest.main()
