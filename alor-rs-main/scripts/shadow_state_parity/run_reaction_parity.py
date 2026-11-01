"""Compare frozen Python model decisions with Rust frozen-context/restart fixtures."""
from __future__ import annotations

import argparse
from collections import Counter
import json
import os
from pathlib import Path
import subprocess

from compare import ParityError, file_hash, loads
from reaction_compare import compare, project_case, keys
from reaction_fixtures import fixtures, EVIDENCE
from reaction_reference import expected
from run_ri_fixture_parity import write_new


def verify(source, expected_file, actual_file, imo, usd):
    f = loads(source.read_text())
    # Re-derive coverage from versioned inputs, not mutually truncated traces.
    compare(fixtures(), f, "fixture")
    want = expected(f, imo, usd)
    compare(want, loads(expected_file.read_text()), "expected")
    actual = loads(actual_file.read_text())
    keys(actual, "schema_version evidence_kind input_sha256 binary_sha256 cases")
    if (actual["schema_version"] != "weekend_reaction_actual_v1" or actual["evidence_kind"] != EVIDENCE
        or actual["input_sha256"] != file_hash(source) or len(actual["binary_sha256"]) != 64
        or any(c not in "0123456789abcdef" for c in actual["binary_sha256"])):
        raise ParityError("candidate provenance mismatch")
    compare(want, [project_case(c) for c in actual["cases"]], "actual")
    counts = Counter(op["kind"] for c in f["cases"] for op in c["operations"])
    return dict(result="PASS_SYNTHETIC_FROZEN_REACTION_STATE_RESTART", cases=len(want),
        rows=sum(len(c["rows"]) for c in want), operations=dict(counts),
        trade_rows=sum(len(c["rows"][-1]["model"]["trades"]) for c in want),
        event_rows=sum(len(c["rows"][-1]["model"]["events"]) for c in want),
        components=dict(Counter(t["owner"] for c in want for t in c["rows"][-1]["model"]["trades"])),
        independent_context_projection=True, independent_decision_state_projection=True,
        uninstrumented_outputs_equal=True, full_historical_weekend_parity=False,
        fee_economics_parity=False, production_receipts_certified=False, deployment=False,
        binary_sha256=actual["binary_sha256"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--imoexf-handoff", type=Path, required=True)
    parser.add_argument("--usdrubf-handoff", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    for handoff in (args.imoexf_handoff, args.usdrubf_handoff):
        if out.is_relative_to(handoff.resolve()):
            raise ParityError("output cannot be inside an immutable handoff")
    out.mkdir()  # never reuse an existing run
    source, expected_file, actual = (out/n for n in ("inputs.json", "expected.json", "actual.json"))
    f = fixtures()
    for path, value in ((source, f), (expected_file, expected(f, args.imoexf_handoff, args.usdrubf_handoff))):
        with path.open("x") as stream:
            json.dump(value, stream, separators=(",", ":"), allow_nan=False)
            stream.write("\n")
    workspace = Path(__file__).resolve().parents[2]
    paths = list(Path(__file__).parent.glob("*.py")) + list((workspace/"strategy-runtime/src").rglob("*.rs"))
    paths += [workspace/"strategy-runtime/Cargo.toml", workspace/"Cargo.lock"]
    hashes = {str(p.relative_to(workspace)): file_hash(p) for p in paths}
    env = {k:v for k, v in os.environ.items() if k not in ("REDIS_URL", "RI_SHADOW_REDIS_URL")}
    env.update(REACTION_FIXTURE_INPUT=str(source), REACTION_FIXTURE_ACTUAL=str(actual))
    cmd = ["cargo", "test", "--offline", "-p", "strategy-runtime", "--lib", "weekend_reaction_export_actual",
           "--", "--ignored", "--nocapture", "--test-threads=1"]
    run = subprocess.run(cmd, cwd=workspace, env=env, capture_output=True, text=True, timeout=600)
    with (out/"candidate_test.log").open("x") as log:
        log.write(run.stdout+run.stderr)
    if run.returncode:
        raise RuntimeError(f"candidate failed: {out/'candidate_test.log'}")
    if hashes != {str(p.relative_to(workspace)): file_hash(p) for p in paths}:
        raise ParityError("sources changed during run")
    try:
        report = verify(source, expected_file, actual, args.imoexf_handoff, args.usdrubf_handoff)
    except ParityError as error:
        write_new(out/"mismatch.json", dict(result="FAIL_REACTION_COMPARISON", error=str(error), deployment=False))
        raise
    report.update(sources=hashes, command=cmd, files={p.name:file_hash(p) for p in (source, expected_file, actual)},
        references={name:file_hash(path/"manifest.json") for name, path in
                    (("imoexf", args.imoexf_handoff), ("usdrubf", args.usdrubf_handoff))})
    write_new(out/"verification.json", report)
    print({k:v for k, v in report.items() if k not in ("sources", "files")})


if __name__ == "__main__":
    main()
