#!/usr/bin/env python3
"""Compare IMOEXF primary Rust replay output with the handoff source package.

The report intentionally separates signal-style exact trade drift from known
BO execution-contract drift. It does not decide whether Rust should emulate
Backtrader fills; it makes the mismatch visible and reviewable.

Rust actual replay stores `pnl_comm` for the full position size. For review
samples, this helper normalizes it to points per contract as `net_points` and
keeps the original full-position value as `actual_pnl_comm`.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import pandas as pd


DEFAULT_MODEL_ID = "hybrid_mr_riskgate_high180_lb120__bo_new_k053"
DEFAULT_SCENARIO = "base_realistic"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--actual", required=True, help="Rust actual_trades_hybrid.csv")
    parser.add_argument("--reference", required=True, help="Source replay_trades.csv")
    parser.add_argument("--model-id", default=DEFAULT_MODEL_ID)
    parser.add_argument("--scenario", default=DEFAULT_SCENARIO)
    parser.add_argument("--out-json", help="Optional path to write the JSON report")
    return parser.parse_args()


def read_actual(path: Path) -> pd.DataFrame:
    actual = pd.read_csv(path, parse_dates=["entry_ts", "exit_ts"])
    actual["family"] = actual["owner"].map(
        {"mean_reversion": "MR", "intraday_breakout": "BO"}
    )
    size_abs = actual["size"].abs().replace(0, pd.NA)
    actual["gross_points"] = actual["pnl"] / size_abs
    actual["net_points"] = actual["pnl_comm"] / size_abs
    actual["actual_pnl_comm"] = actual["pnl_comm"]
    return actual[actual["family"].isin(["MR", "BO"])].copy()


def read_reference(path: Path, model_id: str, scenario: str) -> pd.DataFrame:
    reference = pd.read_csv(path, parse_dates=["entry_ts", "exit_ts"])
    return reference[
        (reference["model_id"] == model_id) & (reference["scenario"] == scenario)
    ].copy()


def add_keys(df: pd.DataFrame) -> pd.DataFrame:
    out = df.copy()
    out["exact_key"] = list(
        zip(
            out["family"].astype(str),
            out["entry_ts"].astype(str),
            out["exit_ts"].astype(str),
            out["side"].astype(str),
            out["entry_price"].round(6),
            out["exit_price"].round(6),
        )
    )
    out["entry_key"] = list(
        zip(
            out["family"].astype(str),
            out["entry_ts"].astype(str),
            out["side"].astype(str),
            out["entry_price"].round(6),
        )
    )
    return out


def sample_rows(df: pd.DataFrame, limit: int = 10) -> list[dict[str, Any]]:
    cols = ["family", "entry_ts", "exit_ts", "side", "entry_price", "exit_price"]
    if "net_points" in df.columns:
        cols.append("net_points")
    if "actual_pnl_comm" in df.columns:
        cols.append("actual_pnl_comm")
    work = df.sort_values("entry_ts").head(limit)[cols].copy()
    for col in ["entry_ts", "exit_ts"]:
        work[col] = work[col].astype(str)
    return work.to_dict(orient="records")


def layer_report(reference: pd.DataFrame, actual: pd.DataFrame, family: str) -> dict[str, Any]:
    ref = reference[reference["family"] == family].copy()
    act = actual[actual["family"] == family].copy()
    ref_exact = set(ref["exact_key"])
    act_exact = set(act["exact_key"])
    ref_entry = set(ref["entry_key"])
    act_entry = set(act["entry_key"])
    missing = ref[~ref["exact_key"].isin(act_exact)]
    extra = act[~act["exact_key"].isin(ref_exact)]
    return {
        "reference": int(len(ref)),
        "actual": int(len(act)),
        "exact_common": int(len(ref_exact & act_exact)),
        "reference_missing_exact": int(len(ref_exact - act_exact)),
        "actual_extra_exact": int(len(act_exact - ref_exact)),
        "entry_common": int(len(ref_entry & act_entry)),
        "reference_missing_entries": int(len(ref_entry - act_entry)),
        "actual_extra_entries": int(len(act_entry - ref_entry)),
        "reference_missing_sample": sample_rows(missing),
        "actual_extra_sample": sample_rows(extra),
    }


def cross_day_report(df: pd.DataFrame) -> dict[str, Any]:
    work = df[df["family"] == "BO"].copy()
    work["cross_day"] = work["entry_ts"].dt.date != work["exit_ts"].dt.date
    cross = work[work["cross_day"]].copy()
    return {
        "count": int(len(cross)),
        "sample": sample_rows(cross, limit=20),
    }


def main() -> None:
    args = parse_args()
    actual = add_keys(read_actual(Path(args.actual)))
    reference = add_keys(read_reference(Path(args.reference), args.model_id, args.scenario))

    report = {
        "model_id": args.model_id,
        "scenario": args.scenario,
        "layers": {
            "MR": layer_report(reference, actual, "MR"),
            "BO": layer_report(reference, actual, "BO"),
        },
        "execution_contract_classes": {
            "bo_cross_day_reference_carry": cross_day_report(reference),
            "bo_gap_flatten_or_cross_day_actual": cross_day_report(actual),
        },
        "interpretation": {
            "mr": "signal_near_parity_review_residuals",
            "bo": "execution_contract_drift_dominates_exact_trade_diff",
        },
    }

    text = json.dumps(report, indent=2, ensure_ascii=False)
    if args.out_json:
        Path(args.out_json).write_text(text + "\n", encoding="utf-8")
    print(text)


if __name__ == "__main__":
    main()
