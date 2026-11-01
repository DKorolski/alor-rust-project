"""Independent historical weekend-pair state/restart comparison; offline only."""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from contextlib import ExitStack
import csv
import json
import os
from pathlib import Path
import subprocess

from compare import ParityError, file_hash, loads
from export_imoexf import reference_modules as imo_modules
from export_usdrubf import reference_modules as usd_modules
from reaction_compare import compare, keys, project_case
from reaction_history import HistoryPlan, EVIDENCE, CLOCK, START, END, model_label
from run_ri_fixture_parity import write_new


def compact(path, value):
    with path.open("x") as stream:
        json.dump(value, stream, separators=(",", ":"), allow_nan=False)
        stream.write("\n")


def verify_chunk(plan, days, source, expected_file, actual_file, modules):
    inputs = loads(source.read_text())
    expected = plan.expected(inputs, days, modules)
    compare(expected, loads(expected_file.read_text()), "saved_expected")
    actual = loads(actual_file.read_text())
    keys(actual, "schema_version evidence_kind input_sha256 binary_sha256 cases")
    if (actual["schema_version"] != "weekend_reaction_actual_v1" or actual["evidence_kind"] != EVIDENCE
        or actual["input_sha256"] != file_hash(source) or len(actual["binary_sha256"]) != 64
        or any(c not in "0123456789abcdef" for c in actual["binary_sha256"])):
        raise ParityError("historical candidate provenance mismatch")
    compare(expected, [project_case(c) for c in actual["cases"]], "historical_actual")
    return expected, actual["binary_sha256"]


def summaries(expected):
    daily, trades = [], []
    for case in expected:
        state, cfg = case["rows"][-1]["model"], case["config"]
        meta = dict(model=model_label(cfg), policy=case["identity"]["policy"], day=case["frozen"]["session_date"])
        for owner in ("MR", "BO"):
            part = [t for t in state["trades"] if t["owner"] == owner]
            daily.append(dict(**meta, component=owner, rounds=len(part), gross_points=sum(t["gross_points"] for t in part)))
        for trade in state["trades"]:
            if (trade["entry"][:10] != meta["day"] or trade["exit"][:10] != meta["day"]
                or trade["entry"] > trade["exit"]):
                raise ParityError("cross-day or time-reversed historical round")
            trades.append(dict(**meta, **trade))
    return daily, trades


def csv_new(path, rows):
    with path.open("x", newline="") as stream:
        if rows:
            writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)


def source_hashes(workspace):
    paths = list(Path(__file__).parent.glob("*.py")) + list((workspace/"strategy-runtime/src").rglob("*.rs"))
    paths += [workspace/"Cargo.lock", workspace/"strategy-runtime/Cargo.toml"]
    return {str(p.relative_to(workspace)):file_hash(p) for p in sorted(paths)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--imoexf-raw", type=Path, required=True)
    parser.add_argument("--imoexf-handoff", type=Path, required=True)
    parser.add_argument("--usdrubf-handoff", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--start", default=START)
    parser.add_argument("--end", default=END)
    args = parser.parse_args()
    out = args.output.resolve()
    for path in (args.imoexf_handoff, args.usdrubf_handoff, args.imoexf_raw.parent):
        if out.is_relative_to(path.resolve()):
            raise ParityError("output cannot be inside immutable input directories")
    workspace = Path(__file__).resolve().parents[2]
    hashes = source_hashes(workspace)
    references = {str((p/"manifest.json").resolve()):file_hash(p/"manifest.json")
                  for p in (args.imoexf_handoff, args.usdrubf_handoff)}
    # Validate frozen references and all selected days BEFORE starting the run.
    with ExitStack() as stack:
        modules = {"imoexf":stack.enter_context(imo_modules(args.imoexf_handoff)),
                   "usdrubf":stack.enter_context(usd_modules(args.usdrubf_handoff))}
        plan = HistoryPlan(args.imoexf_raw, args.usdrubf_handoff, args.start, args.end)
        out.mkdir()
        write_new(out/"inventory.json", {name:p.inventory for name, p in plan.prices.items()})
        write_new(out/"run_contract.json", dict(evidence=EVIDENCE, clock=CLOCK, dates=plan.dates,
            configs=plan.configs, inputs=plan.inputs, references=references, sources=hashes,
            fees_compared=False, multi_session_runner_certified=False, deployment=False))
        stats, daily, trades, chunks, binaries = Counter(), [], [], [], set()
        cmd = ["cargo", "test", "--offline", "-p", "strategy-runtime", "--lib",
               "weekend_reaction_historical_export_actual", "--", "--ignored", "--nocapture", "--test-threads=1"]
        try:
            for offset in range(0, len(plan.dates), 2):
                days = plan.dates[offset:offset+2]
                directory = out/f"chunk_{offset//2:03}"
                directory.mkdir()
                source, want, actual = (directory/name for name in ("inputs.json", "expected.json", "actual.json"))
                f = plan.fixture(days)
                compact(source, f)
                compact(want, plan.expected(f, days, modules))
                env = {k:v for k, v in os.environ.items() if k not in ("REDIS_URL", "RI_SHADOW_REDIS_URL")}
                env.update(REACTION_FIXTURE_INPUT=str(source), REACTION_FIXTURE_ACTUAL=str(actual))
                run = subprocess.run(cmd, cwd=workspace, env=env, capture_output=True, text=True, timeout=600)
                with (directory/"candidate_test.log").open("x") as log:
                    log.write(run.stdout+run.stderr)
                if run.returncode:
                    raise ParityError(f"candidate failed: {directory/'candidate_test.log'}")
                expected, binary = verify_chunk(plan, days, source, want, actual, modules)
                binaries.add(binary)
                d, t = summaries(expected)
                daily.extend(d)
                trades.extend(t)
                stats.update(cases=len(expected), state_rows=sum(len(c["rows"]) for c in expected),
                             rounds=len(t), events=sum(len(c["rows"][-1]["model"]["events"]) for c in expected))
                stats.update(Counter(op["kind"] for c in f["cases"] for op in c["operations"]))
                chunks.append(dict(days=days, files={str(p.relative_to(out)):file_hash(p) for p in (source, want, actual)}))
                print(f"PASS {days[0]}..{days[-1]}: {len(expected)} cases, {len(t)} alternative rounds", flush=True)
            if len(binaries) != 1 or hashes != source_hashes(workspace):
                raise ParityError("candidate binary/source changed during historical run")
            for path, digest in {**plan.inputs, **references}.items():
                if file_hash(Path(path)) != digest:
                    raise ParityError("historical input/reference changed during run")
            if stats["cases"] != len(plan.dates)*14:
                raise ParityError("missing model/policy/date coverage")
        except Exception as error:
            write_new(out/"failure.json", dict(result="FAIL_HISTORICAL_REACTION", error=str(error),
                                               completed_chunks=len(chunks), deployment=False))
            raise
    csv_new(out/"daily_gross_points.csv", daily)
    csv_new(out/"trades.csv", trades)
    grouped = defaultdict(lambda: dict(sessions=0, rounds=0, gross_points=0.))
    for row in daily:
        acc = grouped[(row["model"], row["policy"], row["component"])]
        acc["sessions"] += 1
        acc["rounds"] += row["rounds"]
        acc["gross_points"] += row["gross_points"]
    csv_new(out/"component_summary.csv", [dict(model=k[0], policy=k[1], component=k[2], **v) for k, v in grouped.items()])
    report = dict(result="PASS_HISTORICAL_FROZEN_REACTION_STATE_RESTART", stats=dict(stats),
        date_start=plan.dates[0], date_end=plan.dates[-1], regular_sessions=len(plan.dates),
        full_declared_window=(args.start, args.end) == (START, END), evidence_kind=EVIDENCE, clock=CLOCK,
        independent_context_projection=True, full_raw_anchor_crosscheck=True,
        independent_decision_state_projection=True, uninstrumented_outputs_equal=True,
        fee_economics_parity=False, production_receipts_certified=False,
        multi_session_runner_certified=False, deployment=False, automatic_promotion=False,
        components=dict(Counter(t["owner"] for t in trades)), binary_sha256=binaries.pop(),
        sources=hashes, references=references, inputs=plan.inputs, command=cmd, chunks=chunks,
        outputs={p.name:file_hash(p) for p in out.glob("*.csv")})
    write_new(out/"verification.json", report)
    print({k:v for k, v in report.items() if k not in ("sources", "references", "inputs", "chunks", "outputs")})


if __name__ == "__main__":
    main()
