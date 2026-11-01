"""Historical numerical RI oracle. Availability is NOT collector/fsync evidence."""
from __future__ import annotations

import copy
from datetime import date, datetime, timedelta

from compare import ParityError, compare_value, nullable, validate
from ri_reference import MSK, POLICIES, Reference, is_model_bar, local, opening
from run_ri_fixture_parity import BAR, STATE

SCHEMA = "ri_numerical_history_v1"
EVIDENCE = "legacy_mixed_availability_clock_not_durability"
STREAM = {"symbol": str, "first_decision_session": str, "source_sha256": str,
          "inputs": [{"bar": BAR, "available_epoch": int}]}
FIXTURE = {"schema_version": str, "evidence_kind": str, "streams": [STREAM]}
ROW = {"symbol": str, "policy": str, "index": int, "bar_ts_utc": int,
       "available_epoch": int, "origin": str, "route": str,
       "observed": bool, "state": nullable(STATE)}


def cutoff(day):
    return int(datetime.fromisoformat(day + "T" + opening(day)).replace(tzinfo=MSK).timestamp())


def next_cutoff(day):
    d = date.fromisoformat(day) + timedelta(days=1)
    while d.weekday() >= 5:
        d += timedelta(days=1)
    return cutoff(d.isoformat())


def route(item, policy):
    b, available = item["bar"], item["available_epoch"]
    if not is_model_bar(b):
        return "quarantined:non_model_bar"
    if available >= next_cutoff(local(b["close_time_utc"]).date().isoformat()):
        return "quarantined:at_or_after_cutoff"
    if b["origin"] == "history_gap":
        if policy == POLICIES[0]:
            return "quarantined:recovery_disabled"
        if local(b["close_time_utc"]).strftime("%H:%M") != "23:40":
            return "quarantined:non_closing_recovery"
    return "context_only"


def validate_fixture(f):
    validate(f, FIXTURE)
    if (f["schema_version"] != SCHEMA or f["evidence_kind"] != EVIDENCE
        or not 1 <= len(f["streams"]) <= 2
        or len({s["symbol"] for s in f["streams"]}) != len(f["streams"])):
        raise ParityError("historical numerical fixture identity mismatch")
    for s in f["streams"]:
        first = date.fromisoformat(s["first_decision_session"])
        if (s["symbol"] not in ("RIU6", "RIZ6") or first.weekday() >= 5
            or len(s["source_sha256"]) != 64 or any(c not in "0123456789abcdef" for c in s["source_sha256"])
            or not 1 <= len(s["inputs"]) <= 9900):
            raise ParityError("invalid stream identity/size")
        previous, seen = None, set()
        for i in s["inputs"]:
            b, available = i["bar"], i["available_epoch"]
            ts = b["close_time_utc"]
            key = (available, ts)
            if (b["symbol"] != s["symbol"] or ts in seen or available < ts + 600
                or (previous is not None and key <= previous)
                or b["origin"] not in ("live", "history", "history_gap")
                or not 0 < b["l"] <= min(b["o"], b["close"]) <= max(b["o"], b["close"]) <= b["h"] or b["v"] < 0):
                raise ParityError("invalid/duplicate/out-of-order numerical history input")
            seen.add(ts)
            previous = key


def assert_flat(state, where):
    if state and (state["mr"] or state["bo"] or state["pending"] or state["phase"] != "flat"):
        raise ParityError(f"unresolved historical position/pending at {where}; never reset to flat")


def expected_rows(f):
    validate_fixture(f)
    rows = []
    for stream in f["streams"]:
        for policy in POLICIES:
            canonical, model, state = [], None, None
            for index, item in enumerate(stream["inputs"]):
                b, available = item["bar"], item["available_epoch"]
                dt = local(b["close_time_utc"])
                day = dt.date().isoformat()
                routing = route(item, policy)
                observed = (routing == "context_only" and b["origin"] == "live"
                            and day >= stream["first_decision_session"]
                            and dt.date() == local(available).date())
                if observed:
                    if model is None or model.day != day:
                        assert_flat(state, day)
                        # Only admitted rows available BEFORE the opening cutoff
                        # may form the frozen view, even if first observation is late.
                        warmup = [i["bar"] for i in canonical if i["available_epoch"] < cutoff(day)
                                  and local(i["bar"]["close_time_utc"]).date() < dt.date()]
                        model = Reference(warmup, day, policy)
                    state = model.step(b)
                elif state:
                    state = copy.deepcopy(state)
                    state["branches"], state["actions"] = [], []
                rows.append(dict(symbol=stream["symbol"], policy=policy, index=index,
                                 bar_ts_utc=b["close_time_utc"], available_epoch=available,
                                 origin=b["origin"], route=routing, observed=observed, state=state))
                if routing == "context_only":
                    canonical.append(item)
            if model is None:
                raise ParityError("history has no observations in the declared activation scope")
            assert_flat(state, "end of " + stream["symbol"])
    return rows


def compare_rows(expected, actual):
    if len(expected) != len(actual):
        raise ParityError("missing/extra historical receipt rows")
    for n, (left, right) in enumerate(zip(expected, actual)):
        try:
            validate(left, ROW)
            validate(right, ROW)
            compare_value(left, right, ROW)
        except ParityError as e:
            raise ParityError(f"row={n} {left['symbol']} {left['policy']} input={left['index']}: {e}") from e
