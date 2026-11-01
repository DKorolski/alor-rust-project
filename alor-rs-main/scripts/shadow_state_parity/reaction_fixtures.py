"""Fixed synthetic inputs for context-to-decision parity, not historical receipts."""
from __future__ import annotations

import hashlib
import json
from datetime import date, datetime, timedelta

from ri_reference import MSK
from weekend_reference import POLICIES, cutoff, opening

EVIDENCE = "synthetic_numerical_reaction_not_production"
SCENARIOS = ("mr_take_bo_eod", "mr_ambiguous_stop", "mr_quota", "mr_hold_force", "mr_bo_overlap",
             "bo_terminal", "late_sunday_close", "after_freeze_close", "pre_transition")


def configs():
    values = [dict(engine="imoexf", sleeve=sleeve, costs=dict(point_value_rub=10.,
                  tick_value_rub=5., rate_per_component_per_side=.001, components=2))
              for sleeve in ("mr", "bo", "combo")]
    for hour, horizon, wait in ((7, 3, 4), (7, 4, 4), (7, 3, 2), (9, 3, 2)):
        values.append(dict(engine="usdrubf", settings=dict(model_id=f"fixture_pd{hour}_h{horizon}_w{wait}",
            family="previous_day", context_hour=hour, clock_hour=hour, mr_horizon=horizon,
            bo_wait=wait, mr_start_delay=0)))
    return values


def config_hash(config):
    # Fixture floats deliberately avoid exponent-format differences. Field order
    # matches the typed Rust Config serialization, independently checked at import.
    encoded = json.dumps(["frozen_reaction_prefix_v1", config], separators=(",", ":"), allow_nan=False)
    return hashlib.sha256(encoded.encode()).hexdigest()


def identity(config, policy):
    imo = config["engine"] == "imoexf"
    hour = 7 if imo else config["settings"]["context_hour"]
    return dict(profile="imoexf_reaction07" if imo else f"usdrubf_previous_day{hour:02}",
                symbol="IMOEXF" if imo else "USDRUBF", model_config_sha256=config_hash(config),
                receipt_stream="synthetic-reaction-fixture-v1", policy=policy, excluded_dates=[])


def epoch(day, hour, minute):
    return int(datetime.combine(day, datetime.min.time()).replace(hour=hour, minute=minute,
                                                                 tzinfo=MSK).timestamp())


def bar(symbol, ts, price, origin="replay", spread=0.):
    return dict(symbol=symbol, close_time_utc=ts, close=float(price), o=float(price),
                h=float(price+spread), l=float(price-spread), v=1., origin=origin)


def make_case(config, policy, scenario):
    cfg = identity(config, policy)
    day = date.fromisoformat("2026-07-13" if scenario == "pre_transition" else "2026-10-26")
    history, delayed = [], None
    cut = cutoff(cfg, str(day))
    for offset, price in ((4, 100.), (3, 100.), (2, 102.), (1, 103.)):
        d = day-timedelta(days=offset)
        weekend = d.weekday() >= 5
        start = epoch(d, 9, 50) if weekend else cutoff(cfg, str(d))
        end = epoch(d, 18, 50) if weekend else epoch(d, 23, 40)
        for ts in range(start, end+1, 600):
            r = dict(sequence=0, source_id=f"synthetic-{ts}", bar=bar(cfg["symbol"], ts, price, "history", 2.),
                     received_at_utc=ts+600, durable_at_utc=ts+600)
            if offset == 1 and ts == end and scenario in ("late_sunday_close", "after_freeze_close"):
                delayed = r
            else:
                history.append(dict(value=r, processed_at_utc=ts+600))
    freeze = cut
    if scenario == "late_sunday_close":
        delayed["received_at_utc"] = delayed["durable_at_utc"] = cut+60
        history.append(dict(value=delayed, processed_at_utc=cut+60))
        freeze = cut+120  # first-event delay must not relax the calendar cutoff
    if scenario == "pre_transition":
        history[-1]["processed_at_utc"] = cut+60  # original receipt/durability stay before cutoff
        freeze = cut+120
    for index, h in enumerate(history, 1):
        h["value"]["sequence"] = index
    count = (23*60+40-opening(cfg, str(day))*60)//10+1
    ops = []
    for i in range(count):
        if scenario == "bo_terminal":
            price = 104. if i >= count-3 else 100.
        elif scenario in ("mr_hold_force", "mr_bo_overlap"):
            price = 100.1 if i < 2 else 100.2
            if scenario == "mr_bo_overlap" and i >= 18:
                price = 104.
        elif scenario == "mr_quota":
            price = (100.1, 99.4, 99.6)[i % 3] if i < 9 else (100. if i < 24 else 104.)
        else:
            price = (100.1, 100.12, 99.4, 99.6)[i] if i < 4 else (100. if i < 24 else 104.)
        b = bar(cfg["symbol"], cut+i*600, price)
        if scenario == "mr_ambiguous_stop" and i == 1:
            b["h"], b["l"] = 104., 98.
        now = b["close_time_utc"]+600
        ops.append(dict(kind="bar", value=b, observed_at_utc=now))
        if i == 0 and scenario == "after_freeze_close":
            delayed["sequence"] = len(history)+1
            ops.append(dict(kind="receipt", value=delayed, processed_at_utc=now+1))
        if i in (1, 2, 3, 24, count-2):
            ops.append(dict(kind="restart", now=now))
    label = "imoexf_"+config["sleeve"] if config["engine"] == "imoexf" else config["settings"]["model_id"]
    return dict(id=f"{label}/{policy}/{scenario}", identity=cfg, config=config, history=history,
                day=str(day), freeze_at_utc=freeze, operations=ops)


def fixtures():
    return dict(schema_version="weekend_reaction_fixture_v1", evidence_kind=EVIDENCE,
                cases=[make_case(cfg, policy, s) for cfg in configs() for policy in POLICIES for s in SCENARIOS])
