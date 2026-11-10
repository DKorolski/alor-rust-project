"""Owned native cores + real supervisor + ephemeral Redis; NOT prospective receipts.

Clock hooks live only in ignored Rust tests. No production binary accepts them.
The bridge filters test-harness output, not model inputs or failure responses.
"""
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import selectors
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

import reaction_shadow_service as service

SOURCE_TEST = 'md_source::runner::strict_fixture::source_process'
MODEL_TEST = 'shadow_session_context::collector::runner::tests::strict_fixture::model_process'
MSK = dt.timezone(dt.timedelta(hours=3))


def epoch(hour, minute=0, second=0):
    return int(dt.datetime(2026, 11, 11, hour, minute, second, tzinfo=MSK).timestamp())


def bridge():
    process = subprocess.Popen([os.environ['SHADOW_OWNED_RUNTIME_TEST_BIN'], '--exact', MODEL_TEST,
                                '--ignored', '--nocapture', '--test-threads=1'],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=sys.stderr)
    ending=threading.Event()
    def forward():
        ready=selectors.DefaultSelector(); ready.register(sys.stdin,selectors.EVENT_READ)
        try:
            while not ending.is_set():
                if not ready.select(.1): continue
                chunk=os.read(sys.stdin.fileno(),4096)
                if not chunk: break
                process.stdin.write(chunk); process.stdin.flush()
        except (BrokenPipeError, OSError):
            pass
        finally: ready.close()
    forwarding=threading.Thread(target=forward, daemon=True); forwarding.start()
    for line in process.stdout:
        if b'OWNED_REPLY:' in line:
            sys.stdout.buffer.write(line.split(b'OWNED_REPLY:', 1)[1]); sys.stdout.buffer.flush()
    result=process.wait(); ending.set(); forwarding.join(timeout=1)
    process.stdin.close(); process.stdout.close()
    return result


class SourceProcess:
    def __init__(self, env):
        self.process = subprocess.Popen([env['SHADOW_OWNED_GATEWAY_TEST_BIN'], '--exact', SOURCE_TEST,
            '--ignored', '--nocapture', '--test-threads=1'], env=env, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        self.reader = selectors.DefaultSelector(); self.reader.register(self.process.stdout, selectors.EVENT_READ)
        self.buffer = b''
        assert self.receive()['event'] == 'source_opened'

    def receive(self):
        deadline = time.monotonic()+20
        while time.monotonic() < deadline:
            while b'\n' in self.buffer:
                line, self.buffer = self.buffer.split(b'\n', 1)
                if b'OWNED_REPLY:' in line:
                    return json.loads(line.split(b'OWNED_REPLY:', 1)[1])
            if self.reader.select(.1):
                chunk = os.read(self.process.stdout.fileno(), 65536)
                if not chunk:
                    raise AssertionError('owned source exited: '+self.process.stderr.read().decode()[-3000:])
                self.buffer += chunk
        raise TimeoutError('owned source protocol')

    def command(self, **value):
        self.process.stdin.write(json.dumps(value).encode()+b'\n'); self.process.stdin.flush()
        return self.receive()

    def close(self):
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
        self.process.wait(timeout=5)
        self.reader.close()
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr): stream.close()


@unittest.skipUnless(os.environ.get('SHADOW_OWNED_GATEWAY_TEST_BIN') and os.environ.get('SHADOW_OWNED_RUNTIME_TEST_BIN'),
                     'explicit built Rust test executables and owned Redis required')
class StrictComposed(unittest.TestCase):
    def test_imoexf_full_day_reconnect_terminal_and_paired_eod(self): self.scenario('IMOEXF', 'eod')
    def test_usdrubf_delayed95_reconnect_terminal_and_paired_eod(self): self.scenario('USDRUBF', 'eod')
    def test_imoexf_stop_while_paused(self): self.scenario('IMOEXF', 'stop')
    def test_usdrubf_stop_while_paused(self): self.scenario('USDRUBF', 'stop')
    def test_imoexf_required_bar_deadline_is_not_extended(self): self.scenario('IMOEXF', 'missing')
    def test_usdrubf_required_bar_deadline_is_not_extended(self): self.scenario('USDRUBF', 'missing')

    def scenario(self, symbol, ending):
        with tempfile.TemporaryDirectory(prefix='alor-owned-strict-') as directory:
            root = Path(directory).resolve(); (root/'OWNED_FIXTURE').touch()
            clock = root/'clock'; clock.write_text(str(epoch(6,20)))
            (root/'ready_by').write_text(str(epoch(6,55)))
            with socket.socket() as reservation:
                reservation.bind(('127.0.0.1',0)); port=reservation.getsockname()[1]
            redis_bin=os.environ['REDIS_SERVER_BIN']; cli=os.environ['REDIS_CLI_BIN']
            server=subprocess.Popen([redis_bin,'--bind','127.0.0.1','--port',str(port),'--save','','--appendonly','no','--dir',str(root)],
                                     stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            source=None; thread=None; stop=service.StopRequest(); errors=[]; final=[]; events=[]
            gate_lock=threading.RLock(); changed=threading.Condition()
            opening_after_collect=threading.Event(); disconnect_after_collect=threading.Event()
            def redis(*args):
                return subprocess.check_output([cli,'-h','127.0.0.1','-p',str(port),'--raw',*args],text=True,timeout=5,stderr=subprocess.DEVNULL).strip()
            try:
                for _ in range(100):
                    try:
                        info=redis('INFO','server')
                        self.assertIn(f'process_id:{server.pid}',info); break
                    except subprocess.CalledProcessError:
                        self.assertIsNone(server.poll()); time.sleep(.02)
                else: self.fail('owned Redis startup')
                stream='shadow.source.closed10m.'+symbol+'.owned_strict'
                redis('ACL','SETUSER','owned_reader','on','>fixture-only','~'+stream,'-@all','+ping','+xrange','+multi','+exec')
                cfg=dict(symbol=symbol,stream=stream,bootstrap_start='2026-11-09',first_live_session='2026-11-11',excluded_dates=[],
                    gateway_config='/fixture/gateway.toml',gateway_config_sha256='a'*64,context_finalization=True,
                    ordinary_observation_max_lateness_seconds=300 if symbol=='USDRUBF' else 90,
                    late_context_history_policy='audit_only_v1',terminal=dict(enabled=True,first_live_session='2026-11-09',
                    grace_seconds=10,max_lateness_seconds=90,max_heartbeat_age_seconds=45,max_clock_step_seconds=5,
                    symbols={symbol:dict(weekday_end='23:50:00',weekend_end='19:00:00',overrides={})}))
                (root/'source.json').write_text(json.dumps(cfg))
                template='reaction-shadow.imoexf-combo.toml' if symbol=='IMOEXF' else 'reaction-shadow.usdrubf-h4-w4.toml'
                raw=(Path(__file__).resolve().parents[1]/'configs/examples'/template).read_text()
                raw=raw.replace('2026-11-02','2026-11-11').replace('REPLACE_WITH_APPROVED_'+symbol+'_M10_SOURCE',stream)
                raw=raw.replace('max_sessions = 20','max_sessions = 1')
                (root/'worker.toml').write_text(raw)
                env={**os.environ,'SHADOW_OWNED_FIXTURE':'strict_composed_v1','SHADOW_OWNED_ROOT':str(root),
                     'SHADOW_OWNED_REDIS_URL':f'redis://127.0.0.1:{port}/', 'SHADOW_OWNED_REDIS_PID':str(server.pid),
                     'REACTION_SHADOW_REDIS_URL':f'redis://owned_reader:fixture-only@127.0.0.1:{port}/',
                     'PYTHONDONTWRITEBYTECODE':'1'}
                source=SourceProcess(env)
                source.command(op='history',now=epoch(6,20))
                subprocess.check_output([env['SHADOW_OWNED_RUNTIME_TEST_BIN'],'--exact',
                    MODEL_TEST.rsplit('::',1)[0]+'::bootstrap_manifest','--ignored','--nocapture','--test-threads=1'],
                    env=env,stderr=subprocess.STDOUT,timeout=20)
                pin=json.loads((root/'owned-bootstrap.json').read_text())
                self.assertEqual(pin['rows'],202)
                raw+='\n[runner.bootstrap]\n'+'\n'.join(f'{key} = {json.dumps(value)}' for key,value in pin.items())+'\n'
                (root/'worker.toml').write_text(raw)
                def now(): return int(clock.read_text())
                def emit(line):
                    row=json.loads(line)
                    with changed: events.append(row); changed.notify_all()
                def wait(predicate):
                    deadline=time.monotonic()+20
                    with changed:
                        while not predicate():
                            if errors: raise errors[0]
                            remaining=deadline-time.monotonic()
                            self.assertGreater(remaining,0,'owned model progress timeout')
                            changed.wait(min(remaining,.1))
                def latest():
                    for row in reversed(events):
                        if row['event']=='reaction_shadow_status': return row['status']
                    return None
                def command(op,at,**kwargs):
                    with gate_lock:
                        result=source.command(op=op,now=at,**kwargs)
                        clock.write_text(str(at))
                        return result
                original_channel=service.Channel
                class LockedChannel(original_channel):
                    def send(self, request):
                        gate_lock.acquire(); self.owned_locked=True
                        try: super().send(request)
                        except BaseException:
                            self.owned_locked=False; gate_lock.release(); raise
                    def receive(self,*args):
                        if not getattr(self,'owned_locked',False): gate_lock.acquire()
                        try:
                            row=super().receive(*args)
                            if row.get('event')=='collected':
                                if opening_after_collect.is_set():
                                    opening_after_collect.clear()
                                    source.command(op='bar',now=epoch(7,0,3),label=epoch(7))
                                    clock.write_text(str(epoch(7,0,3)))
                                if disconnect_after_collect.is_set():
                                    disconnect_after_collect.clear()
                                    source.command(op='disconnect',now=epoch(7,15))
                                    clock.write_text(str(epoch(7,15)))
                            return row
                        finally:
                            self.owned_locked=False; gate_lock.release()
                def run_model():
                    attempt=None
                    try:
                        with mock.patch.object(service.time,'time',side_effect=now), mock.patch.object(service,'Channel',LockedChannel):
                            guard=service.ClosedSourceGuard(root/'source',raw.encode())
                            check=guard.check
                            def locked_check(*args,**kwargs):
                                with gate_lock: return check(*args,**kwargs)
                            guard.check=locked_check
                            attempt=service.Attempt(root/'supervisor-owned','create',dict(owned_fixture=True))
                            policy=service.policy_from_spec(raw.encode())
                            # Only fixture pacing is accelerated. Clock/queue/model
                            # budgets remain the exact validated production values.
                            pacing={**policy,'poll_interval_seconds':0}
                            result=service.supervise([sys.executable,'-u',str(Path(__file__).resolve()),'--owned-bridge'],
                                pacing,epoch(6,55),attempt,env,hashlib.sha256(raw.encode()).hexdigest(),stop,emit,source_guard=guard)
                            final.append(result)
                    except BaseException as error:
                        detail=root/'owned-failure.txt'
                        errors.append(AssertionError('owned native: '+detail.read_text()) if detail.exists() else error)
                    finally:
                        if attempt: attempt.close()
                        with changed: changed.notify_all()
                thread=threading.Thread(target=run_model,daemon=True); thread.start()
                wait(lambda: latest() is not None and latest()['armed'])
                opening_after_collect.set()
                wait(lambda: latest()['runner']['pair_ready'])
                command('bar',epoch(7,10,3),label=epoch(7,10))
                wait(lambda: latest()['runner']['observed_sequence']==203)
                before=latest()['runner']['observed_sequence']
                disconnect_after_collect.set()
                wait(lambda: any(row['event']=='reaction_shadow_recovery_paused' for row in events))
                self.assertEqual(latest()['runner']['observed_sequence'],before)
                if ending=='stop':
                    stop.request()
                elif ending=='missing':
                    command('heartbeat',latest()['next_guard_deadline_utc']+1)
                    wait(lambda: bool(errors))
                    self.assertIsInstance(errors[0],service.SourceUnavailable)
                    self.assertEqual(errors[0].code,'REQUIRED_BAR_MISSING')
                else:
                    command('resume',epoch(7,16),label=epoch(7,10))
                    wait(lambda: any(row['event']=='reaction_shadow_recovery_resumed' for row in events))
                    for slot in range(2,101):
                        label=epoch(7)+slot*600
                        at=label+(95 if symbol=='USDRUBF' and slot==71 else 3)
                        price=103. if 36<=slot<40 else 100.
                        command('bar',at,label=label,price=price)
                        wait(lambda: latest()['runner']['observed_sequence']>=202+slot)
                    command('heartbeat',epoch(23,50,10))
                    wait(lambda: bool(final) or bool(errors))
                thread.join(timeout=10); self.assertFalse(thread.is_alive())
                if ending!='missing':
                    self.assertEqual(errors,[])
                    self.assertFalse(final[0]['order_emission_allowed'])
                    if ending=='eod':
                        self.assertTrue(final[0]['completed'])
                        self.assertTrue(final[0]['runner']['session_complete'])
                        self.assertEqual(final[0]['last_complete_session'],'2026-11-11')
                        self.assertEqual(final[0]['runner']['observed_sequence'],303)
                        records=[json.loads(line)['record'] for line in (root/'source/committed.jsonl').read_text().splitlines()]
                        terminal=records[-1]['release']
                        self.assertEqual(terminal['proof']['reason'],'session_boundary_ws')
                        self.assertEqual(terminal['bar']['close_time_utc'],epoch(23,40))
                        if symbol=='USDRUBF':
                            self.assertTrue(any(row.get('release',{}).get('proof',{}).get('reason')=='next_timestamp_delayed' for row in records if row.get('release')))
                self.assertEqual(redis('EXISTS','fixture:commands'),'0')
                output=os.environ.get('SHADOW_OWNED_RESULT_DIR')
                if output:
                    folder=Path(output); folder.mkdir(parents=True,exist_ok=True)
                    (folder/f'{symbol.lower()}-{ending}.json').write_text(json.dumps(dict(
                        evidence='owned_native_composed_fixture_not_prospective',symbol=symbol,ending=ending,
                        events=events,final=final,error_codes=[getattr(e,'code',type(e).__name__) for e in errors],
                        order_emission_allowed=False),indent=2)+'\n')
            finally:
                stop.request()
                if thread: thread.join(timeout=10)
                if source: source.close()
                server.terminate(); server.wait(timeout=5)


if __name__=='__main__':
    if sys.argv[1:]==['--owned-bridge']: sys.exit(bridge())
    unittest.main()
