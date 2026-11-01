"""Frozen Python trading logic + independent cutoff oracle; no Rust output input.

Only the daily-context boundary is replaced, explicitly, in temporary modules.
The instrumented and uninstrumented frozen models must yield identical outputs.
"""
from __future__ import annotations

import copy
from contextlib import ExitStack
import math

import pandas as pd

from compare import ParityError
from export_imoexf import reference_modules as imo_modules
from export_usdrubf import reference_modules as usd_modules
from reaction_fixtures import EVIDENCE, config_hash
from weekend_reference import Context


def stamp(value):
    return None if value is None or pd.isna(value) else pd.Timestamp(value).isoformat(timespec="seconds")


def label(ts):
    return (pd.Timestamp(ts, unit="s")+pd.Timedelta(hours=3)).isoformat(timespec="seconds")


def optional(value):
    return float(value) if value is not None and math.isfinite(value) else None


def position(owner, side, signal, entry, price, held=None, scale=None, anchor=None,
             stop1=None, stop2=None, stop=None, take=None):
    return dict(owner=owner, side=side, signal=stamp(signal), entry=stamp(entry), price=float(price),
                bars_held=held, scale=optional(scale), anchor=optional(anchor), stop1=optional(stop1),
                stop2=optional(stop2), stop=optional(stop), take=optional(take))


def pending(role, owner, side, signal, action, reason, scale=None, anchor=None,
            stop1=None, stop2=None, ambiguous=False):
    return dict(role=role, owner=owner, side=side, signal=stamp(signal), action=stamp(action),
                reason=reason, scale=optional(scale), anchor=optional(anchor), stop1=optional(stop1),
                stop2=optional(stop2), ambiguous=bool(ambiguous))


def event(e, rust=False):
    return dict(role=e["role"], owner=e["component"], side=e["side"], action=stamp(e["ts"]),
                signal=stamp(e.get("signal_ts")), price=optional(e.get("price")), reason=e.get("reason"))


def trade(t):
    return dict(owner=t["component"], side=t["side"], entry_signal=stamp(t["entry_signal_ts"]),
                entry=stamp(t["entry_ts"]), entry_price=float(t["entry_price"]),
                exit_signal=stamp(t["exit_signal_ts"]), exit=stamp(t["exit_ts"]),
                exit_price=float(t["exit_price"]), reason=t["exit_reason"], gross_points=float(t["gross_points"]),
                bars_held=t.get("bars_held"), ambiguous=bool(t.get("ambiguous_mr_bar", False)))


def frozen_projection(value):
    f = copy.deepcopy(value)
    f.pop("identity_sha256")
    f["barrier"].pop("ingress_sha256")
    for k in ("previous", "previous2"):
        f[k].pop("source_sha256")
    f["ready"] = True
    f["previous_return"] = f["previous"]["close"]/f["previous2"]["close"]-1
    return f


def context_anchor(a):
    return dict(day=a["session_date"], first=label(a["first_bar_utc"]), last=label(a["last_bar_utc"]),
                high=a["high"], low=a["low"], close=a["close"], count=a["distinct_bars"])


class Collector:
    def __init__(self, case, frozen, frame):
        self.case, self.frozen, self.frame = case, frozen, frame
        self.imo = case["config"]["engine"] == "imoexf"
        self.rows, self.features, self.mr_scheduled = [], {}, 0
        self.high, self.low = frame.high.cummax(), frame.low.cummin()

    def capture_features(self, state):
        self.features = pd.DataFrame({k: state[k] for k in ("numerator", "cumulative_volume")}).to_dict("index")

    def __call__(self, phase, s):
        if phase != "bar_end":
            return
        p, q, row = s["position"], s["pending"], s["row"]
        ts, count, branch = row.Index, s["i"]+1, s["_state_trace_branch"]
        pp = qq = None
        if self.imo:
            if p:
                pp = position(p["component"], p["side"], p["entry_signal_ts"], p["entry_ts"],
                              p["entry_price"], p["bars_held"])
            if q:
                owner = q.get("component", p["component"] if p else None)
                side = q.get("side", p["side"] if p else None)
                qq = pending(q["role"], owner, side, q["signal_ts"], q["when"], q.get("reason", owner.lower()+"_entry"))
            mr, long, short = s["mr_entries"], s["was_long"], s["was_short"]
            features = dict(anchor=s["pc"], scale=s["pr"], normalizer=s["pl"], previous_return=s["ret"])
            obs = dict(count=count, first=stamp(s["start"]), last=stamp(ts), high=s["day_high"],
                       low=s["day_low"], close=float(row.close), open=None, numerator=None, volume=None)
        else:
            self.mr_scheduled += branch == "mr_selected"
            if p:
                pp = position(p["component"], p["side"], p["entry_signal_ts"], p["entry_ts"], p["entry_price"],
                    scale=p["scale"], anchor=p["anchor"], stop1=p["stop1"], stop2=p["stop2"],
                    stop=p.get("stop_price"), take=p.get("take_price"))
            if q:
                owner = q.get("component", p["component"] if p else None)
                side = q.get("side", p["side"] if p else None)
                qq = pending(q["role"], owner, side, q.get("entry_signal_ts", q.get("signal_ts")), q["action_ts"],
                    q.get("reason", owner.lower()+"_entry"), scale=q.get("scale"), anchor=q.get("anchor"),
                    stop1=q.get("stop1"), stop2=q.get("stop2"), ambiguous=q.get("ambiguous_mr_bar", False))
            mr, long, short = self.mr_scheduled, "long" in s["used"], "short" in s["used"]
            features = dict(anchor=float(row.mr_anchor), scale=float(row.scale), normalizer=float(row.close), previous_return=float(row.gate_return))
            f = self.features[ts]
            obs = dict(count=count, first=stamp(self.frame.index[0]), last=stamp(ts), high=float(self.high[ts]),
                low=float(self.low[ts]), close=float(row.close), open=float(self.frame.open.iloc[0]),
                numerator=float(f["numerator"]), volume=float(f["cumulative_volume"]))
        self.rows.append(dict(branch=branch, mr_counter=mr, mr_limit=2 if self.imo else None,
            bo_long_used=bool(long), bo_short_used=bool(short), observation=obs, features=features,
            previous=context_anchor(self.frozen["previous"]), previous2=context_anchor(self.frozen["previous2"]),
            position=pp, pending=qq, events=[event(e) for e in s["events"]], trades=[trade(t) for t in s["trades"]]))


def frame_for(case):
    rows = []
    for op in case["operations"]:
        if op["kind"] == "bar":
            b = op["value"]
            rows.append(dict(ts_local=pd.Timestamp(label(b["close_time_utc"])), open=b["o"], high=b["h"],
                             low=b["l"], close=b["close"], volume=b["v"]))
    frame = pd.DataFrame(rows).set_index("ts_local")
    hour = 9 if case["day"] < "2026-07-14" or case["identity"]["profile"].endswith("09") else 7
    expected = pd.date_range(case["day"]+f" {hour:02}:00", case["day"]+" 23:40", freq="10min")
    if not frame.index.equals(expected):
        raise ParityError("incomplete or duplicate execution prefix")
    return frame


def install_context(module, cfg, frozen):
    """Explicit extension boundary. Do not replace any trading expression."""
    p, p2 = frozen["previous"], frozen["previous2"]
    if cfg["engine"] == "imoexf":
        original = module.context
        def context(frame, daily_close_overrides=None):
            if daily_close_overrides:
                raise ParityError("unexpected close override")
            return pd.DataFrame([dict(prev_high=p["high"], prev_low=p["low"], prev_close=p["close"],
                prev_range=p["range"], prev2_close=p2["close"], prev_return=(p["close"]-p2["close"])/p2["close"])],
                index=[pd.Timestamp(frozen["session_date"])])
        module.context = context
        return lambda: setattr(module, "context", original)
    original = module.features
    def features(frame, family, **kwargs):
        if family != "previous_day":
            raise ParityError("no VWAP context replacement permitted")
        work = original(frame, family, **kwargs)
        work["mr_anchor"] = work["bo_anchor"] = p["close"]
        work["scale"] = p["range"]
        work["gate_return"] = p["close"]/p2["close"]-1
        return work
    module.features = features
    return lambda: setattr(module, "features", original)


def run_reference(case, frozen, modules):
    cfg = case["config"]
    frame = frame_for(case)
    original, observed, _ = modules
    collector = Collector(case, frozen, frame)
    with ExitStack() as stack:
        stack.callback(install_context(original, cfg, frozen))
        stack.callback(install_context(observed, cfg, frozen))
        def run(m, observer=None):
            kw = dict(_state_trace=observer) if observer is not None else {}
            if cfg["engine"] == "imoexf":
                sleeve = cfg["sleeve"]
                candidate = m.Candidate("fixture_"+sleeve, sleeve != "bo", None if sleeve == "mr" else "hybrid", .53)
                return m.replay(frame, candidate, hour=7, contract="reaction_next_open", **kw)
            return m.replay(frame, m.Candidate(**cfg["settings"]), **kw)
        baseline = run(original)
        instrumented = run(observed, collector)
        for left, right in zip(baseline, instrumented):
            pd.testing.assert_frame_equal(left, right, check_exact=True)
    if len(collector.rows) != len(frame):
        raise ParityError("reference observer coverage mismatch")
    return collector.rows


def expected_case(case, modules):
    if case["identity"]["model_config_sha256"] != config_hash(case["config"]):
        raise ParityError("fixture model config hash mismatch")
    ctx = Context(case["identity"])
    routes = [ctx.ingest(h["value"], h["processed_at_utc"]) for h in case["history"]]
    ctx.freeze(case["day"], case["freeze_at_utc"])
    frozen = copy.deepcopy(ctx.frozen[-1])
    if not frozen["ready"]:
        raise ParityError("missing frozen anchors")
    observed = run_reference(case, frozen, modules)
    rows, bar_index, last, floor = [], 0, None, case["freeze_at_utc"]
    for index, op in enumerate(case["operations"]):
        route = None
        if op["kind"] == "bar":
            if op["value"]["origin"] != "replay" or op["value"]["symbol"] != ctx.cfg["symbol"]:
                raise ParityError("wrong execution input")
            now = op["observed_at_utc"]
            if now < max(floor, op["value"]["close_time_utc"]+600):
                raise ParityError("execution clock/availability mismatch")
            floor = now
            last = observed[bar_index]
            bar_index += 1
        elif op["kind"] == "restart":
            if op["now"] < floor:
                raise ParityError("restart clock regression")
            ctx.check_clock(op["now"])
            ctx.clock = floor = op["now"]
        elif op["kind"] == "receipt":
            route = ctx.ingest(op["value"], op["processed_at_utc"])
        else:
            raise ParityError("unknown execution operation")
        if last is None:
            raise ParityError("fixture must observe a bar before recovery/late receipt")
        rows.append(dict(index=index, kind=op["kind"], route=route, context_sequence=len(ctx.receipts),
                         context_accepted=len(ctx.accepted), bars_processed=bar_index, model=copy.deepcopy(last)))
    if last["position"] or last["pending"]:
        raise ParityError("unresolved reference session")
    return dict(id=case["id"], identity=case["identity"], config=case["config"], routes=routes, frozen=frozen,
                rows=rows, complete=True, order_emission_allowed=False)


def expected(fixture, imo_handoff, usd_handoff):
    if (fixture["schema_version"] != "weekend_reaction_fixture_v1" or fixture["evidence_kind"] != EVIDENCE
        or not 1 <= len(fixture["cases"]) <= 128 or len({c["id"] for c in fixture["cases"]}) != len(fixture["cases"])):
        raise ParityError("invalid fixture identity/coverage")
    with imo_modules(imo_handoff) as im, usd_modules(usd_handoff) as us:
        return [expected_case(c, im if c["config"]["engine"] == "imoexf" else us) for c in fixture["cases"]]
