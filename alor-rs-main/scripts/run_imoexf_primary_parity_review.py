#!/usr/bin/env python3
"""Run the full IMOEXF primary parity review pipeline.

This is the reproducible wrapper for the review gate:

1. build the filtered model-feed bundle from raw/audit data;
2. run Rust `hybrid_replay --profile imoexf_primary_riskgate_k053`;
3. run layer, MR residual, and BO execution-contract diagnostics;
4. write one consolidated markdown parity review report.

The script intentionally defaults to a `target/` work directory so generated CSV
and JSON artifacts do not get mixed into source control by accident.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


PROFILE = "imoexf_primary_riskgate_k053"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw", required=True, help="Raw/audit CSV or parquet bars")
    parser.add_argument("--reference", required=True, help="Saved source replay_trades.csv")
    parser.add_argument(
        "--source-mr",
        required=True,
        help="Saved source imoexf_mr_execution_economics_strategy_trades.csv",
    )
    parser.add_argument(
        "--work-dir",
        default="target/imoexf-primary-parity-review",
        help="Directory for generated bundle, replay, JSON diagnostics, and report",
    )
    parser.add_argument(
        "--report",
        help="Optional markdown report path. Defaults to <work-dir>/imoexf-primary-parity-review.md",
    )
    parser.add_argument(
        "--accept-bo-gap-flatten",
        action="store_true",
        help="Mark Rust bo_gap_flatten as accepted runtime contract in the report.",
    )
    return parser.parse_args()


def run(cmd: list[str], cwd: Path) -> None:
    print("+ " + " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=cwd, check=True)


def main() -> None:
    args = parse_args()
    root = Path(__file__).resolve().parents[1]
    scripts = root / "scripts"
    work_dir = Path(args.work_dir)
    if not work_dir.is_absolute():
        work_dir = root / work_dir
    work_dir.mkdir(parents=True, exist_ok=True)

    bundle_dir = work_dir / "bundle"
    replay_dir = work_dir / "replay"
    parity_json = work_dir / "imoexf_primary_parity_diff.json"
    mr_json = work_dir / "imoexf_mr_residual_diagnostic.json"
    bo_json = work_dir / "imoexf_bo_execution_contract_diagnostic.json"
    report = Path(args.report) if args.report else work_dir / "imoexf-primary-parity-review.md"
    if not report.is_absolute():
        report = root / report
    report.parent.mkdir(parents=True, exist_ok=True)

    run(
        [
            sys.executable,
            str(scripts / "build_imoexf_filtered_bundle.py"),
            "--raw",
            args.raw,
            "--out-dir",
            str(bundle_dir),
        ],
        cwd=root,
    )
    run(
        [
            "cargo",
            "run",
            "-p",
            "strategy-runtime",
            "--bin",
            "hybrid_replay",
            "--",
            "--bundle-dir",
            str(bundle_dir),
            "--split",
            "hybrid",
            "--out-dir",
            str(replay_dir),
            "--profile",
            PROFILE,
            "--assert-gap-flatten",
        ],
        cwd=root,
    )

    actual = replay_dir / "actual_trades_hybrid.csv"
    run(
        [
            sys.executable,
            str(scripts / "imoexf_primary_parity_diff.py"),
            "--actual",
            str(actual),
            "--reference",
            args.reference,
            "--out-json",
            str(parity_json),
        ],
        cwd=root,
    )
    run(
        [
            sys.executable,
            str(scripts / "imoexf_mr_residual_diagnostic.py"),
            "--actual",
            str(actual),
            "--reference",
            args.reference,
            "--source-mr",
            args.source_mr,
            "--raw",
            args.raw,
            "--out-json",
            str(mr_json),
        ],
        cwd=root,
    )
    run(
        [
            sys.executable,
            str(scripts / "imoexf_bo_execution_contract_diagnostic.py"),
            "--actual",
            str(actual),
            "--reference",
            args.reference,
            "--out-json",
            str(bo_json),
        ],
        cwd=root,
    )

    report_cmd = [
        sys.executable,
        str(scripts / "imoexf_write_parity_review_report.py"),
        "--parity-json",
        str(parity_json),
        "--mr-json",
        str(mr_json),
        "--bo-json",
        str(bo_json),
        "--bundle-metadata-json",
        str(bundle_dir / "metadata.json"),
        "--out-md",
        str(report),
    ]
    if args.accept_bo_gap_flatten:
        report_cmd.append("--accept-bo-gap-flatten")
    run(report_cmd, cwd=root)

    print("\nGenerated IMOEXF parity review artifacts:")
    print(f"- bundle: {bundle_dir}")
    print(f"- replay: {replay_dir}")
    print(f"- parity JSON: {parity_json}")
    print(f"- MR diagnostic JSON: {mr_json}")
    print(f"- BO diagnostic JSON: {bo_json}")
    print(f"- report: {report}")


if __name__ == "__main__":
    main()
