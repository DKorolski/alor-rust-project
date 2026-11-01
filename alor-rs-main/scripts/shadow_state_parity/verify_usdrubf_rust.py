"""Compare independent Rust states, trades and events against the frozen USDRUBF bundle."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import pandas as pd

from compare import ParityError, compare_traces, file_hash, loads
from export_usdrubf import reference_modules


def verify(handoff, reference, candidate):
    report_path = candidate / "verification.json"
    if report_path.exists():
        raise ParityError("Verification already exists; use a new candidate run directory")
    metadata = loads((candidate / "run_metadata.json").read_text())
    if metadata["status"] != "GENERATED_NOT_YET_COMPARED":
        raise ParityError("Unexpected candidate run status")
    if metadata["raw_file_sha256"] != file_hash(handoff / "raw_model_input.csv"):
        raise ParityError("Candidate input file mismatch")
    with reference_modules(handoff) as (_, _, verifier):
        spec = verifier.integrity()
        model_ids = [p["engine"]["model_id"] for p in spec["profiles"]]
        if [item["model_id"] for item in metadata["checks"]] != model_ids:
            raise ParityError("Candidate profile coverage mismatch")
        checks, trades, events = [], [], []
        for model_id, run in zip(model_ids, metadata["checks"]):
            manifest = reference / model_id / "manifest.json"
            actual = candidate / model_id / "actual_state_trace.jsonl"
            if run["manifest_sha256"] != file_hash(manifest) or run["trace_sha256"] != file_hash(actual):
                raise ParityError("Candidate run provenance mismatch")
            if not run["raw_selection_checked"]:
                raise ParityError("Raw feed selection not checked")
            check = compare_traces(reference / model_id / "expected_state_trace.jsonl.gz", actual, manifest)
            if check["inputs"] != run["prepared_inputs"] or check["trace_rows"] != run["trace_rows"]:
                raise ParityError("Candidate run count mismatch")
            check["model_id"] = model_id
            checks.append(check)
            trades.append(pd.read_csv(candidate / model_id / "trades.csv"))
            events.append(pd.read_csv(candidate / model_id / "events.csv"))
        actual_t, actual_e = pd.concat(trades, ignore_index=True), pd.concat(events, ignore_index=True)
        verifier.assert_parity(pd.read_csv(handoff / "expected_trades.csv"), pd.read_csv(handoff / "expected_events.csv"), actual_t, actual_e)
        report = dict(result="PASS", scope="offline_numerical_state_and_output_parity",
                      trade_rows=len(actual_t), event_rows=len(actual_e), checks=checks,
                      candidate_metadata_sha256=file_hash(candidate / "run_metadata.json"),
                      operational_receipt_restart_certified=False, broker_isolation_certified=False,
                      prospective_warmup_certified=False, deployment=False)
        report_path.write_text(json.dumps(report, indent=2)+"\n")
        return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--handoff", type=Path, required=True)
    parser.add_argument("--reference-state", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(verify(args.handoff, args.reference_state, args.candidate), indent=2))


if __name__ == "__main__":
    main()
