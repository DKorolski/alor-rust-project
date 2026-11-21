"""Export small native calendar fixtures from pinned, unmodified research functions.

Requires pandas==2.2.3 and numpy==2.1.3 in a disposable local environment. No downloads, prices,
model execution, network calls or writes to the frozen research package.
"""
from __future__ import annotations

import argparse
import ast
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

CALENDAR_SHA = "76c50e7d7a43a256472b867ea3e0bb092a61c43732c37d93fd9757d51933177d"
SESSIONS_SHA = "5bb6b17a2863b1053750160252f44b21c14eafd33df1787a3cffb8bbf74ec46e"
WINDOWS = (
    ("working_saturday", "2024-04-25", "2024-04-30"),
    ("late_open", "2024-06-12", "2024-06-17"),
    ("morning09", "2025-01-24", "2025-01-28"),
    ("weekend_admission", "2025-08-14", "2025-08-19"),
    ("extended_holiday", "2026-06-10", "2026-06-15"),
    ("morning07", "2026-07-10", "2026-07-20"),
    ("main09", "2026-09-10", "2026-09-15"),
    ("november_holiday", "2026-11-02", "2026-11-09"),
)


def pinned(path, expected):
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != expected:
        raise ValueError(f"frozen reference changed: {path.name}")
    return data


def load_reference(reference):
    import numpy as np
    import pandas as pd

    if pd.__version__ != "2.2.3" or np.__version__ != "2.1.3":
        raise ValueError("use the documented pandas==2.2.3/numpy==2.1.3 reference environment")
    path = reference / "exchange_calendar.py"
    source = pinned(path, CALENDAR_SHA)
    spec = importlib.util.spec_from_file_location("calendar_reference_fixture", path)
    cal = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = cal
    exec(compile(source, str(path), "exec"), cal.__dict__)
    # Execute the exact two pure functions without importing research plotting,
    # model loaders, data caches or their module-level side effects.
    tree = ast.parse(pinned(reference / "session_data.py", SESSIONS_SHA))
    nodes = [n for n in tree.body if isinstance(n, ast.FunctionDef)
             and n.name in ("inspect_sessions", "build_ledger")]
    if len(nodes) != 2:
        raise ValueError("reference function inventory changed")
    env = dict(pd=pd, np=np, session=cal.session)
    exec(compile(ast.Module(body=nodes, type_ignores=[]), "pinned-session-functions", "exec"), env)
    return cal, env, pd


def export(reference):
    cal, env, pd = load_reference(reference)

    def epochs(labels):
        return (labels.tz_localize("Europe/Moscow").tz_convert("UTC").asi8 // 1_000_000_000).tolist()

    cases = []
    for symbol in ("IMOEXF", "USDRUBF"):
        for name, first, last in WINDOWS:
            sessions, labels, frames = [], {}, []
            for day in pd.date_range(first, last):
                s = cal.session(day, symbol)
                active = s.kind in ("REGULAR", "DSVD")
                key = str(day.date())
                sessions.append(dict(date=key, exchange_trade_date=None,
                    kind=s.kind if active else "CLOSED",
                    context_open_minute=s.open_minute if active else None,
                    close_minute=s.end_minute if active else None,
                    main_open_minute=s.main_open if active else None,
                    main_close_minute=s.main_end if active else None,
                    breaks=[dict(start_minute=a, end_minute=b) for a, b in s.breaks],
                    note=s.note or s.kind))
                expected = s.expected_labels()
                labels[key] = epochs(expected)
                frames.append(pd.DataFrame(dict(open=100., high=102., low=98., close=100.,
                                                 origin="live"), index=expected))
            definition = dict(schema_version="moex_shadow_calendar_v1",
                calendar_version="research-frozen-20261115-fixture-v1", reference_sha256=CALENDAR_SHA,
                symbol=symbol, first_date=first, last_date=last, sessions=sessions)
            encoded = json.dumps(definition, separators=(",", ":")).encode()
            pin = dict(artifact_sha256=hashlib.sha256(encoded).hexdigest(),
                reference_sha256=CALENDAR_SHA, calendar_version=definition["calendar_version"],
                symbol=symbol, first_date=first, last_date=last)
            raw = pd.concat(frames).sort_index()
            target = next(s["date"] for s in reversed(sessions)
                          if s["kind"] == "REGULAR" and pd.Timestamp(s["date"]).weekday() < 5)
            for scenario in ("complete", "missing_regular", "incomplete_dsvd"):
                observed = raw.copy()
                kind = "REGULAR" if scenario == "missing_regular" else "DSVD"
                candidates = [s for s in sessions if s["date"] < target and s["kind"] == kind]
                if scenario != "complete":
                    if not candidates:
                        continue
                    day = pd.Timestamp(candidates[-1]["date"])
                    # Remove every row: the reference must retain this scheduled
                    # candidate as unknown rather than selecting an older good day.
                    observed = observed.loc[observed.index.normalize() != day]
                inventory, _ = env["inspect_sessions"](observed, symbol)
                coverage = {str(r.date.date()): bool(r.accepted) for r in inventory.itertuples()}
                selections = []
                for policy in (False, True):
                    ledger = env["build_ledger"](inventory, include_dsvd=policy)
                    chosen = ledger.loc[ledger.date.eq(pd.Timestamp(target))].iloc[0]
                    selections.append(dict(policy="regular_and_dsvd" if policy else "regular_only",
                        previous=None if pd.isna(chosen.prev_date) else str(chosen.prev_date.date()),
                        previous2=None if pd.isna(chosen.prev2_date) else str(chosen.prev2_date.date()),
                        ready=bool(chosen.ready)))
                cases.append(dict(id=f"{symbol}/{name}/{scenario}", definition=definition, pin=pin,
                    target=target, days=[dict(date=s["date"], expected_labels=labels[s["date"]],
                        observed_labels=epochs(observed.loc[observed.index.normalize() ==
                            pd.Timestamp(s["date"])].index),
                        accepted=coverage.get(s["date"], False)) for s in sessions],
                    selections=selections))
    return dict(schema_version="moex_calendar_reference_fixture_v1",
        evidence_kind="pinned_calendar_and_sliced_session_functions_synthetic_coverage_not_model_parity",
        pandas_version=pd.__version__, reference_sha256=CALENDAR_SHA,
        session_functions_sha256=SESSIONS_SHA, cases=cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    payload = export(args.reference)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(payload, output, separators=(",", ":"), allow_nan=False)
        output.write("\n")
    print(json.dumps(dict(cases=len(payload["cases"]), output=str(args.output))))


if __name__ == "__main__":
    main()
