"""Export private, pinned final-history A/A, A/B and lag1-OI reference cases.

Requires the documented pandas/pyarrow environment. No network or research writes.
Output is an exclusive JSONL file with a final completeness/hash record. Synthetic
receipt clocks describe numerical replay ONLY, never historical PIT availability.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import math
import os
from pathlib import Path
import types

from export_component_reaction_reference import (
    CALENDAR_SHA, COMPILER_SHA, MODEL_SHA, OI_IO_SHA, OI_PROTOCOL_SHA, PROTOCOL_SHA,
    SESSIONS_SHA, STATE_COLUMNS, epoch, function_tree, load_kernels, pinned, replay_reference,
    resolve_model,
)

STUDY_SHA = "a2548f077717512c638fe20737bf4a8e1e4ee64909eed5b467cbbfc59019ee50"
MANIFEST_SHA = "b6a07b9d15adc08b925a649bd21c73efa4fa44385f32cd80e000e91b32aa7e5e"
BOOK_PINS = {
    "component_trades": ("moex_component_context_2026_11_15/results/trade_log.csv",
                         "d978e112dfa6bd3aca599df0815325cd3b8b1c5d11828b64667599bb4ce89618"),
    "component_events": ("moex_component_context_2026_11_15/results/events.csv",
                         "0dd6e0d5b7bb09d884ccfe2e7806979d439fbefb99484434d0105203aa98673e"),
    "oi_gates": ("moex_oi_gate_overnight_2026_11_15/results/oi_gate_days.csv",
                 "d536de8141b0c04939a5337ece6565e755d15b7227a74034bc957b82e8636bf3"),
    "oi_trades": ("moex_oi_gate_overnight_2026_11_15/results/oi_trades.csv",
                  "90b799f1980c16944d42434ee01cdd4c8bf7dcc1ccd35fb634affcc4d8a52d19"),
    "oi_events": ("moex_oi_gate_overnight_2026_11_15/results/oi_events.csv",
                  "dafbecb45dd9f840b988b760aca88d2127bb1c887599af7d8e006f61317002f2"),
}
ARMS = (
    ("IMO_AA", "IMO_AA", "mr_A_bo_A"),
    ("IMO_AB", "IMO_AB", "mr_A_bo_B"),
    ("IMO_AB_OI_OFF", "IMO_AB_OI", "mr_A_bo_B"),
)


def component_features(row):
    values = [float(row["prev_"+k]) for k in ("close", "high", "low", "range")]
    values += [float(row.prev2_close), float(row.prev_return)]
    return values + [values[3], values[3]] if all(math.isfinite(v) for v in values) else None


def compare_records(pd, actual, expected, label):
    try:
        pd.testing.assert_frame_equal(pd.DataFrame(actual), pd.DataFrame(expected),
            check_dtype=False, check_exact=False, rtol=0, atol=1e-9)
    except AssertionError as error:
        raise AssertionError(f"frozen book mismatch: {label}\n{error}") from error


def stored_trades(frame):
    result = []
    for r in frame.to_dict("records"):
        result.append(dict(component=r["component"], side=r["side"], entry_ts_utc=epoch(r["entry_ts"]),
            entry_signal_ts_utc=epoch(r["entry_signal_ts"]), entry_price=r["entry_price"],
            bars_held=r["bars_held"], exit_ts_utc=epoch(r["exit_ts"]),
            exit_signal_ts_utc=epoch(r["exit_signal_ts"]), exit_price=r["exit_price"],
            exit_reason=r["exit_reason"], gross_points=r["gross_points"],
            friction_rub=r["cost_x1_rub"], net_x1_rub=r["net_x1_rub"], net_x2_rub=r["net_x2_rub"]))
    return result


def stored_events(pd, frame):
    return [dict(role=r["role"], ts_utc=epoch(r["ts"]), component=r["component"], side=r["side"],
                 signal_ts_utc=epoch(r["signal_ts"]) if pd.notna(r["signal_ts"]) else None,
                 price=r["price"] if pd.notna(r["price"]) else None) for r in frame.to_dict("records")]


def load_history(research, pd):
    import pyarrow
    if pyarrow.__version__ != "18.1.0":
        raise ValueError("use the documented pyarrow==18.1.0 reference environment")
    comp = research / "moex_component_context_2026_11_15"
    base = resolve_model(research).parents[2]
    model = base / "live_baseline_revalidation/imoexf_author_comparison"
    batch = research / "moex_mr_robust_range_batch2_2026_11_15"
    manifest = json.loads(pinned(comp / "frozen_inputs.json", MANIFEST_SHA))
    study = pinned(batch / "run_study.py", STUDY_SHA).decode()
    previous = types.SimpleNamespace()
    env = dict(pd=pd, A=research, HERE=batch, IMO=model, AUDIT=base)
    exec(compile(function_tree(study, ("sources", "read_bars")), "pinned-history-input-functions", "exec"), env)
    previous.sources, previous.read_bars = env["sources"], env["read_bars"]
    paths = [path for path, _ in previous.sources("IMOEXF")]
    paths.append(base / "live_baseline_revalidation/cache/IMOEXF_raw_10m.parquet")
    inputs = {}
    for path in paths:
        relative = os.path.relpath(path, comp)
        expected = manifest[relative]
        pinned(path, expected)
        inputs[relative] = expected
    source = pinned(research / "moex_calendar_context_2026_11_15/session_data.py", SESSIONS_SHA).decode()
    raw_env = dict(pd=pd, previous=previous, BASE=base / "live_baseline_revalidation")
    exec(compile(function_tree(source, ("raw_python",)), "pinned-raw-python-function", "exec"), raw_env)
    raw, used = raw_env["raw_python"]("IMOEXF")
    if sorted(map(str, used)) != sorted(map(str, paths)):
        raise ValueError("pinned history source inventory changed")
    return raw, inputs


def load_books(research, pd):
    books = {}
    for name, (path, expected) in BOOK_PINS.items():
        pinned(research / path, expected)
        frame = pd.read_csv(research / path)
        for column in ("date", "ts", "signal_ts", "entry_ts", "entry_signal_ts", "exit_ts", "exit_signal_ts"):
            if column in frame:
                frame[column] = pd.to_datetime(frame[column])
        books[name] = frame
    gates = books["oi_gates"]
    gates = gates.loc[gates.profile.eq("IMO_AB") & gates.policy.eq("negative_oi_lag1")]
    if gates.date.duplicated().any() or not gates.BO_entry_allowed.isin([True, False]).all():
        raise ValueError("invalid pinned OI cohort")
    return books, dict(zip(gates.date, gates.BO_entry_allowed))


def indices(frame, first, after_last):
    return [int(frame.index.searchsorted(first)), int(frame.index.searchsorted(after_last))]


def calendar_definition(cal, pd, first, last):
    sessions = []
    for day in pd.date_range(first, last):
        s = cal.session(day, "IMOEXF")
        active = s.kind in ("REGULAR", "DSVD")
        sessions.append(dict(date=str(day.date()), exchange_trade_date=None,
            kind=s.kind if active else "CLOSED",
            context_open_minute=s.open_minute if active else None,
            close_minute=s.end_minute if active else None,
            main_open_minute=s.main_open if active else None,
            main_close_minute=s.main_end if active else None,
            breaks=[dict(start_minute=a, end_minute=b) for a,b in s.breaks], note=s.note or s.kind))
    return dict(schema_version="moex_shadow_calendar_v1",
        calendar_version="pinned-history-component-20261115-v1", reference_sha256=CALENDAR_SHA,
        symbol="IMOEXF", first_date=str(first.date()), last_date=str(last.date()), sessions=sessions)


def export(research, output):
    cal, sf, pd, build, leg_cost = load_kernels(research)
    cfg = json.loads(pinned(research / "moex_component_context_2026_11_15/protocol.json", PROTOCOL_SHA))
    raw, input_pins = load_history(research, pd)
    books, gates = load_books(research, pd)
    inventory, current_bars = sf["inspect_sessions"](raw, "IMOEXF")
    ledgers = {key: sf["build_ledger"](inventory, key == "B") for key in ("A", "B")}
    lookup = {key: frame.set_index("date") for key, frame in ledgers.items()}
    start, end = (pd.Timestamp(cfg[key]) for key in ("data_start", "data_end"))
    dates = inventory.loc[inventory.trade_permission & inventory.date.between(start, end), "date"]
    needed = [date for ledger in lookup.values() for column in ("prev_date", "prev2_date")
              for date in ledger.reindex(dates)[column].dropna()]
    first = min(needed)
    normalized = raw.index.normalize()
    raw = raw.loc[(normalized >= first) & (normalized <= end)].copy()
    if not raw.index.is_unique or not raw.index.is_monotonic_increasing:
        raise ValueError("history must be an ordered unique source vintage")
    plans = [(day, *arm, None) for day in dates for arm in ARMS]
    plans += [(day, "IMO_AB_OI_LAG1", "IMO_AB_OI", "mr_A_bo_B", bool(gates[day])) for day in dates if day in gates]
    plans.sort(key=lambda item: (item[0], item[1]))
    expected_ids = [f"{day.date()}/{profile}" for day, profile, *_ in plans]
    header = dict(record_type="header", schema_version="imoexf_component_history_v1",
        evidence_kind="frozen_final_history_not_pit", start=str(start.date()), end=str(end.date()),
        definition=calendar_definition(cal, pd, first, end), model_sha256=MODEL_SHA,
        source_hashes=dict(calendar=CALENDAR_SHA, sessions=SESSIONS_SHA, compiler=COMPILER_SHA, model=MODEL_SHA,
                           component_protocol=PROTOCOL_SHA, oi_io=OI_IO_SHA, oi_protocol=OI_PROTOCOL_SHA,
                           input_study=STUDY_SHA, input_manifest=MANIFEST_SHA,
                           **{name: digest for name, (_, digest) in BOOK_PINS.items()}),
        input_pins=input_pins, history_columns=["ts_utc", "o", "h", "l", "close", "v"],
        bars=[[epoch(ts), float(r.open), float(r.high), float(r.low), float(r.close), float(r.volume)]
              for ts, r in raw.iterrows()], state_columns=STATE_COLUMNS, expected_case_ids=expected_ids)
    totals = dict(cases=0, admitted=0, current_grid_excluded=0, context_hold=0, states=0, trades=0, events=0)
    by_profile, frozen_counts = {}, Counter()
    digest = hashlib.sha256()
    def write(value, hashed=True):
        line = (json.dumps(value, separators=(",", ":"), allow_nan=False)+"\n").encode()
        output.write(line)
        if hashed:
            digest.update(line)
    write(header)
    for day, profile, arm, variant, gate in plans:
        mr = lookup["A"].loc[day]
        bo = lookup["A" if variant == "mr_A_bo_A" else "B"].loc[day]
        mr_inputs, bo_inputs = component_features(mr), component_features(bo)
        dependencies = mr_inputs is not None and bo_inputs is not None
        accepted = bool(mr.current_accepted)
        if bool(mr.ready and bo.ready) != (accepted and dependencies):
            raise ValueError("historical readiness decomposition changed")
        earliest = min(date for date in (mr.prev_date, mr.prev2_date, bo.prev_date, bo.prev2_date) if pd.notna(date))
        cal_day = cal.session(day, "IMOEXF")
        current_range = indices(raw, day+pd.Timedelta(minutes=int(cal_day.open_minute)),
                                day+pd.Timedelta(minutes=int(cal_day.end_minute)))
        case = dict(record_type="case", id=f"{day.date()}/{profile}", date=str(day.date()), arm=arm, gate=gate,
            history_range=indices(raw, earliest, day), current_range=current_range,
            current_accepted=accepted, dependency_ready=dependencies,
            mr_previous=None if pd.isna(mr.prev_date) else str(mr.prev_date.date()),
            mr_previous2=None if pd.isna(mr.prev2_date) else str(mr.prev2_date.date()),
            bo_previous=None if pd.isna(bo.prev_date) else str(bo.prev_date.date()),
            bo_previous2=None if pd.isna(bo.prev2_date) else str(bo.prev2_date.date()),
            mr_inputs=mr_inputs, bo_inputs=bo_inputs, expected_state_trace=[], events=[], trades=[])
        if accepted and dependencies:
            bars = current_bars[day]
            pd.testing.assert_frame_equal(raw.iloc[slice(*current_range)], bars)
            allowed = None if gate is None else {day: gate}
            case.update(replay_reference(pd, build, leg_cost, variant, ledgers, bars, allowed))
        if profile == "IMO_AB_OI_LAG1":
            t = books["oi_trades"]
            t = t.loc[t.profile.eq("IMO_AB") & t.policy.eq("negative_oi_lag1") & t.date.eq(day)]
            e = books["oi_events"]
            e = e.loc[e.profile.eq("IMO_AB") & e.policy.eq("negative_oi_lag1") & e.ts.dt.normalize().eq(day)]
        else:
            t = books["component_trades"]
            t = t.loc[t.instrument.eq("IMOEXF") & t.variant.eq(variant) & t.date.eq(day)]
            e = books["component_events"]
            e = e.loc[e.instrument.eq("IMOEXF") & e.variant.eq(variant) & e.ts.dt.normalize().eq(day)]
        compare_records(pd, case["trades"], stored_trades(t), case["id"]+" trades/costs")
        compare_records(pd, case["events"], stored_events(pd, e), case["id"]+" events")
        frozen_counts[profile] += len(t)
        stats = by_profile.setdefault(profile, dict(cases=0, admitted=0, current_grid_excluded=0,
            context_hold=0, states=0, trades=0, events=0))
        for item in (totals, stats):
            item["cases"] += 1
            item["admitted"] += int(accepted and dependencies)
            item["current_grid_excluded"] += int(not accepted)
            item["context_hold"] += int(accepted and not dependencies)
            for key, source in (("states", "expected_state_trace"), ("trades", "trades"), ("events", "events")):
                item[key] += len(case[source])
        write(case)
        if totals["cases"] % 40 == 0:
            print(json.dumps(dict(processed=totals["cases"], last_case=case["id"])), flush=True)
    # Recheck sources after execution; a partially written output has no valid footer.
    for relative, expected in input_pins.items():
        pinned(research / "moex_component_context_2026_11_15" / relative, expected)
    for _, (relative, expected) in BOOK_PINS.items():
        pinned(research / relative, expected)
    for profile, _, variant in (*ARMS, ("IMO_AB_OI_LAG1", "IMO_AB_OI", "mr_A_bo_B")):
        if profile == "IMO_AB_OI_LAG1":
            t, e = books["oi_trades"], books["oi_events"]
            t = t.loc[t.profile.eq("IMO_AB") & t.policy.eq("negative_oi_lag1")]
            e = e.loc[e.profile.eq("IMO_AB") & e.policy.eq("negative_oi_lag1")]
        else:
            t, e = books["component_trades"], books["component_events"]
            t = t.loc[t.instrument.eq("IMOEXF") & t.variant.eq(variant)]
            e = e.loc[e.instrument.eq("IMOEXF") & e.variant.eq(variant)]
        if (len(t.loc[t.date.between(start, end)]) != by_profile[profile]["trades"]
                or len(e.loc[e.ts.dt.normalize().between(start, end)]) != by_profile[profile]["events"]):
            raise AssertionError(f"frozen events/trades omitted outside planned regular cases: {profile}")
    summary = dict(**totals, by_profile=by_profile, frozen_book_trade_counts=dict(frozen_counts))
    write(dict(record_type="complete", prefix_sha256=digest.hexdigest(), summary=summary), hashed=False)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--research", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="NEW private evidence path, never a public fixture")
    args = parser.parse_args()
    with args.output.open("xb") as output:
        summary = export(args.research, output)
    print(json.dumps(summary), flush=True)


if __name__ == "__main__":
    main()
