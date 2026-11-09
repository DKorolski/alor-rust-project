"""Owned worker protocol/Redis smoke, not OS-role or admission-grant qualification."""
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import unittest

import reaction_shadow_service as service

MSK=dt.timezone(dt.timedelta(hours=3))


@unittest.skipUnless(os.environ.get('REACTION_WORKER_BIN'), 'set built worker for explicit owned late-start smoke')
class LateWorkerSmoke(unittest.TestCase):
    def test_real_imoexf_late_worker_keeps_diagnostic_day_out_of_clean_count(self):
        self.smoke('IMOEXF')

    def test_real_usdrubf_late_worker_keeps_diagnostic_day_out_of_clean_count(self):
        self.smoke('USDRUBF')

    def test_changed_recovery_hash_fails_without_flat_fallback(self):
        self.smoke('USDRUBF',bad_digest=True)

    def smoke(self,symbol,*,bad_digest=False):
        now=dt.datetime.now(MSK)
        if now.weekday()>=5 or not dt.time(7,11)<=now.time()<dt.time(23,20):
            self.skipTest('real-clock diagnostic smoke requires a regular session, no fake clock')
        worker=Path(os.environ['REACTION_WORKER_BIN']).resolve(strict=True)
        server_bin=os.environ.get('REDIS_SERVER_BIN') or shutil.which('redis-server')
        cli=os.environ.get('REDIS_CLI_BIN') or shutil.which('redis-cli')
        self.assertTrue(server_bin and cli)
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory).resolve()
            with socket.socket() as reserved:
                reserved.bind(('127.0.0.1',0));port=reserved.getsockname()[1]
            server=subprocess.Popen([server_bin,'--bind','127.0.0.1','--port',str(port),
                                     '--save','','--appendonly','no','--dir',str(root)],
                                    stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            def redis(*args,reader=False):
                return subprocess.check_output([cli,'-h','127.0.0.1','-p',str(port),'--raw',
                    *(['--user','shadow_ws_reader_fixture'] if reader else []),*args],text=True,timeout=4,
                    env={**os.environ,**({'REDISCLI_AUTH':'fixture-only'} if reader else {})},stderr=subprocess.DEVNULL).strip()
            try:
                for _ in range(100):
                    try:
                        info=redis('INFO','server')
                        pid=next(int(line.split(':')[1]) for line in info.splitlines() if line.startswith('process_id:'))
                        self.assertEqual(pid,server.pid);break
                    except subprocess.CalledProcessError:
                        self.assertIsNone(server.poll());time.sleep(.02)
                else:self.fail('owned Redis startup timeout')
                stream='fixture:late:bars'
                self.assertEqual(redis('ACL','SETUSER','shadow_ws_reader_fixture','on','>fixture-only','~'+stream,
                                       '-@all','+ping','+xrange','+multi','+exec'),'OK')
                self.assertIn('NOPERM',redis('XADD',stream,'*','payload','denied',reader=True))
                today=now.date(); first_clean=today+dt.timedelta(days=1)
                while first_clean.weekday()>=5:first_clean+=dt.timedelta(days=1)
                prior=[];day=today-dt.timedelta(days=1)
                while len(prior)<2:
                    if day.weekday()<5:prior.append(day)
                    day-=dt.timedelta(days=1)
                labels=[]
                for day in sorted(prior):
                    start=int(dt.datetime.combine(day,dt.time(7),MSK).timestamp())
                    labels.extend(start+600*slot for slot in range(101))
                start=int(dt.datetime.combine(today,dt.time(7),MSK).timestamp())
                last=int(time.time())//600*600-600
                labels.extend(range(start,last+1,600))
                for label in labels:
                    payload=dict(symbol=symbol,close_time_utc=label,o=100.,h=102.,l=98.,c=100.,v=1.,origin='history')
                    envelope=dict(schema_version=1,ts_utc=label+610,source='owned-late-fixture',msg_type='bar',payload=payload)
                    self.assertEqual(redis('XADD',stream,str(label*1000)+'-0','payload',json.dumps(envelope)),str(label*1000)+'-0')
                template='reaction-shadow.imoexf-combo.toml' if symbol=='IMOEXF' else 'reaction-shadow.usdrubf-h4-w4.toml'
                raw=(Path(__file__).resolve().parents[1]/'configs/examples'/template).read_text()
                raw=raw.replace('2026-11-02',today.isoformat()).replace('REPLACE_WITH_APPROVED_'+symbol+'_M10_SOURCE',stream)
                spec=root/'launch.toml';spec.write_text(raw)
                env=dict(REACTION_SHADOW_REDIS_URL=f'redis://shadow_ws_reader_fixture:fixture-only@127.0.0.1:{port}/0',TZ='UTC')
                first_id=str(labels[0]*1000)+'-0';last_id=str(labels[-1]*1000)+'-0'
                inspected=subprocess.run([str(worker),'inspect-late-start',str(spec),first_id,last_id],
                    capture_output=True,text=True,timeout=20,env=env)
                self.assertEqual(inspected.returncode,0,inspected.stdout)
                preview=json.loads(inspected.stdout)['preview']
                self.assertEqual(preview['bootstrap']['rows'],202)
                self.assertEqual(preview['recovery']['rows'],len(labels))
                self.assertTrue(all(len(arm)==2 for arm in preview['anchors']))
                self.assertFalse(preview['order_emission_allowed'])
                raw+='\n[runner.bootstrap]\n'+'\n'.join(f'{k} = {json.dumps(v)}' for k,v in preview['bootstrap'].items())
                raw+='\n[runner.late_start]\nversion="diagnostic_late_start_v1"\n'
                raw+=f'source_first_live_session="{first_clean}"\nfirst_clean_session="{first_clean}"\n'
                recovery=dict(preview['recovery'])
                if bad_digest:recovery['source_sha256']='0'*64
                raw+='\n[runner.late_start.recovery]\n'+'\n'.join(f'{k} = {json.dumps(v)}' for k,v in recovery.items())+'\n'
                spec.write_text(raw)
                policy=service.policy_from_spec(raw.encode()); ready=int(time.time())+policy['startup_timeout_seconds']+60
                generation=root/'generation';spec_sha=hashlib.sha256(raw.encode()).hexdigest()
                manifest=dict(version='owned_late_worker_protocol_fixture',generation=str(generation),
                              binary_sha256=service.file_hash(worker),spec_sha256=spec_sha)
                attempt=service.Attempt(generation,'create',manifest);stop=service.StopRequest()
                def emit(line):
                    event=json.loads(line)
                    if event['event']=='reaction_shadow_status' and event['status']['armed']:stop.request()
                try:
                    if bad_digest:
                        with self.assertRaises(service.WorkerFailure):
                            service.supervise([str(worker),'create',str(spec),str(generation),str(ready)],
                                              policy,ready,attempt,env,spec_sha,stop,emit)
                    else:
                        final=service.supervise([str(worker),'create',str(spec),str(generation),str(ready)],
                                                policy,ready,attempt,env,spec_sha,stop,emit)
                        self.assertTrue(final['armed'] and final['runner']['pair_ready'])
                        self.assertEqual(final['runner']['durable_receipts'],len(labels))
                        self.assertEqual(final['started_sessions'],0)
                        self.assertEqual(final['clean_sessions_completed'],0)
                        self.assertFalse(final['diagnostic_late_start']['current_session_clean_eligible'])
                        self.assertFalse(final['order_emission_allowed'])
                finally:attempt.close()
                if bad_digest:
                    with self.assertRaisesRegex(ValueError,'incomplete/failed'):
                        service.Attempt(generation,'resume',manifest)
                self.assertEqual(redis('XLEN',stream),str(len(labels)))
                self.assertEqual(redis('XINFO','GROUPS',stream),'')
                self.assertEqual(redis('EXISTS','fixture:commands'),'0')
            finally:
                server.terminate();server.wait(timeout=5)


if __name__=='__main__':unittest.main()
