"""Independent historical RI numerical state gate; never deploys or writes Redis."""
from __future__ import annotations

import argparse
from collections import Counter
import csv
import os
from pathlib import Path
import subprocess

from audit_ri_handoff import audit, inside
from compare import ParityError, file_hash, loads, validate
from ri_history import EVIDENCE, ROW, SCHEMA, compare_rows, expected_rows
from ri_reference import POLICIES, local
from run_ri_fixture_parity import write_new

ACTIVATION = {"RIU6": "2026-07-03", "RIZ6": "2026-09-03"}


def build(handoff):
    preflight = audit(handoff)
    if preflight["artifact_integrity"] != "PASS":
        raise ParityError(str(preflight["errors"]))
    manifest = loads((handoff/"results/fixture_manifest.json").read_text())
    streams = []
    for symbol, first in ACTIVATION.items():
        entry = next(e for e in manifest if e["symbol"] == symbol)
        path = inside(handoff.parent, entry["input_path"])
        with path.open(newline="") as source:
            inputs = [{"bar": dict(symbol=symbol, close_time_utc=int(r["epoch"]),
                                   o=float(r["open"]), h=float(r["high"]), l=float(r["low"]),
                                   close=float(r["close"]), v=float(r["volume"]), origin=r["origin"]),
                       "available_epoch": int(r["available_epoch"])} for r in csv.DictReader(source)]
        streams.append(dict(symbol=symbol, first_decision_session=first,
                            source_sha256=file_hash(path), inputs=inputs))
    return dict(schema_version=SCHEMA, evidence_kind=EVIDENCE, streams=streams), preflight


def summarize(rows):
    summaries = []
    for symbol in sorted({r["symbol"] for r in rows}):
        for policy in POLICIES:
            selected = [r for r in rows if r["symbol"] == symbol and r["policy"] == policy]
            observed = [r for r in selected if r["observed"]]
            actions = [a for r in selected if r["state"] for a in r["state"]["actions"]]
            summaries.append(dict(symbol=symbol, policy=policy, receipts=len(selected),
                observed_states=len(observed), first_session=local(observed[0]["bar_ts_utc"]).date().isoformat(),
                last_session=local(observed[-1]["bar_ts_utc"]).date().isoformat(),
                sessions=len({local(r["bar_ts_utc"]).date() for r in observed}),
                routes=dict(Counter(r["route"] for r in selected)),
                branches=dict(Counter(b for r in selected if r["state"] for b in r["state"]["branches"])),
                actions=len(actions), components={c: dict(
                    entries=sum(a["component"] == c and a["role"] == "entry" for a in actions),
                    exits=sum(a["component"] == c and a["role"] == "exit" for a in actions),
                    diagnostic_pnl_points=sum(a["pnl"] for a in actions if a["component"] == c and a["pnl"] is not None))
                    for c in ("author41_mr", "author42_bo")}))
    return summaries


def legacy_action_compatibility(handoff, rows):
    """Secondary scoped regression, NEVER the source of expected state/actions."""
    reports = []
    for symbol, start in ACTIVATION.items():
        path = handoff/f"results/{symbol}_expected_corrected_actions.jsonl"
        original = [loads(line) for line in path.read_text().splitlines()]
        projected = [dict(component=a["component"], role=a["role"], side=a["side"],
                          reason=a["entry_exit_reason"], scheduled_ts=a["candidate_scheduled_ts_local"],
                          pnl=a["shadow_pnl_points"]) for a in original]
        scoped = [a for a in projected if a["scheduled_ts"][:10] >= start]
        actual = [a for r in rows if r["symbol"] == symbol and r["policy"] == POLICIES[1] and r["state"]
                  for a in r["state"]["actions"]]
        reports.append(dict(symbol=symbol, first_decision_session=start,
                            result="PASS_SCOPED_ACTION_PROJECTION" if scoped == actual else "MISMATCH",
                            legacy_full_actions=len(original), scoped_actions=len(scoped), actual_actions=len(actual),
                            excluded_before_activation=[a for a in projected if a["scheduled_ts"][:10] < start],
                            source_sha256=file_hash(path), independent_state_oracle=False))
    return reports


def verify(source, expected_path, actual_path):
    f = loads(source.read_text())
    expected = expected_rows(f)
    compare_rows(expected, loads(expected_path.read_text()))
    actual = loads(actual_path.read_text())
    validate(actual, {"schema_version": str, "evidence_kind": str, "adapter_version": str,
                      "input_sha256": str, "binary_sha256": str, "recovery_verified": bool, "rows": [ROW]})
    if (actual["schema_version"] != "ri_numerical_history_actual_v1" or actual["evidence_kind"] != EVIDENCE
        or actual["adapter_version"] != "ri_frozen_prospective_shadow_v2"
        or actual["input_sha256"] != file_hash(source) or not actual["recovery_verified"]
        or len(actual["binary_sha256"]) != 64 or any(c not in "0123456789abcdef" for c in actual["binary_sha256"])):
        raise ParityError("historical actual identity mismatch")
    compare_rows(expected, actual["rows"])
    return dict(result="PASS_HISTORICAL_NUMERICAL_STATE", receipt_rows=len(expected),
                observed_states=sum(r["observed"] for r in expected), summaries=summarize(expected),
                historical_numerical_parity_certified=True, original_receipt_parity_certified=False,
                broker_execution_parity_certified=False, old_handoff_actions_certified=False,
                no_overnight_verified=True, numerical_recovery_verified=True, deployment=False,
                binary_sha256=actual["binary_sha256"], evidence_kind=EVIDENCE,
                cost_contract="MR diagnostic 2 points; BO diagnostic 0; NOT broker net PnL")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--handoff", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True, help="NEW directory outside frozen handoff")
    args = p.parse_args()
    handoff, out = args.handoff.resolve(), args.output.resolve()
    if out.is_relative_to(handoff.parent):
        p.error("write derivative reports outside the original revalidation package")
    f, preflight = build(handoff)
    out.mkdir()
    source, expected, actual = (out/n for n in ("inputs.json", "expected.json", "actual.json"))
    write_new(source, f)
    write_new(out/"preflight.json", preflight)
    write_new(expected, expected_rows(f))
    workspace = Path(__file__).resolve().parents[2]
    sources = list(Path(__file__).parent.glob("*.py")) + list((workspace/"strategy-runtime/src").rglob("*.rs"))
    sources += [workspace/"Cargo.lock", workspace/"Cargo.toml", workspace/"strategy-runtime/Cargo.toml"]
    hashes = {str(path.relative_to(workspace)): file_hash(path) for path in sources}
    env = {k:v for k, v in os.environ.items() if k not in ("REDIS_URL", "RI_SHADOW_REDIS_URL")}
    env.update(RI_HISTORY_INPUT=str(source), RI_HISTORY_ACTUAL=str(actual))
    cmd = ["cargo", "test", "--offline", "--release", "-p", "strategy-runtime", "--lib",
           "ri_export_independent_history_actual", "--", "--ignored", "--nocapture", "--test-threads=1"]
    run = subprocess.run(cmd, cwd=workspace, env=env, capture_output=True, text=True, timeout=1800)
    with (out/"candidate_test.log").open("x") as log:
        log.write(run.stdout+run.stderr)
    if run.returncode:
        raise RuntimeError(f"Rust numerical export failed: {out/'candidate_test.log'}")
    if hashes != {str(path.relative_to(workspace)): file_hash(path) for path in sources}:
        raise ParityError("sources changed during numerical run")
    # Verify frozen source hashes/evidence again; derivative input is never authority.
    current, current_preflight = build(handoff)
    if f != current or preflight != current_preflight:
        raise ParityError("handoff changed during numerical run")
    try:
        report = verify(source, expected, actual)
    except ParityError as error:
        write_new(out/"mismatch.json", dict(result="FAIL_NUMERICAL_COMPARISON", error=str(error), deployment=False))
        raise
    report["secondary_legacy_action_check"] = legacy_action_compatibility(handoff, loads(actual.read_text())["rows"])
    report.update(sources=hashes, command=cmd, files={path.name:file_hash(path) for path in (source, expected, actual)})
    write_new(out/"verification.json", report)
    print({k:v for k, v in report.items() if k not in ("summaries", "sources", "files")})


if __name__ == "__main__":
    main()
