"""Owned policy fixtures. No broker, real Redis, host unit or real model start."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock

import reaction_shadow_launch as launch
import reaction_shadow_preflight as preflight
import reaction_shadow_service as service
import shadow_source_continuity as continuity
import test_reaction_shadow_preflight as fixtures


PROCESS = dict(pid=4242,start_ticks=12345,boot_id='00000000-0000-0000-0000-000000000001')


class ReuseTests(unittest.TestCase):
    def setUp(self):
        self.process = mock.patch.object(continuity,'inspect_process',return_value=PROCESS.copy())
        self.process.start();self.addCleanup(self.process.stop)
        self.f = self.fixture()

    def fixture(self,symbol='USDRUBF'):
        f = fixtures.LateContextTests('test_empty_late_context_review_is_still_required')
        f.setUp();self.addCleanup(f.doCleanups);f.configure(symbol)
        cfg = json.loads(f.manifest.read_text())['config']
        cfg['first_live_session'] = '2026-11-06'  # model remains 9 November
        cfg_sha = continuity.fingerprint(cfg)
        f.manifest.write_text(json.dumps(dict(version='shadow_gateway_ws_closed_m10_v4',config=cfg,config_sha256=cfg_sha)))
        f.health['config_sha256'] = cfg_sha;f.write_health()
        helper = Path(continuity.__file__).resolve()
        binding = dict(version=continuity.VERSION,source_root=str(f.source),
            source_manifest_sha256=preflight.digest(f.manifest.read_bytes()),symbol=symbol,source_stream=f.stream,
            source_first_live_session='2026-11-06',model_first_session='2026-11-09',
            prepared_spec=dict(path=str(f.prepared),sha256=preflight.digest(f.prepared.read_bytes())),
            process=PROCESS.copy(),minimum_ws_generation=1)
        f.plan.update(version='reaction_shadow_preflight_v4',source_continuity=binding)
        f.plan['files']['source_manifest']['sha256'] = binding['source_manifest_sha256']
        f.plan['files']['continuity_helper'] = dict(path=str(helper),sha256=service.file_hash(helper))
        f.approval.update(version='reaction_shadow_admission_v4',source_continuity_sha256=continuity.fingerprint(binding))
        self.reseal(f)
        def envelope(first='-',last='+'):
            for row in f.records:
                sid = row['id'];key = tuple(map(int,sid.split('-')))
                if first != '-':
                    lower = tuple(map(int,first.lstrip('(').split('-')))
                    if key < lower or (first.startswith('(') and key == lower):continue
                if last != '+' and key > tuple(map(int,last.split('-'))):continue
                return sid,copy.deepcopy(row['envelope'])
            raise ValueError('fixture reader row unavailable')
        f.reader.envelope.side_effect = envelope
        f.reader.stream = f.stream
        f.reader.command.side_effect = lambda *args: b'PONG' if args == ('PING',) else []
        return f

    @staticmethod
    def reseal(f):
        f.plan_path.write_text(json.dumps(f.plan));f.plan_sha=preflight.digest(f.plan_path.read_bytes())
        f.approval['plan_sha256']=f.plan_sha;f.save_approval()

    def check(self,f=None,stage='admission',**kwargs):
        f = f or self.f
        with mock.patch.object(preflight,'Reader',return_value=f.reader),mock.patch.object(preflight.subprocess,'run',return_value=f.completed):
            return preflight.preflight(f.plan_path,f.plan_sha,stage,f.env,now=kwargs.get('now',f.now))

    def start(self,f=None,supervise=None):
        f=f or self.f
        with mock.patch.object(preflight,'Reader',return_value=f.reader),mock.patch.object(preflight.subprocess,'run',return_value=f.completed),mock.patch.object(service,'supervise',side_effect=supervise or (lambda *a,**k:{'fixture_only':True})) as run:
            value=launch.launch(f.plan_path,f.plan_sha,f.approval['ready_by'],environ=f.env,clock=lambda:f.now,emit=lambda _:None)
            return value,run

    @staticmethod
    def manifest(f):
        root=Path(f.paths['generation'])
        return root.with_name(root.name+'.supervisor')/'manifest.json'

    def clean_stop(self):
        def owned_fixture(command,policy,ready,attempt,*args,**kwargs):
            Path(self.f.paths['generation']).mkdir()
            attempt.start(ready);attempt.stopped('operator_stop',{'fixture_only':True})
            return {'fixture_only':True}
        self.start(supervise=owned_fixture)

    def resume(self,when=None,ready=None):
        f=self.f
        with mock.patch.object(preflight,'Reader',return_value=f.reader),mock.patch.object(service,'supervise',return_value={'fixture_only':True}) as run:
            result=launch.resume(f.plan_path,f.plan_sha,ready or f.approval['ready_by'],environ=f.env,
                clock=lambda: f.now if when is None else when,emit=lambda _:None)
            return result,run

    def test_old_strict_guard_still_rejects_source_model_date_mismatch(self):
        with self.assertRaisesRegex(ValueError,'identity or calendar mismatch'):
            service.ClosedSourceGuard(self.f.source,self.f.prepared_raw.encode())
        self.assertEqual(list(self.f.owner.iterdir()),[])

    def test_both_symbols_admit_with_unchanged_canonical_data_and_original_model_date(self):
        for f in (self.f if symbol=='USDRUBF' else self.fixture(symbol) for symbol in ('USDRUBF','IMOEXF')):
            before=(f.source/'committed.jsonl').read_bytes()
            result=self.check(f)
            grant=result['admission_grant']
            self.assertTrue(result['admission_allowed'])
            self.assertEqual(grant['version'],'reaction_shadow_admission_grant_v2')
            self.assertEqual(grant['source_continuity']['source_first_live_session'],'2026-11-06')
            self.assertEqual(result['model_first_session'],'2026-11-09')
            self.assertEqual(result['reader_prefix_verified_rows'],len(f.records))
            self.assertFalse(result['worker_started'] or result['generation_created'] or result['order_emission_allowed'])
            self.assertEqual(before,(f.source/'committed.jsonl').read_bytes())
            self.assertEqual(list(f.owner.iterdir()),[])

    def test_preparation_is_not_a_grant_and_never_calls_model(self):
        with mock.patch.object(preflight,'Reader',return_value=self.f.reader),mock.patch.object(preflight.subprocess,'run') as run:
            result=preflight.preflight(self.f.plan_path,self.f.plan_sha,'preparation',self.f.env,now=self.f.now)
        run.assert_not_called()
        self.assertTrue(result['preparation_ready']);self.assertFalse(result['admission_allowed'])
        self.assertNotIn('admission_grant',result)

    def test_v3_cannot_smuggle_a_continuity_binding(self):
        self.f.plan['version']='reaction_shadow_preflight_v3';self.reseal(self.f)
        with self.assertRaisesRegex(ValueError,'unexpected_preflight_schema'):self.check()

    def test_unapproved_binding_or_changed_manifest_is_not_accepted(self):
        self.f.approval['source_continuity_sha256']='0'*64;self.f.save_approval()
        with self.assertRaisesRegex(ValueError,'continuity_not_explicitly_approved'):self.check()
        self.f.manifest.write_text(self.f.manifest.read_text()+' ')
        with self.assertRaisesRegex(ValueError,'pinned_file_changed'):self.check()

    def test_dates_config_and_process_binding_cannot_be_forged(self):
        original=copy.deepcopy(self.f.plan['source_continuity'])
        for change in [dict(source_first_live_session='2026-11-10'),dict(model_first_session='2026-11-10'),
                       dict(source_stream='other'),dict(symbol='IMOEXF'),dict(minimum_ws_generation=True),
                       dict(process=dict(PROCESS,start_ticks=12346)),dict(extra='not-allowed')]:
            self.f.plan['source_continuity']={**copy.deepcopy(original),**change};self.reseal(self.f)
            with self.subTest(change=change),self.assertRaises(ValueError):self.check()

    def test_same_process_new_ws_generation_is_normal_recovery(self):
        guard=service.ClosedSourceGuard(self.f.source,self.f.prepared_raw.encode(),source_continuity=self.f.plan['source_continuity'])
        self.f.health['generation']=3;self.f.write_health()
        self.assertTrue(guard.check(require_ready=True));self.assertEqual(guard.generation,3)
        with mock.patch.object(continuity,'inspect_process',return_value=dict(PROCESS,boot_id='f'*36)):
            with self.assertRaises(service.SourceUnavailable):guard.check(require_ready=True)

    def test_early_redis_row_mutation_is_detected_before_any_grant(self):
        old=self.f.reader.envelope.side_effect
        changed_id=self.f.records[1]['id']
        def mutate(*args,**kw):
            sid,e=old(*args,**kw)
            if sid==changed_id:e['ts_utc']+=1
            return sid,e
        self.f.reader.envelope.side_effect=mutate
        with self.assertRaisesRegex(ValueError,'reader_canonical_content_mismatch'):self.check()
        self.assertEqual(list(self.f.owner.iterdir()),[])

    def test_process_change_during_inspection_blocks_before_grant(self):
        with mock.patch.object(preflight,'Reader',return_value=self.f.reader):
            def changed(*args,**kw):
                self.process.stop()
                self.process=mock.patch.object(continuity,'inspect_process',return_value=dict(PROCESS,start_ticks=99999))
                self.process.start()
                return self.f.completed
            with mock.patch.object(preflight.subprocess,'run',side_effect=changed):
                with self.assertRaises(service.SourceUnavailable):
                    preflight.preflight(self.f.plan_path,self.f.plan_sha,'admission',self.f.env,now=self.f.now)
            self.process.stop()
        self.assertEqual(list(self.f.owner.iterdir()),[])

    def test_grant_and_source_binding_are_durable_before_supervise(self):
        def inspect(*args,**kwargs):
            m=json.loads(self.manifest(self.f).read_text())
            self.assertEqual(m['version'],'reaction_shadow_supervisor_v4')
            self.assertEqual(m['closed_source']['source_continuity'],self.f.plan['source_continuity'])
            self.assertEqual(m['admission_grant']['bootstrap'],self.f.bootstrap)
            self.assertEqual(set(args[4]),{'REACTION_SHADOW_REDIS_URL','TZ'})
            self.assertEqual(args[0][1],'create')
            self.assertTrue(kwargs['require_full_startup_window'])
            return {'fixture_only':True}
        self.start(supervise=inspect)

    def test_clean_preopen_resume_preserves_original_grant_and_does_not_rerun_preflight(self):
        self.clean_stop()
        before=self.manifest(self.f).read_bytes()
        with mock.patch.object(preflight,'preflight',side_effect=AssertionError('resume must not mint another grant')):
            _,run=self.resume()
        self.assertEqual(run.call_args.args[0][1],'resume')
        self.assertEqual(before,self.manifest(self.f).read_bytes())

    def test_failed_previous_attempt_and_midday_resume_are_refused(self):
        self.clean_stop()
        with self.assertRaisesRegex(ValueError,'full_preopen_window'):
            self.resume(when=self.f.epoch('2026-11-09T08:00:00'),ready='2026-11-09T08:30:00+03:00')
        p=self.manifest(self.f).parent/'0001.stopped.json'
        data=json.loads(p.read_text());data['reason']='source_failed';p.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError,'cannot resume'):self.resume()

    def test_resume_cannot_drop_binding_or_change_approved_spec(self):
        self.clean_stop()
        spec=Path(self.f.paths['approved_spec']);spec.write_text(spec.read_text()+'\n')
        with self.assertRaisesRegex(ValueError,'code_spec_or_approval_changed'):self.resume()

    def test_resume_cannot_replace_source_process(self):
        self.clean_stop()
        with mock.patch.object(continuity,'inspect_process',return_value=dict(PROCESS,start_ticks=54321)):
            with self.assertRaises(ValueError):self.resume()


class LinuxIdentityTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform=='linux','actual Linux /proc identity fixture')
    def test_owned_process_pid_start_and_boot_are_all_bound(self):
        process=subprocess.Popen([sys.executable,'-c','import time; time.sleep(20)'])
        try:
            pin=continuity.inspect_process(process.pid)
            continuity.check_process(dict(process=pin))
            with self.assertRaisesRegex(ValueError,'process_or_boot_changed'):
                continuity.check_process(dict(process=dict(pin,start_ticks=pin['start_ticks']+1)))
        finally:
            process.terminate();process.wait(timeout=5)


if __name__=='__main__':unittest.main()
