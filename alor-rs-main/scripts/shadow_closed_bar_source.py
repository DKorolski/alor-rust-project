#!/usr/bin/env python3
"""Isolated, authenticated REST closed-M10 source. Never a broker command client.

The HTTP endpoint is called history, but only freshly observed, closed bars can
be Live. Historical/context import is explicitly History. No existing gateway
stream is read, repaired, relabelled or trimmed by this process.
"""
import argparse
import contextlib
import datetime as dt
import email.utils
import fcntl
import hashlib
import ipaddress
import json
import math
import os
from pathlib import Path
import signal
import socket
import time
import urllib.error
import urllib.parse
import urllib.request

MSK = dt.timezone(dt.timedelta(hours=3))
TF = 600
GRACE = 5
CONFIRM = 5
MAX_AGE = 90  # Leave 30 seconds for the existing 120-second consumer deadline.
MAX_ROWS = 4500
VERSION = "shadow_authenticated_rest_closed_m10_v2"


class SourceFault(Exception):
    pass


def require(ok, message):
    if not ok:
        raise SourceFault(message)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)


def local(ts):
    return dt.datetime.fromtimestamp(ts, MSK)


def label(day, hour, minute=0):
    return int(dt.datetime.combine(day, dt.time(hour, minute), MSK).timestamp())


def slots(day, excluded):
    if context_only_day(day, excluded):
        return range(label(day, 9, 50), label(day, 19), TF)
    return range(label(day, 7), label(day, 23, 50), TF)


def context_only_day(day, excluded):
    return day.weekday() >= 5 or day.isoformat() in excluded


def context_gap(ts, first, observed, excluded):
    require(context_only_day(local(ts).date(), excluded), "regular candle cannot be skipped")
    require(ts in slots(local(ts).date(), excluded), "invalid context gap label")
    if first < ts + TF + MAX_AGE or observed - first < CONFIRM:
        return None
    return dict(bar_ts_utc=ts, first_absent_at_utc=first, confirmed_absent_at_utc=observed,
                reason="no_bar_in_two_authenticated_responses", method=VERSION)


def check_bar(raw):
    require(isinstance(raw, dict), "invalid REST bar")
    ts = raw.get("time")
    require(type(ts) is int and ts > 0 and ts % TF == 0, "invalid M10 timestamp")
    out = {"time": ts}
    for key in ("open", "high", "low", "close", "volume"):
        v = raw.get(key)
        require(type(v) in (float, int) and math.isfinite(v), "invalid OHLCV number")
        out[key] = float(v)
    require(0 < out["low"] <= min(out["open"], out["close"]) <=
            max(out["open"], out["close"]) <= out["high"] and out["volume"] >= 0,
            "invalid OHLCV geometry")
    return out


def parse_rows(body, start, end):
    require(isinstance(body, dict) and isinstance(body.get("history"), list), "invalid REST history response")
    require(len(body["history"]) <= 1500, "REST response row budget exceeded")
    result = {}
    for raw in body["history"]:
        row = check_bar(raw)
        ts = row["time"]
        if start <= ts <= end:
            require(ts not in result, "duplicate REST timestamp")
            result[ts] = row
    return result


class Finalizer:
    def __init__(self, first_live_session, excluded=()):
        self.first_live_session = first_live_session
        self.excluded = excluded
        self.candidates = {}

    def observe(self, row, observed):
        ts = row["time"]
        age = observed - ts - TF
        require(age >= GRACE, "bar not closed with grace")
        origin = "history" if (local(ts).date() < self.first_live_session or
                                context_only_day(local(ts).date(), self.excluded)) else "live"
        if origin == "live":
            require(age <= MAX_AGE and local(ts).date() == local(observed).date(),
                    "fresh closed bar deadline missed; never relabel as Live")
        fingerprint = hashlib.sha256(canonical(row).encode()).hexdigest()
        candidate = self.candidates.get(ts)
        if candidate is None or candidate[0] != fingerprint:
            self.candidates[ts] = (fingerprint, observed)
            return None
        first = candidate[1]
        if observed - first < CONFIRM:
            return None
        return dict(origin=origin, method=VERSION, first_confirmed_at_utc=first,
                    confirmed_at_utc=observed, closed_at_utc=ts + TF,
                    candle_sha256=fingerprint)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise SourceFault("HTTP redirects prohibited")


class MarketData:
    def __init__(self):
        self.refresh = os.environ.get("SHADOW_ALOR_REFRESH_TOKEN", "").strip().strip('"')
        require(bool(self.refresh), "missing shadow market-data credential")
        self.token, self.expires = None, 0
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())

    def request(self, url, headers=None, method="GET"):
        request = urllib.request.Request(url, headers=headers or {}, method=method)
        try:
            with self.http.open(request, timeout=8) as response:
                raw = response.read(2 * 1024 * 1024 + 1)
                require(len(raw) <= 2 * 1024 * 1024, "HTTP response budget exceeded")
                server = email.utils.parsedate_to_datetime(response.headers["Date"]).timestamp()
                require(abs(time.time() - server) <= 5, "HTTP/server clock skew")
                return json.loads(raw)
        except urllib.error.HTTPError as exc:
            # Never expose URLs, credential query strings or response bodies.
            raise SourceFault("market-data HTTP status %d" % exc.code) from None
        except SourceFault:
            raise
        except Exception:
            raise SourceFault("market-data transport/decode failure") from None

    def bars(self, symbol, start, end):
        if time.monotonic() >= self.expires:
            response = self.request("https://oauth.alor.ru/refresh?" +
                                    urllib.parse.urlencode({"token": self.refresh}), method="POST")
            self.token = response.get("AccessToken")
            require(isinstance(self.token, str) and self.token, "missing access token")
            self.expires = time.monotonic() + 600
        query = urllib.parse.urlencode(dict(symbol=symbol, exchange="MOEX", tf=TF,
                                            format="Simple", untraded="false",
                                            **{"from": start, "to": end + TF - 1}))
        body = self.request("https://api.alor.ru/md/v2/history?" + query,
                            {"Authorization": "Bearer " + self.token})
        return parse_rows(body, start, end)


class Redis:
    """Small bounded RESP client; own stream only, no trim/group/command keys."""
    def __init__(self):
        url = urllib.parse.urlparse(os.environ.get("SHADOW_SOURCE_REDIS_URL", ""))
        require(url.scheme == "redis" and url.hostname and url.username and url.password,
                "dedicated Redis ACL URL required")
        require(ipaddress.ip_address(url.hostname).is_private and url.path in ("", "/0"),
                "private Redis database 0 required")
        self.sock = socket.create_connection((url.hostname, url.port or 6379), timeout=5)
        self.file = self.sock.makefile("rb")
        self.command("AUTH", urllib.parse.unquote(url.username), urllib.parse.unquote(url.password))

    def close(self):
        self.file.close()
        self.sock.close()

    def command(self, *args):
        parts = [str(x).encode() for x in args]
        self.sock.sendall(b"*%d\r\n" % len(parts) + b"".join(
            b"$%d\r\n" % len(x) + x + b"\r\n" for x in parts))
        return self.read()

    def read(self, depth=0):
        require(depth < 8, "Redis response nesting budget")
        line = self.file.readline(65538)
        require(line.endswith(b"\r\n") and len(line) <= 65536, "invalid Redis response")
        tag, value = line[:1], line[1:-2]
        if tag == b"+":
            return value.decode()
        if tag == b"-":
            raise SourceFault("Redis command refused")
        if tag == b":":
            return int(value)
        if tag == b"$":
            size = int(value)
            if size == -1:
                return None
            require(0 <= size <= 65536, "Redis bulk budget")
            raw = self.file.read(size + 2)
            require(len(raw) == size + 2 and raw[-2:] == b"\r\n", "truncated Redis response")
            return raw[:-2].decode()
        if tag == b"*":
            size = int(value)
            require(0 <= size <= MAX_ROWS * 2, "Redis array budget")
            return [self.read(depth + 1) for _ in range(size)]
        raise SourceFault("invalid Redis protocol")


def envelope(symbol, row, proof):
    return dict(schema_version=1, ts_utc=proof["confirmed_at_utc"], source=VERSION,
                msg_type="bar", payload=dict(symbol=symbol, close_time_utc=row["time"],
                    o=row["open"], h=row["high"], l=row["low"], c=row["close"],
                    v=row["volume"], origin=proof["origin"], source_proof=proof))


def request_start(first, published):
    previous = sorted(published)[-2:]
    return min([first - 2 * TF] + previous)


def stored_row(entry, symbol):
    stream_id, fields = entry
    require(len(fields) == 2 and fields[0] == "payload", "unexpected source stream fields")
    env = json.loads(fields[1])
    require(env["source"] == VERSION and env["msg_type"] == "bar" and env["schema_version"] == 1,
            "source identity mismatch")
    bar = env["payload"]
    require(bar["symbol"] == symbol, "source symbol mismatch")
    row = check_bar(dict(time=bar["close_time_utc"], open=bar["o"], high=bar["h"],
                         low=bar["l"], close=bar["c"], volume=bar["v"]))
    require(stream_id == "%d-0" % (row["time"] * 1000), "source ID mismatch")
    proof = bar["source_proof"]
    require(proof["method"] == VERSION and proof["origin"] == bar["origin"] and
            proof["candle_sha256"] == hashlib.sha256(canonical(row).encode()).hexdigest() and
            env["ts_utc"] == proof["confirmed_at_utc"], "source proof mismatch")
    require(proof["closed_at_utc"] == row["time"] + TF and
            proof["first_confirmed_at_utc"] >= row["time"] + TF + GRACE,
            "source closure proof mismatch")
    age = proof["confirmed_at_utc"] - row["time"] - TF
    require(age >= GRACE and (bar["origin"] == "history" or
            bar["origin"] == "live" and age <= MAX_AGE), "invalid source finality age")
    require(proof["confirmed_at_utc"] - proof["first_confirmed_at_utc"] >= CONFIRM,
            "invalid source confirmation")
    return row


def validate_config(config):
    require(set(config) == {"symbol", "stream", "bootstrap_start", "first_live_session", "excluded_dates"},
            "unexpected source config fields")
    require(config["symbol"] in ("IMOEXF", "USDRUBF"), "source symbol not approved")
    require(config["stream"].startswith("shadow.source.closed10m." + config["symbol"] + ".") and
            len(config["stream"]) < 180, "isolated source stream required")
    start = dt.date.fromisoformat(config["bootstrap_start"])
    live = dt.date.fromisoformat(config["first_live_session"])
    require(2 <= (live - start).days <= 7, "bounded bootstrap interval required")
    require(live.weekday() < 5 and live.isoformat() not in config["excluded_dates"], "invalid activation date")
    for day in config["excluded_dates"]:
        dt.date.fromisoformat(day)
    return start, live


def log(action, **fields):
    print(canonical(dict(action=action, observed_at_utc=int(time.time()), **fields)), flush=True)


def run(config, root):
    validate_config(config)
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    with contextlib.ExitStack() as resources:
        lock = resources.enter_context((root / "writer.lock").open("a"))
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return run_locked(config, root, resources)


def run_locked(config, root, resources):
    start, live = validate_config(config)
    require(not (root / "FAILED.json").exists(), "failed source generation requires operator review")
    manifest = root / "manifest.json"
    expected = canonical(dict(version=VERSION, config=config))
    resuming = manifest.exists()
    if resuming:
        require(manifest.read_text() == expected, "source manifest changed")
    else:
        with manifest.open("x") as f:
            f.write(expected)
            f.flush()
            os.fsync(f.fileno())
    stopped = False

    def stop(_signum, _frame):
        nonlocal stopped
        stopped = True

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    redis = Redis()
    resources.callback(redis.close)
    count = redis.command("XLEN", config["stream"])
    require(not resuming or count > 0, "resume source stream missing/empty")
    require(resuming or count == 0, "new source generation requires empty stream")
    require(count < MAX_ROWS, "source capacity exhausted")
    entries = redis.command("XRANGE", config["stream"], "-", "+", "COUNT", MAX_ROWS)
    require(len(entries) == count, "source changed during resume")
    published = {row["time"]: row for row in (stored_row(e, config["symbol"]) for e in entries)}
    require(len(published) == count, "duplicate source labels")
    gaps_path = root / "context-gaps.jsonl"
    gaps = {}
    if gaps_path.exists():
        require(gaps_path.stat().st_size <= 1024 * 1024, "context gap audit budget")
        for line in gaps_path.read_text().splitlines():
            gap = json.loads(line)
            ts = gap["bar_ts_utc"]
            require(ts not in gaps and ts not in published, "duplicate/conflicting context gap")
            require(context_gap(ts, gap["first_absent_at_utc"], gap["confirmed_absent_at_utc"],
                                config["excluded_dates"]) == gap, "invalid context gap audit")
            gaps[ts] = gap
    gap_candidates = {}
    finalizer = Finalizer(live, config["excluded_dates"])
    api = MarketData()
    clock, mono = time.time(), time.monotonic()
    log("closed_source_started", symbol=config["symbol"], stream=config["stream"], rows=count,
        first_live_session=live.isoformat(), finality="two_authenticated_rest_observations")
    try:
        while not stopped:
            now = int(time.time())
            require(abs(time.time() - clock - (time.monotonic() - mono)) <= 5, "local clock stepped")
            require((local(now).date() - start).days <= 40, "source generation lifetime exhausted")
            due = []
            for d in range((local(now).date() - start).days + 1):
                day = start + dt.timedelta(days=d)
                due.extend(ts for ts in slots(day, config["excluded_dates"])
                           if ts + TF + GRACE <= now)
            known = set(published) | set(gaps)
            pending = [ts for ts in due if ts not in known]
            require(all(ts in due for ts in known), "source contains unexpected/future labels")
            require(not published or all(ts in known for ts in due if ts <= max(published)),
                    "source prefix missing/trimmed")
            if not pending:
                time.sleep(1)
                continue
            # Missing active-session candles fail before any catch-up publication.
            for ts in pending:
                if local(ts).date() >= live and not context_only_day(local(ts).date(), config["excluded_dates"]):
                    require(now - ts - TF <= MAX_AGE, "active source gap/deadline missed")
            first, last = pending[0], pending[-1]
            # Bootstrap in per-session requests, bounded to one day. Previously
            # published last two candles are verified on every subsequent fetch.
            request_from = request_start(first, published)
            request_to = min(last, label(local(first).date(), 23, 40))
            rows = api.bars(config["symbol"], request_from, request_to)
            observed = int(time.time())
            for ts, old in published.items():
                if request_from <= ts <= request_to:
                    require(rows.get(ts) == old, "published REST candle revised/missing")
            for ts in gaps:
                if request_from <= ts <= request_to:
                    require(ts not in rows, "previously absent context candle arrived; explicit review required")
            batch = [ts for ts in pending if ts <= request_to]
            proofs = []
            absent = []
            for ts in batch:
                row = rows.get(ts)
                if row is None:
                    if context_only_day(local(ts).date(), config["excluded_dates"]):
                        if observed >= ts + TF + MAX_AGE:
                            first_absent = gap_candidates.setdefault(ts, observed)
                            gap = context_gap(ts, first_absent, observed, config["excluded_dates"])
                            if gap:
                                absent.append(gap)
                        continue
                    require(local(ts).date() >= live, "bootstrap session incomplete")
                    break
                gap_candidates.pop(ts, None)
                proof = finalizer.observe(row, observed)
                if proof is not None:
                    proofs.append((row, proof))
            # Do not append a later confirmed candle before an earlier one.
            if len(proofs) + len(absent) != len(batch):
                time.sleep(CONFIRM)
                continue
            for gap in absent:
                require(len(gaps) < MAX_ROWS, "context gap budget exhausted")
                with gaps_path.open("a") as f:
                    f.write(canonical(gap) + "\n")
                    f.flush()
                    os.fsync(f.fileno())
                gaps[gap["bar_ts_utc"]] = gap
                log("context_bar_absent", symbol=config["symbol"], **gap)
            for row, proof in proofs:
                ts = row["time"]
                require(count < MAX_ROWS, "source row budget exhausted")
                wanted_id = "%d-0" % (ts * 1000)
                actual_id = redis.command("XADD", config["stream"], wanted_id, "payload",
                                          canonical(envelope(config["symbol"], row, proof)))
                require(actual_id == wanted_id, "unexpected source append ID")
                published[ts] = row
                count += 1
                del finalizer.candidates[ts]
                log("closed_bar_published", symbol=config["symbol"], bar_ts_utc=ts,
                    origin=proof["origin"], latency_seconds=observed-ts-TF, rows=count)
        log("closed_source_stopped", symbol=config["symbol"], reason="operator_stop", rows=count)
    except Exception as exc:
        reason = str(exc) if isinstance(exc, SourceFault) else type(exc).__name__
        with (root / "FAILED.json").open("x") as f:
            f.write(canonical(dict(reason=reason, observed_at_utc=int(time.time()))))
            f.flush()
            os.fsync(f.fileno())
        raise SourceFault(reason) from None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", type=Path)
    parser.add_argument("state", type=Path)
    args = parser.parse_args()
    try:
        run(json.loads(args.config.read_text()), args.state)
    except Exception as exc:
        log("closed_source_failed", reason=str(exc) if isinstance(exc, SourceFault) else type(exc).__name__)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
