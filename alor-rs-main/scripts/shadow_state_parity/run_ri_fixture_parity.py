"""Run an independent, bounded RI synthetic state/action gate. Never deploys."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess

from compare import ParityError, compare_value, file_hash, loads, nullable, validate
from ri_fixtures import SCHEMA, fixtures
from ri_reference import POLICIES, Reference, local

ANCHOR = {"date": str, "high": float, "low": float, "close": float, "range": float, "bars": int}
POS = {"side": str, "entry_ts": str, "entry_price": float, "bars_held": int}
STATE = {
    "context": {"previous": nullable(ANCHOR), "previous2": nullable(ANCHOR)},
    "phase": str, "mr_total": int, "bo_total": int,
    "lifecycle_owner": nullable(str), "lifecycle_side": nullable(str), "lifecycle_entry_ts": nullable(str),
    "mr": nullable({**POS, "prev_close": float, "prev_range": float}), "bo": nullable(POS),
    "pending": nullable({"kind": str, "side": nullable(str), "reason": nullable(str)}),
    "observation_count": int, "first_bar": str, "long_level": nullable(float), "short_level": nullable(float),
    "day_high": float, "day_low": float, "trade_allowed": bool, "long_used": bool, "short_used": bool,
    "branches": [str], "actions": [{"component": str, "role": str, "side": str, "reason": str,
                                    "scheduled_ts": str, "pnl": nullable(float)}],
}
ROW = {"case": str, "policy": str, "index": int, "bar_ts_utc": int, "state": STATE}
BAR = {"symbol": str, "close_time_utc": int, "o": float, "h": float, "l": float, "close": float,
       "v": float, "origin": str}
FIXTURE = {"schema_version": str, "evidence_kind": str,
           "cases": [{"id": str, "day": str, "warmup": [BAR], "events": [BAR]}]}


def expected_rows(f):
    validate(f, FIXTURE)
    if f["schema_version"] != SCHEMA or f["evidence_kind"] != "synthetic_model_clock":
        raise ParityError("synthetic fixture identity mismatch")
    if not 1 <= len(f["cases"]) <= 64 or len({c["id"] for c in f["cases"]}) != len(f["cases"]):
        raise ParityError("empty/duplicate fixture cases")
    rows = []
    for case in f["cases"]:
        if not 1 <= len(case["events"]) <= 1000 or len(case["warmup"]) > 1000:
            raise ParityError("fixture size invalid")
        for section in ("warmup", "events"):
            last = -1
            for b in case[section]:
                dt = local(b["close_time_utc"])
                if (b["symbol"] != "RIZ6" or b["close_time_utc"] <= last or dt.minute % 10 or dt.second
                    or dt.weekday() >= 5 or not "07:00" <= dt.strftime("%H:%M") <= "23:40"
                    or not 0 < b["l"] <= min(b["o"], b["close"]) <= max(b["o"], b["close"]) <= b["h"] or b["v"] < 0):
                    raise ParityError("invalid model input")
                if section == "events" and (dt.date().isoformat() != case["day"] or b["origin"] != "live"):
                    raise ParityError("event outside fixture day/contract")
                if section == "warmup" and (dt.date().isoformat() >= case["day"] or b["origin"] not in ("history", "history_gap")):
                    raise ParityError("invalid warmup")
                last = b["close_time_utc"]
        for policy in POLICIES:
            r = Reference(case["warmup"], case["day"], policy)
            for index, b in enumerate(case["events"]):
                rows.append(dict(case=case["id"], policy=policy, index=index,
                                 bar_ts_utc=b["close_time_utc"], state=r.step(b)))
    return rows


def compare_rows(expected, actual):
    if len(expected) != len(actual):
        raise ParityError("missing/extra state rows")
    for i, (left, right) in enumerate(zip(expected, actual)):
        try:
            validate(left, ROW)
            validate(right, ROW)
            compare_value(left, right, ROW)
        except ParityError as e:
            raise ParityError(f"row={i} case={left['case']} policy={left['policy']} input={left['index']}: {e}") from e


def verify(fixture_path, expected_path, actual_path):
    f = loads(fixture_path.read_text())
    expected = expected_rows(f)
    stored = loads(expected_path.read_text())
    # Coverage is re-derived from the complete input, not from two agreeing tails.
    compare_rows(expected, stored)
    actual = loads(actual_path.read_text())
    validate(actual, {"schema_version": str, "evidence_kind": str, "adapter_version": str,
                      "input_sha256": str, "binary_sha256": str, "rows": [ROW]})
    if (actual["schema_version"] != "ri_prospective_fixture_actual_v1"
        or actual["evidence_kind"] != "synthetic_model_clock"
        or actual["adapter_version"] != "ri_frozen_prospective_shadow_v2"
        or actual["input_sha256"] != file_hash(fixture_path)
        or len(actual["binary_sha256"]) != 64
        or any(c not in "0123456789abcdef" for c in actual["binary_sha256"])):
        raise ParityError("candidate identity/input mismatch")
    compare_rows(expected, actual["rows"])
    return dict(result="PASS_BOUNDED_SYNTHETIC_STATE", cases=len(f["cases"]), policies=list(POLICIES),
                state_rows=len(expected), actions=sum(len(r["state"]["actions"]) for r in expected),
                branch_counts={b:sum(r["state"]["branches"].count(b) for r in expected)
                               for b in sorted({b for r in expected for b in r["state"]["branches"]})},
                historical_parity_certified=False, operational_receipt_parity_certified=False,
                deployment=False, binary_sha256=actual["binary_sha256"])


def write_new(path, payload):
    with path.open("x", encoding="utf-8") as stream:
        json.dump(payload, stream, indent=2, allow_nan=False)
        stream.write("\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="NEW directory, existing parent")
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir()
    fixture_path, expected_path, actual_path = (out/n for n in ("inputs.json", "expected.json", "actual.json"))
    f = fixtures()
    write_new(fixture_path, f)
    write_new(expected_path, expected_rows(f))
    workspace = Path(__file__).resolve().parents[2]
    sources = list(Path(__file__).parent.glob("ri_*.py")) + [Path(__file__), Path(__file__).with_name("compare.py")]
    rust = workspace / "strategy-runtime/src"
    sources += [rust/"strategies/ri_author41_42_live.rs", rust/"strategies/moex_author41_42.rs", rust/"ri_context.rs", rust/"state.rs",
                rust/"strategies/ri_author41_42_live/frozen_shadow.rs", rust/"strategies/ri_author41_42_live/frozen_shadow/tests.rs",
                rust/"strategies/ri_author41_42_live/frozen_shadow/tests/reference.rs", workspace/"Cargo.lock"]
    hashes = {str(p.relative_to(workspace)):file_hash(p) for p in sources}
    env = {k:v for k, v in os.environ.items() if k not in ("REDIS_URL", "RI_SHADOW_REDIS_URL")}
    env.update(RI_STATE_FIXTURE_INPUT=str(fixture_path), RI_STATE_FIXTURE_ACTUAL=str(actual_path))
    cmd = ["cargo", "test", "--offline", "-p", "strategy-runtime", "--lib",
           "ri_export_independent_state_fixture_actual", "--", "--ignored", "--nocapture", "--test-threads=1"]
    run = subprocess.run(cmd, cwd=workspace, env=env, capture_output=True, text=True)
    with (out/"candidate_test.log").open("x") as log:
        log.write(run.stdout+run.stderr)
    if run.returncode:
        raise RuntimeError(f"Rust fixture export failed; see {out/'candidate_test.log'}")
    if hashes != {str(p.relative_to(workspace)):file_hash(p) for p in sources}:
        raise ParityError("sources changed during validation")
    report = verify(fixture_path, expected_path, actual_path)
    report.update(sources=hashes, command=cmd, files={p.name:file_hash(p) for p in (fixture_path, expected_path, actual_path)})
    write_new(out/"verification.json", report)
    print(json.dumps({k:v for k, v in report.items() if k not in ("sources", "files")}, indent=2))


if __name__ == "__main__":
    main()
