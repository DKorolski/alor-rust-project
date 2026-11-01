"""Independent context-only oracle. Synthetic evidence, not a trading engine."""
from __future__ import annotations

import copy
from datetime import date, datetime, timedelta

from compare import ParityError, nullable, validate
from ri_reference import MSK, local
from run_ri_fixture_parity import BAR

POLICIES = ("weekday_only", "weekend_context_no_trades")
PROFILES = ("imoexf_reaction07", "usdrubf_previous_day07", "usdrubf_previous_day09")
IDENTITY = {"profile": str, "symbol": str, "model_config_sha256": str,
            "receipt_stream": str, "policy": str, "excluded_dates": [str]}
RECEIPT = {"sequence": int, "source_id": str, "bar": BAR,
           "received_at_utc": nullable(int), "durable_at_utc": nullable(int)}
ANCHOR = {"session_date": str, "open": float, "high": float, "low": float,
          "close": float, "range": float, "distinct_bars": int, "required_bars": int,
          "total_slots": int, "first_bar_utc": int, "last_bar_utc": int,
          "latest_received_at_utc": int, "latest_durable_at_utc": int, "source_sequences": [int]}
FROZEN = {"session_date": str, "cutoff_utc": int, "frozen_at_utc": int,
          "barrier": {"receipt_stream": str, "through_utc": int, "last_sequence": int, "attested_at_utc": int},
          "previous": nullable(ANCHOR), "previous2": nullable(ANCHOR), "ready": bool,
          "previous_return": nullable(float), "skipped": [{"session_date": str, "distinct_bars": int,
              "required_bars": int, "first_bar_utc": int, "last_bar_utc": int, "reason": str}]}
STATE = {"last_sequence": int, "accepted_count": int, "frozen": [FROZEN]}


def opening(cfg, day):
    return 9 if day < "2026-07-14" or cfg["profile"] == PROFILES[2] else 7


def cutoff(cfg, day):
    return int(datetime.fromisoformat(f"{day}T{opening(cfg, day):02}:00").replace(tzinfo=MSK).timestamp())


def next_cutoff(cfg, day):
    d = date.fromisoformat(day) + timedelta(days=1)
    while d.weekday() >= 5 or d.isoformat() in cfg["excluded_dates"]:
        d += timedelta(days=1)
    return cutoff(cfg, d.isoformat())


def quote_bits(bar):
    return tuple(float(bar[k]).hex() for k in ("o", "h", "l", "close", "v"))


class Context:
    def __init__(self, identity):
        validate(identity, IDENTITY)
        if identity["profile"] not in PROFILES or identity["policy"] not in POLICIES:
            raise ParityError("unsupported context profile/policy")
        expected_symbol = "IMOEXF" if identity["profile"] == PROFILES[0] else "USDRUBF"
        if identity["symbol"] != expected_symbol or not identity["receipt_stream"]:
            raise ParityError("context identity mismatch")
        digest = identity["model_config_sha256"]
        if (len(digest) != 64 or any(c not in "0123456789abcdefABCDEF" for c in digest)
            or len(identity["receipt_stream"]) > 256 or len(identity["excluded_dates"]) > 256):
            raise ParityError("invalid context identity bounds")
        for day in identity["excluded_dates"]:
            date.fromisoformat(day)
        self.cfg = identity
        self.receipts, self.accepted, self.frozen = [], {}, []
        self.sources, self.clock = {}, None

    def check_clock(self, now):
        local(now)
        if self.clock is not None and now < self.clock:
            raise ParityError("processing clock regression")

    def rejection(self, r, now):
        b = r["bar"]
        dt = local(b["close_time_utc"])
        day, minute = dt.date().isoformat(), dt.hour*60 + dt.minute
        weekend = dt.weekday() >= 5
        if b["symbol"] != self.cfg["symbol"]:
            return "wrong_contract"
        if day in self.cfg["excluded_dates"]:
            return "excluded_date"
        if weekend and self.cfg["policy"] == POLICIES[0]:
            return "weekend_disabled"
        start, end = (590, 1130) if weekend else (opening(self.cfg, day)*60, 1420)
        if not start <= minute <= end or dt.minute % 10 or dt.second:
            return "non_model_bar"
        if not 0 < b["l"] <= min(b["o"], b["close"]) <= max(b["o"], b["close"]) <= b["h"] or b["v"] < 0:
            return "invalid_ohlcv"
        received, durable = r["received_at_utc"], r["durable_at_utc"]
        if received is None:
            return "missing_receipt"
        if durable is None:
            return "missing_durability"
        if not received <= durable <= now:
            return "invalid_clock"
        if received < b["close_time_utc"]+600:
            return "bar_not_closed"
        if durable >= next_cutoff(self.cfg, day):
            return "at_or_after_cutoff"
        if b["origin"] == "replay":
            return "unsupported_origin"
        return None

    def ingest(self, r, now):
        validate(r, RECEIPT)
        self.check_clock(now)
        if r["sequence"] != len(self.receipts)+1 or not r["source_id"]:
            raise ParityError("invalid receipt sequence/source id")
        fingerprint = (r["source_id"], r["bar"]["symbol"], r["bar"]["close_time_utc"],
                       r["bar"]["origin"], quote_bits(r["bar"]), r["received_at_utc"], r["durable_at_utc"])
        reason, duplicate = None, False
        if r["source_id"] in self.sources:
            duplicate = self.sources[r["source_id"]] == fingerprint
            if not duplicate:
                reason = "conflicting_receipt"
        else:
            reason = self.rejection(r, now)
            if reason is None:
                ts = r["bar"]["close_time_utc"]
                if ts in self.accepted:
                    old = self.accepted[ts]["bar"]
                    duplicate = quote_bits(old) == quote_bits(r["bar"])
                    if not duplicate:
                        reason = "conflicting_quote"
                elif any(ts < f["cutoff_utc"] and r["durable_at_utc"] < f["cutoff_utc"] for f in self.frozen):
                    reason = "behind_frozen_barrier"
        route = {"kind": "quarantined", "reason": reason} if reason else {"kind": "duplicate" if duplicate else "context_only"}
        if route["kind"] == "context_only":
            self.accepted[r["bar"]["close_time_utc"]] = copy.deepcopy(r)
        self.sources.setdefault(r["source_id"], copy.deepcopy(fingerprint))
        self.receipts.append(copy.deepcopy(r))
        self.clock = now
        return route

    def freeze(self, day, now):
        self.check_clock(now)
        cut = cutoff(self.cfg, day)
        if (date.fromisoformat(day).weekday() >= 5 or day in self.cfg["excluded_dates"] or now < cut
            or (self.frozen and day <= self.frozen[-1]["session_date"])):
            raise ParityError("invalid freeze")
        days = {}
        for ts, r in sorted(self.accepted.items()):
            d = local(ts).date().isoformat()
            if d < day and r["durable_at_utc"] < cut:
                days.setdefault(d, []).append(r)
        anchors, skipped = [], []
        for d, rs in sorted(days.items()):
            weekend = date.fromisoformat(d).weekday() >= 5
            total = 55 if weekend else (1420-opening(self.cfg, d)*60)//10+1
            required = 44 if weekend else total
            first, last = rs[0]["bar"]["close_time_utc"], rs[-1]["bar"]["close_time_utc"]
            minute = lambda t: local(t).hour*60+local(t).minute
            if len(rs) < required or minute(first) > (600 if weekend else opening(self.cfg, d)*60) or minute(last) < (1130 if weekend else 1420):
                skipped.append(dict(session_date=d, distinct_bars=len(rs), required_bars=required,
                                    first_bar_utc=first, last_bar_utc=last, reason="incomplete_context"))
                continue
            high, low = max(r["bar"]["h"] for r in rs), min(r["bar"]["l"] for r in rs)
            anchors.append(dict(session_date=d, open=rs[0]["bar"]["o"], high=high, low=low,
                close=rs[-1]["bar"]["close"], range=high-low, distinct_bars=len(rs), required_bars=required,
                total_slots=total, first_bar_utc=first, last_bar_utc=last,
                latest_received_at_utc=max(r["received_at_utc"] for r in rs),
                latest_durable_at_utc=max(r["durable_at_utc"] for r in rs), source_sequences=[r["sequence"] for r in rs]))
        previous, previous2 = (anchors[-1] if anchors else None), (anchors[-2] if len(anchors) >= 2 else None)
        self.frozen.append(dict(session_date=day, cutoff_utc=cut, frozen_at_utc=now,
            barrier=dict(receipt_stream=self.cfg["receipt_stream"], through_utc=cut,
                         last_sequence=len(self.receipts), attested_at_utc=now),
            previous=previous, previous2=previous2, ready=previous is not None and previous2 is not None,
            previous_return=previous["close"]/previous2["close"]-1 if previous2 else None, skipped=skipped))
        self.clock = now

    def state(self):
        return copy.deepcopy(dict(last_sequence=len(self.receipts), accepted_count=len(self.accepted), frozen=self.frozen))


def expected_rows(f):
    validate(f, {"schema_version": str, "evidence_kind": str, "cases": [dict]})
    if f["schema_version"] != "weekend_context_fixture_v1" or f["evidence_kind"] != "synthetic_original_receipt_fixture_not_production":
        raise ParityError("fixture evidence/schema mismatch")
    if not 1 <= len(f["cases"]) <= 128 or len({c["id"] for c in f["cases"]}) != len(f["cases"]):
        raise ParityError("invalid fixture case coverage")
    rows = []
    for case in f["cases"]:
        validate(case, {"id": str, "identity": IDENTITY, "operations": [dict]})
        ctx = Context(case["identity"])
        if not 1 <= len(case["operations"]) < 2000:
            raise ParityError("invalid operation coverage")
        for index, op in enumerate(case["operations"]):
            routing = None
            if op["kind"] == "receipt":
                validate(op, {"kind": str, "value": RECEIPT, "processed_at_utc": int})
                routing = ctx.ingest(op["value"], op["processed_at_utc"])
            elif op["kind"] == "freeze":
                validate(op, {"kind": str, "day": str, "now": int})
                ctx.freeze(op["day"], op["now"])
            elif op["kind"] == "restart":
                validate(op, {"kind": str, "now": int})
                ctx.check_clock(op["now"])
                ctx.clock = op["now"]  # independent uninterrupted logical expectation
            else:
                raise ParityError("unsupported operation")
            rows.append(dict(case=case["id"], index=index, route=routing, state=ctx.state()))
    return rows
