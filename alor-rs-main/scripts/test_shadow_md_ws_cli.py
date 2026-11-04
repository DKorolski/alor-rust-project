"""Owned WS/Redis component smoke. No third-party Python packages or broker."""
import base64
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import threading
import time
import unittest

import reaction_shadow_service as supervisor

MSK = dt.timezone(dt.timedelta(hours=3))

def exact(stream, n):
    data = stream.read(n)
    if len(data) != n:
        raise EOFError()
    return data

def resp(stream, depth=0):
    if depth > 8:
        raise ValueError('RESP depth')
    line = stream.readline(65536)
    if not line.endswith(b'\r\n'):
        raise EOFError()
    raw = line
    kind, value = line[:1], line[1:-2]
    if kind == b'$':
        n = int(value)
        if n == -1:
            return raw, None
        if not 0 <= n <= 2 * 1024 * 1024:
            raise ValueError('RESP bulk budget')
        body = exact(stream, n + 2)
        return raw + body, body[:-2]
    if kind == b'*':
        n = int(value)
        if not 0 <= n <= 10000:
            raise ValueError('RESP array budget')
        values = []
        for _ in range(n):
            item, parsed = resp(stream, depth + 1)
            raw += item
            values.append(parsed)
        return raw, values
    if kind in (b'+', b'-', b':'):
        return raw, value
    raise ValueError('unknown RESP marker')

class OwnedProxy:
    def __init__(self, backend, drop_ack):
        self.backend, self.drop_ack = backend, drop_ack
        self.xadds = 0
        self.lost = 0
        self.stop = threading.Event()
        self.sock = socket.socket()
        self.sock.bind(('127.0.0.1', 0)); self.sock.listen(); self.sock.settimeout(.2)
        self.port = self.sock.getsockname()[1]
        self.thread = threading.Thread(target=self.accept, daemon=True)
        self.thread.start()

    def accept(self):
        while not self.stop.is_set():
            try:
                conn, _ = self.sock.accept()
            except (TimeoutError, socket.timeout):
                continue
            except OSError:
                return
            threading.Thread(target=self.serve, args=(conn,), daemon=True).start()

    def serve(self, conn):
        try:
            with conn, socket.create_connection(('127.0.0.1', self.backend), timeout=5) as remote:
                with conn.makefile('rb') as incoming, remote.makefile('rb') as replies:
                    while True:
                        raw, command = resp(incoming)
                        remote.sendall(raw)
                        response, value = resp(replies)
                        if command[0].upper() == b'XADD':
                            self.xadds += 1
                            if self.drop_ack and self.lost == 0 and response.startswith(b'$'):
                                self.lost += 1
                                response = b'-ERR owned_fixture_uncertain_ack\r\n'
                        conn.sendall(response)
        except (EOFError, OSError):
            pass

    def close(self):
        self.stop.set(); self.sock.close(); self.thread.join(timeout=2)

class OwnedWs:
    def __init__(self, bars):
        self.bars = bars
        self.opcodes, self.errors = [], []
        self.connections = []
        self.connection_lock = threading.Lock()
        self.stop = threading.Event()
        self.sock = socket.socket()
        self.sock.bind(('127.0.0.1', 0)); self.sock.listen(); self.sock.settimeout(.2)
        self.port = self.sock.getsockname()[1]
        self.thread = threading.Thread(target=self.accept, daemon=True)
        self.thread.start()

    def accept(self):
        while not self.stop.is_set():
            try:
                conn, _ = self.sock.accept()
            except (TimeoutError, socket.timeout):
                continue
            except OSError:
                return
            threading.Thread(target=self.serve, args=(conn,), daemon=True).start()

    @staticmethod
    def send(conn, opcode, payload):
        header = bytes([0x80 | opcode])
        n = len(payload)
        if n < 126:
            header += bytes([n])
        elif n < 65536:
            header += b'\x7e' + struct.pack('!H', n)
        else:
            header += b'\x7f' + struct.pack('!Q', n)
        conn.sendall(header + payload)

    def serve(self, conn):
        with self.connection_lock:
            self.connections.append(conn)
        try:
            with conn, conn.makefile('rb') as stream:
                request = b''
                while not request.endswith(b'\r\n\r\n'):
                    request += exact(stream, 1)
                    if len(request) > 16384:
                        raise ValueError('handshake budget')
                headers = dict(s.split(':', 1) for s in request.decode().split('\r\n')[1:] if ':' in s)
                key = next(v.strip() for k, v in headers.items() if k.lower() == 'sec-websocket-key')
                accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest())
                conn.sendall(b'HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + b'\r\n\r\n')
                while True:
                    first, second = exact(stream, 2)
                    opcode, length = first & 15, second & 127
                    if length == 126:
                        length = struct.unpack('!H', exact(stream, 2))[0]
                    elif length == 127:
                        length = struct.unpack('!Q', exact(stream, 8))[0]
                    if length > 2 * 1024 * 1024:
                        raise ValueError('WS payload budget')
                    mask = exact(stream, 4) if second & 128 else b''
                    payload = exact(stream, length)
                    if mask:
                        payload = bytes(v ^ mask[i % 4] for i, v in enumerate(payload))
                    if opcode == 8:
                        self.send(conn, 8, payload); return
                    if opcode == 9:
                        self.send(conn, 10, payload); continue
                    if opcode != 1:
                        continue
                    message = json.loads(payload)
                    name = message['opcode']; self.opcodes.append(name)
                    if name == 'unsubscribe':
                        continue
                    if name != 'BarsGetAndSubscribe' or 'portfolio' in message:
                        raise ValueError('forbidden account/order opcode')
                    guid = message['guid']
                    # Observed native order: historical bars precede ACK 200.
                    for bar in self.bars():
                        self.send(conn, 1, json.dumps(dict(guid=guid, data=bar)).encode())
                    self.send(conn, 1, json.dumps(dict(requestGuid=guid, httpCode=200)).encode())
        except (EOFError, OSError):
            pass
        except Exception as error:
            self.errors.append(type(error).__name__ + ':' + str(error))
        finally:
            with self.connection_lock:
                self.connections.remove(conn)

    def force_protocol_reset(self):
        with self.connection_lock:
            if len(self.connections) != 1:
                raise AssertionError('expected one owned source socket')
            self.send(self.connections[0], 3, b'owned-invalid-opcode')

    def close(self):
        self.stop.set(); self.sock.close(); self.thread.join(timeout=2)

@unittest.skipUnless(os.environ.get('SHADOW_MD_WS_BIN'), 'set built SHADOW_MD_WS_BIN for owned smoke')
class WsSourceSmoke(unittest.TestCase):
    def test_create_stop_resume_and_tampered_prefix(self):
        self.smoke(False, tamper=True)

    def test_uncertain_xadd_ack_reconciles_without_duplicate_append(self):
        self.smoke(True, tamper=False)

    def test_usdrubf_sparse_history_bootstrap_and_resume(self):
        self.smoke(False, tamper=True, symbol='USDRUBF')

    def test_usdrubf_uncertain_ack_and_reconnect(self):
        self.smoke(True, tamper=False, symbol='USDRUBF')

    def smoke(self, drop_ack, tamper, symbol='IMOEXF'):
        binary = Path(os.environ['SHADOW_MD_WS_BIN']).resolve(strict=True)
        server_bin = os.environ.get('REDIS_SERVER_BIN') or shutil.which('redis-server')
        cli_bin = os.environ.get('REDIS_CLI_BIN') or shutil.which('redis-cli')
        self.assertTrue(server_bin and cli_bin, 'owned Redis tools required')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            with socket.socket() as reserve:
                reserve.bind(('127.0.0.1', 0)); port = reserve.getsockname()[1]
            server = subprocess.Popen([server_bin, '--bind', '127.0.0.1', '--port', str(port), '--save', '',
                '--appendonly', 'no', '--dir', str(root)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            def redis(*args, reader=False, source=False):
                user = 'reader' if reader else 'source' if source else None
                env = {**os.environ, 'REDISCLI_AUTH':'fixture'} if user else os.environ
                cmd = [cli_bin, '-h', '127.0.0.1', '-p', str(port), *(['--user',user] if user else []), '--raw', *args]
                return subprocess.check_output(cmd, env=env, text=True, timeout=5, stderr=subprocess.DEVNULL).strip()
            processes = []
            logs = []
            try:
                for _ in range(100):
                    self.assertIsNone(server.poll())
                    try:
                        pid = next(int(s.split(':')[1]) for s in redis('INFO','server').splitlines() if s.startswith('process_id:'))
                        self.assertEqual(pid, server.pid, 'refusing unrelated Redis process'); break
                    except (subprocess.CalledProcessError, StopIteration): time.sleep(.02)
                else: self.fail('owned Redis startup timeout')
                today = dt.datetime.now(MSK).date()
                first = today + dt.timedelta(days=1)
                while first.weekday() >= 5: first += dt.timedelta(days=1)
                excluded = [today + dt.timedelta(days=n) for n in range((first-today).days)]
                prior = []; day = today - dt.timedelta(days=1)
                while len(prior) < 2:
                    if day.weekday() < 5: prior.append(day)
                    day -= dt.timedelta(days=1)
                begin = min(prior)
                if symbol == 'USDRUBF':
                    # Keep the named sparse-weekend test meaningful on any weekday.
                    sunday = today - dt.timedelta(days=1)
                    while sunday.weekday() != 6:
                        sunday -= dt.timedelta(days=1)
                    begin = min(begin, sunday - dt.timedelta(days=2))
                source_key = f'shadow.source.closed10m.{symbol}.owned_fixture'
                redis('ACL','SETUSER','source','on','>fixture','~'+source_key,'-@all','+xadd','+xrange','+xlen','+ping')
                redis('ACL','SETUSER','reader','on','>fixture','~'+source_key,'-@all','+xrange','+multi','+exec','+ping')
                self.assertIn('NOPERM',redis('XADD','cmd.orders.fixture','*','payload','forbidden',reader=True))
                self.assertIn('NOPERM',redis('XADD','cmd.orders.fixture','*','payload','forbidden',source=True))
                self.assertIn('NOPERM',redis('XRANGE','md.bars.live.IMOEXF','-','+',source=True))
                def bars():
                    now = int(time.time()); result=[]; day=begin
                    while day <= today:
                        if day in excluded: day+=dt.timedelta(days=1); continue
                        start = dt.time(9,50) if day.weekday()>=5 else dt.time(7)
                        total = 55 if day.weekday()>=5 else 101
                        base = int(dt.datetime.combine(day,start,MSK).timestamp())
                        for slot in range(total):
                            if symbol=='USDRUBF' and day.weekday()>=5 and slot in (8,15,27,40):
                                continue  # Real history may have absent non-tradable context slots.
                            label=base+slot*600
                            if label<=now:
                                result.append(dict(time=label,open=100.,high=102.,low=98.,close=101.,volume=1.))
                        day+=dt.timedelta(days=1)
                    return result
                ws=OwnedWs(bars); proxy=OwnedProxy(port,drop_ack)
                self.addCleanup(ws.close); self.addCleanup(proxy.close)
                gateway=root/'gateway.toml'
                from_ts=int(dt.datetime.combine(begin,dt.time(7),MSK).timestamp())
                gateway.write_text(f'''portfolio = "__market_data_only__"
refresh_token = "fixture-not-a-real-token"
symbols = ["{symbol}"]
tf_sec = 600
from_ts = {from_ts}
ws_url = "ws://127.0.0.1:{ws.port}"
oauth_url = "http://127.0.0.1:1/unused-oauth"
skip_history_bars = false
''')
                cfg=dict(symbol=symbol,stream=source_key,bootstrap_start=str(begin),first_live_session=str(first),context_finalization=True,ordinary_observation_max_lateness_seconds=300 if symbol=='USDRUBF' else 90,
                    excluded_dates=[str(d) for d in excluded],gateway_config=str(gateway),
                    gateway_config_sha256=hashlib.sha256(gateway.read_bytes()).hexdigest(),terminal=dict(
                        enabled=True,first_live_session=str(today),grace_seconds=10,max_lateness_seconds=90,
                        max_heartbeat_age_seconds=45,max_clock_step_seconds=5,symbols={symbol:dict(
                            weekday_end='23:50:00',weekend_end='19:00:00',overrides={str(d):None for d in excluded})}))
                config=root/'source.json';config.write_text(json.dumps(cfg))
                state=root/'source'
                env={'PATH':os.environ.get('PATH','/usr/bin:/bin'),'SHADOW_SOURCE_REDIS_URL':f'redis://source:fixture@127.0.0.1:{proxy.port}/0',
                     'SHADOW_FIXTURE_ACCESS_TOKEN':'fixture-access-token','TZ':'UTC'}
                def start(mode, ready=True):
                    log=(root/f'{len(processes)}.log').open('w+')
                    logs.append(log)
                    p=subprocess.Popen([str(binary),mode,str(config),str(state),'--fixture'],env=env,
                        stdout=log,stderr=subprocess.STDOUT,umask=0o027)
                    processes.append(p)
                    if ready:
                        for _ in range(800):
                            if p.poll() is not None:
                                log.flush();self.fail((root/f'{len(processes)-1}.log').read_text())
                            hp=state/'HEALTH.json'
                            if hp.exists() and json.loads(hp.read_text()).get('status')=='ready':break
                            time.sleep(.025)
                        else:self.fail('source readiness timeout')
                    return p
                p=start('create')
                before=redis('XRANGE',source_key,'-','+')
                count=int(redis('XLEN',source_key));self.assertGreaterEqual(count,202)
                self.assertEqual(proxy.xadds,count)
                self.assertEqual(proxy.lost,int(drop_ack))
                old_generation=json.loads((state/'HEALTH.json').read_text())['generation']
                ws.force_protocol_reset()
                for _ in range(400):
                    self.assertIsNone(p.poll(), 'source stopped during owned reconnect')
                    health=json.loads((state/'HEALTH.json').read_text())
                    if health['status']=='ready' and health['generation']>old_generation:
                        break
                    time.sleep(.025)
                else:self.fail('owned WS reconnect readiness timeout')
                self.assertEqual(redis('XRANGE',source_key,'-','+'),before)
                self.assertEqual(proxy.xadds,count)
                template_name='reaction-shadow.imoexf-combo.toml' if symbol=='IMOEXF' else 'reaction-shadow.usdrubf-h4-w4.toml'
                template=(Path(__file__).resolve().parents[1]/'configs/examples'/template_name).read_text()
                raw=template.replace('"2026-11-02"',json.dumps(str(first))).replace(f'REPLACE_WITH_APPROVED_{symbol}_M10_SOURCE',source_key)
                raw=raw.replace('excluded_dates = []','excluded_dates = '+json.dumps(cfg['excluded_dates']))
                guard=supervisor.ClosedSourceGuard(state,raw.encode());guard.check()
                worker=os.environ.get('REACTION_WORKER_BIN')
                if worker:
                    spec=root/'worker.toml';spec.write_text(raw)
                    first_id=str(from_ts*1000)+'-0'
                    last_label=int(dt.datetime.combine(max(prior),dt.time(23,40),MSK).timestamp())
                    last_id=str(last_label*1000)+'-0'
                    reader_env={'REACTION_SHADOW_REDIS_URL':f'redis://reader:fixture@127.0.0.1:{port}/0','TZ':'UTC'}
                    preview=json.loads(subprocess.check_output([worker,'inspect-bootstrap',str(spec),first_id,last_id],env=reader_env,text=True,timeout=20))['preview']
                    self.assertTrue(all(len(arm)>=2 for arm in preview['anchors']))
                    raw+='\n[runner.bootstrap]\n'+''.join(f'{k} = {json.dumps(v)}\n' for k,v in preview['bootstrap'].items())
                    spec.write_text(raw)
                    generation=root/'worker'
                    attempt=supervisor.Attempt(generation,'create',dict(binary_sha256=supervisor.file_hash(worker),spec_sha256=hashlib.sha256(raw.encode()).hexdigest(),closed_source=guard.identity))
                    stop=supervisor.StopRequest()
                    def report(line):
                        event=json.loads(line)
                        if event['event']=='reaction_shadow_status' and event['status']['armed']:stop.request()
                    try:
                        status=supervisor.supervise([worker,'create',str(spec),str(generation),str(int(time.time())+60)],
                            supervisor.policy_from_spec(raw.encode()),time.time()+60,attempt,reader_env,
                            hashlib.sha256(raw.encode()).hexdigest(),stop,report,source_guard=guard)
                    finally:attempt.close()
                    self.assertFalse(status['order_emission_allowed'])
                    self.assertEqual(status['runner']['bootstrap_status'],'loaded')
                p.send_signal(signal.SIGTERM);self.assertEqual(p.wait(timeout=8),0)
                with self.assertRaises(supervisor.SourceUnavailable):guard.check()
                p=start('resume')
                self.assertEqual(redis('XRANGE',source_key,'-','+'),before)
                p.send_signal(signal.SIGTERM);self.assertEqual(p.wait(timeout=8),0)
                self.assertEqual(proxy.xadds,count)
                self.assertEqual(redis('EXISTS','cmd.orders.fixture'),'0')
                self.assertFalse(ws.errors,ws.errors)
                self.assertTrue(set(ws.opcodes)<= {'BarsGetAndSubscribe','unsubscribe'})
                if tamper:
                    records=redis('XREVRANGE',source_key,'+','-','COUNT','1').splitlines()
                    redis('XDEL',source_key,records[0])
                    p=start('resume',ready=False);self.assertNotEqual(p.wait(timeout=8),0)
                    self.assertTrue((state/'FAILED.json').exists())
            finally:
                for p in processes:
                    if p.poll() is None:
                        p.terminate()
                        try:p.wait(timeout=5)
                        except subprocess.TimeoutExpired:p.kill();p.wait()
                for log in logs:log.close()
                server.terminate();server.wait(timeout=5)

if __name__=='__main__':unittest.main()
