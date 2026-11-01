"""Explicit synthetic receipt/cutoff scenarios, never inferred historical fsync."""
import copy

from ri_fixtures import bar
from weekend_reference import POLICIES, PROFILES, cutoff, opening

SCENARIOS = ("on_time", "exact_cutoff", "late_before_first_event", "durability_crossing",
             "delayed_processing", "missing_receipt", "missing_durability", "after_freeze",
             "duplicate_conflict", "sparse44", "sparse43", "first1010", "weekday_incomplete")


def make_case(profile, policy, scenario):
    cfg = dict(profile=profile, symbol="IMOEXF" if profile == PROFILES[0] else "USDRUBF",
               model_config_sha256="a"*64, receipt_stream="fixture-only", policy=policy, excluded_dates=[])
    ops, sequence = [], 0

    def ingest(b, received=None, durable=None, processed=None, source=None):
        nonlocal sequence
        sequence += 1
        received = b["close_time_utc"]+600 if received is None else received
        durable = received if durable is None else durable
        r = dict(sequence=sequence, source_id=source or f"source-{sequence}", bar=b,
                 received_at_utc=received, durable_at_utc=durable)
        ops.append(dict(kind="receipt", value=r, processed_at_utc=durable if processed is None else processed))
        return r

    def bars(day, close, weekend=False, end=True):
        start, last = (590, 1130) if weekend else (opening(cfg, day)*60, 1420)
        labels = list(range(start, last+1, 10))
        if weekend and day == "2026-10-25":
            if scenario in ("sparse44", "sparse43"):
                labels = labels[:43 if scenario == "sparse44" else 42] + [1130]
            elif scenario == "first1010":
                labels = labels[2:]
        if not end:
            labels = labels[:-1]
        for m in labels:
            b = bar(day, f"{m//60:02}:{m%60:02}", close, high=200 if weekend and m == 590 else close+2, low=close-2)
            b["symbol"] = cfg["symbol"]
            ingest(b)

    bars("2026-10-22", 100.)
    fri = cutoff(cfg, "2026-10-23")
    ops.extend([dict(kind="restart", now=fri), dict(kind="freeze", day="2026-10-23", now=fri)])
    bars("2026-10-23", 105., end=scenario != "weekday_incomplete")
    bars("2026-10-24", 103., weekend=True)
    bars("2026-10-25", 104., weekend=True, end=False)
    mon = cutoff(cfg, "2026-10-26")
    closing = bar("2026-10-25", "18:50", 104., high=106, low=102, origin="history_gap")
    closing["symbol"] = cfg["symbol"]
    received, durable, processed = mon-2, mon-1, mon-1
    if scenario == "exact_cutoff":
        received = durable = processed = mon
    if scenario == "late_before_first_event":
        received = durable = processed = mon+300
    if scenario == "durability_crossing":
        durable = processed = mon
    if scenario in ("delayed_processing", "after_freeze"):
        processed = mon+300
    if scenario == "after_freeze":
        ops.extend([dict(kind="freeze", day="2026-10-26", now=mon), dict(kind="restart", now=mon)])
    r = ingest(closing, received, durable, processed)
    if scenario == "missing_receipt":
        r["received_at_utc"] = None
    if scenario == "missing_durability":
        r["durable_at_utc"] = None
    if scenario == "duplicate_conflict":
        duplicate = ingest(copy.deepcopy(closing), received, durable, mon, source=r["source_id"])
        changed = copy.deepcopy(closing)
        changed["close"] = 105.
        ingest(changed, received, durable, mon, source=r["source_id"])
        ingest(changed, received, durable, mon)
        assert duplicate["sequence"] != r["sequence"]
    if scenario != "after_freeze":
        ops.extend([dict(kind="restart", now=mon+600), dict(kind="freeze", day="2026-10-26", now=mon+600)])
    ops.append(dict(kind="restart", now=mon+600))
    bars("2026-10-26", 108.)
    tue = cutoff(cfg, "2026-10-27")
    ops.extend([dict(kind="freeze", day="2026-10-27", now=tue), dict(kind="restart", now=tue)])
    return dict(id=f"{profile}__{policy}__{scenario}", identity=cfg, operations=ops)


def fixtures():
    return dict(schema_version="weekend_context_fixture_v1", evidence_kind="synthetic_original_receipt_fixture_not_production",
                cases=[make_case(p, policy, scenario) for p in PROFILES for policy in POLICIES for scenario in SCENARIOS])
