"""Compare synthetic context provenance/freeze/restart, NOT strategy trade parity."""
from __future__ import annotations

import argparse
from collections import Counter
import json
import os
from pathlib import Path
import subprocess

from compare import ParityError, compare_value, file_hash, loads, nullable, validate
from run_ri_fixture_parity import write_new
from weekend_fixtures import fixtures
from weekend_reference import STATE, expected_rows

ROW = {"case": str, "index": int, "route": nullable(dict), "state": STATE}


def compare_rows(expected, actual):
    if len(expected) != len(actual):
        raise ParityError("missing/extra context rows")
    for n, (left, right) in enumerate(zip(expected, actual)):
        try:
            validate(left, ROW)
            validate(right, ROW)
            compare_value(left, right, ROW)
        except ParityError as e:
            raise ParityError(f"row={n} case={left['case']} operation={left['index']}: {e}") from e


def verify(source, expected_path, actual_path):
    f = loads(source.read_text())
    expected = expected_rows(f)
    compare_rows(expected, loads(expected_path.read_text()))
    actual = loads(actual_path.read_text())
    validate(actual, {"schema_version": str, "evidence_kind": str, "version": str,
                      "input_sha256": str, "binary_sha256": str, "rows": [ROW]})
    if (actual["schema_version"] != "weekend_context_actual_v1" or actual["evidence_kind"] != f["evidence_kind"]
        or actual["version"] != "weekend_context_cutoff_v1" or actual["input_sha256"] != file_hash(source)
        or len(actual["binary_sha256"]) != 64 or any(c not in "0123456789abcdef" for c in actual["binary_sha256"])):
        raise ParityError("context actual identity mismatch")
    compare_rows(expected, actual["rows"])
    return dict(result="PASS_SYNTHETIC_CONTEXT_CUTOFF_RESTART", cases=len(f["cases"]), state_rows=len(expected),
                operations=dict(Counter(o["kind"] for c in f["cases"] for o in c["operations"])),
                route_counts=dict(Counter((r["route"].get("reason") or r["route"]["kind"]) for r in expected if r["route"])),
                binary_sha256=actual["binary_sha256"], context_projection_parity=True,
                strategy_decisions_parity=False, full_historical_weekend_parity=False,
                production_receipts_certified=False, deployment=False)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output", type=Path, required=True, help="NEW derivative directory, existing parent")
    args = p.parse_args()
    out = args.output.resolve()
    out.mkdir()
    source, expected, actual = (out/n for n in ("inputs.json", "expected.json", "actual.json"))
    f = fixtures()
    write_new(source, f)
    with expected.open("x", encoding="utf-8") as stream:
        json.dump(expected_rows(f), stream, separators=(",", ":"), allow_nan=False)
        stream.write("\n")
    workspace = Path(__file__).resolve().parents[2]
    paths = list(Path(__file__).parent.glob("*weekend*.py")) + [Path(__file__).with_name("compare.py"),
        Path(__file__).with_name("ri_fixtures.py"), Path(__file__).with_name("ri_reference.py"),
        Path(__file__).with_name("run_ri_fixture_parity.py"), workspace/"Cargo.lock", workspace/"strategy-runtime/Cargo.toml"]
    paths += list((workspace/"strategy-runtime/src").rglob("*.rs"))
    hashes = {str(path.relative_to(workspace)):file_hash(path) for path in paths}
    env = {k:v for k, v in os.environ.items() if k not in ("REDIS_URL", "RI_SHADOW_REDIS_URL")}
    env.update(WEEKEND_FIXTURE_INPUT=str(source), WEEKEND_FIXTURE_ACTUAL=str(actual))
    cmd = ["cargo", "test", "--offline", "-p", "strategy-runtime", "--lib", "weekend_context_export_actual",
           "--", "--ignored", "--nocapture", "--test-threads=1"]
    run = subprocess.run(cmd, cwd=workspace, env=env, capture_output=True, text=True, timeout=600)
    with (out/"candidate_test.log").open("x") as log:
        log.write(run.stdout+run.stderr)
    if run.returncode:
        raise RuntimeError(f"Rust context fixture failed: {out/'candidate_test.log'}")
    if hashes != {str(path.relative_to(workspace)):file_hash(path) for path in paths}:
        raise ParityError("sources changed during verification")
    try:
        report = verify(source, expected, actual)
    except ParityError as error:
        write_new(out/"mismatch.json", dict(result="FAIL_CONTEXT_COMPARISON", error=str(error), deployment=False))
        raise
    report.update(sources=hashes, command=cmd, files={path.name:file_hash(path) for path in (source, expected, actual)})
    write_new(out/"verification.json", report)
    print({k:v for k, v in report.items() if k not in ("sources", "files")})


if __name__ == "__main__":
    main()
