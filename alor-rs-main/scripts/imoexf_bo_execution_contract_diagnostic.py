#!/usr/bin/env python3
"""Diagnose IMOEXF BO execution-contract drift.

The saved source reference records Backtrader-style next-bar fills, while the
Rust replay records close-bar event-loop actions. This helper applies a simple
source timestamp normalization (`source_ts - 10m` by default) to estimate signal
level agreement separately from exact fill-level agreement.
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
    parser.add_argument("--reference", required=True, help="Saved source replay_trades.csv")
    parser.add_argument("--model-id", default=DEFAULT_MODEL_ID)
    parser.add_argument("--scenario", default=DEFAULT_SCENARIO)
    parser.add_argument(
        "--source-next-bar-minutes",
        type=int,
        default=10,
        help="Minutes to subtract from source timestamps to approximate signal time",
    )
    parser.add_argument("--out-json", help="Optional path to write the JSON report")
    return parser.parse_args()


def read_actual(path: Path) -> pd.DataFrame:
    actual = pd.read_csv(path, parse_dates=["entry_ts", "exit_ts"])
    actual["family"] = actual["owner"].map(
        {"mean_reversion": "MR", "intraday_breakout": "BO"}
    )
    actual["side"] = actual["side"].astype(str).str.lower()
    return actual[actual["family"] == "BO"].copy()


def read_reference(path: Path, model_id: str, scenario: str) -> pd.DataFrame:
    reference = pd.read_csv(path, parse_dates=["entry_ts", "exit_ts"])
    out = reference[
        (reference["model_id"] == model_id)
        & (reference["scenario"] == scenario)
        & (reference["family"] == "BO")
    ].copy()
    out["side"] = out["side"].astype(str).str.lower()
    return out


def add_fill_keys(df: pd.DataFrame) -> pd.DataFrame:
    out = df.copy()
    out["fill_key"] = list(
        zip(
            out["entry_ts"].astype(str),
            out["exit_ts"].astype(str),
            out["side"].astype(str),
            out["entry_price"].round(6),
            out["exit_price"].round(6),
        )
    )
    return out


def add_signal_keys(reference: pd.DataFrame, actual: pd.DataFrame, shift_min: int) -> tuple[pd.DataFrame, pd.DataFrame]:
    ref = reference.copy()
    act = actual.copy()
    shift = pd.Timedelta(minutes=shift_min)
    ref["entry_signal_ts"] = ref["entry_ts"] - shift
    ref["exit_signal_ts"] = ref["exit_ts"] - shift
    act["entry_signal_ts"] = act["entry_ts"]
    act["exit_signal_ts"] = act["exit_ts"]

    for df in [ref, act]:
        df["entry_signal_key"] = list(
            zip(df["entry_signal_ts"].astype(str), df["side"].astype(str))
        )
        df["entry_exit_signal_key"] = list(
            zip(
                df["entry_signal_ts"].astype(str),
                df["exit_signal_ts"].astype(str),
                df["side"].astype(str),
            )
        )
        df["date_side_key"] = list(
            zip(df["entry_signal_ts"].dt.date.astype(str), df["side"].astype(str))
        )
    return ref, act


def sample_rows(df: pd.DataFrame, limit: int = 12) -> list[dict[str, Any]]:
    cols = [
        "entry_ts",
        "exit_ts",
        "side",
        "entry_price",
        "exit_price",
    ]
    for optional in ["entry_signal_ts", "exit_signal_ts", "root_cause"]:
        if optional in df.columns:
            cols.append(optional)
    work = df.sort_values("entry_ts").head(limit)[cols].copy()
    for col in ["entry_ts", "exit_ts", "entry_signal_ts", "exit_signal_ts"]:
        if col in work.columns:
            work[col] = work[col].astype(str)
    return work.to_dict(orient="records")


def compare_sets(ref: pd.DataFrame, act: pd.DataFrame, key_col: str) -> dict[str, int]:
    ref_keys = set(ref[key_col])
    act_keys = set(act[key_col])
    return {
        "reference": int(len(ref_keys)),
        "actual": int(len(act_keys)),
        "common": int(len(ref_keys & act_keys)),
        "reference_missing": int(len(ref_keys - act_keys)),
        "actual_extra": int(len(act_keys - ref_keys)),
    }


def date_side_diffs(ref: pd.DataFrame, act: pd.DataFrame) -> list[dict[str, Any]]:
    ref_counts = ref.groupby("date_side_key").size()
    act_counts = act.groupby("date_side_key").size()
    rows = []
    for key in sorted(set(ref_counts.index) | set(act_counts.index)):
        reference = int(ref_counts.get(key, 0))
        actual = int(act_counts.get(key, 0))
        if reference != actual:
            date, side = key
            rows.append(
                {
                    "date": date,
                    "side": side,
                    "reference": reference,
                    "actual": actual,
                }
            )
    return rows


def cross_day(df: pd.DataFrame) -> pd.DataFrame:
    return df[df["entry_ts"].dt.date != df["exit_ts"].dt.date].copy()


def time_counts(df: pd.DataFrame, col: str) -> dict[str, int]:
    return {
        str(k): int(v)
        for k, v in df[col].dt.strftime("%H:%M").value_counts().head(12).items()
    }


def classify_missing_extra(ref: pd.DataFrame, act: pd.DataFrame) -> dict[str, Any]:
    ref_entry_keys = set(ref["entry_signal_key"])
    act_entry_keys = set(act["entry_signal_key"])
    missing = ref[~ref["entry_signal_key"].isin(act_entry_keys)].copy()
    extra = act[~act["entry_signal_key"].isin(ref_entry_keys)].copy()

    missing["root_cause"] = "source_signal_missing_after_shift"
    extra["root_cause"] = "rust_signal_extra_after_shift"

    return {
        "missing_count": int(len(missing)),
        "extra_count": int(len(extra)),
        "missing_sample": sample_rows(missing),
        "extra_sample": sample_rows(extra),
    }


def main() -> None:
    args = parse_args()
    reference = add_fill_keys(read_reference(Path(args.reference), args.model_id, args.scenario))
    actual = add_fill_keys(read_actual(Path(args.actual)))
    reference, actual = add_signal_keys(reference, actual, args.source_next_bar_minutes)

    ref_fill = set(reference["fill_key"])
    act_fill = set(actual["fill_key"])

    report = {
        "model_id": args.model_id,
        "scenario": args.scenario,
        "source_next_bar_minutes": args.source_next_bar_minutes,
        "fill_level": {
            "reference": int(len(reference)),
            "actual": int(len(actual)),
            "exact_common": int(len(ref_fill & act_fill)),
            "reference_missing_exact": int(len(ref_fill - act_fill)),
            "actual_extra_exact": int(len(act_fill - ref_fill)),
        },
        "signal_level": {
            "entry_signal": compare_sets(reference, actual, "entry_signal_key"),
            "entry_exit_signal": compare_sets(reference, actual, "entry_exit_signal_key"),
            "date_side_diffs": date_side_diffs(reference, actual),
        },
        "execution_contract_classes": {
            "source_cross_day_reference_carry": {
                "count": int(len(cross_day(reference))),
                "sample": sample_rows(cross_day(reference), limit=20),
            },
            "rust_cross_day_gap_flatten": {
                "count": int(len(cross_day(actual))),
                "sample": sample_rows(cross_day(actual), limit=20),
            },
            "source_exit_time_top": time_counts(reference, "exit_ts"),
            "rust_exit_time_top": time_counts(actual, "exit_ts"),
            "source_entry_time_top": time_counts(reference, "entry_ts"),
            "rust_entry_time_top": time_counts(actual, "entry_ts"),
        },
        "residual_after_signal_shift": classify_missing_extra(reference, actual),
        "interpretation": {
            "fill_exact": "expected_to_be_poor_when_comparing_next_bar_source_to_close_bar_rust",
            "signal_entry": "high_common_after_source_timestamp_shift",
            "remaining": "review_as_hybrid_interaction_or_true_signal_drift",
        },
    }

    text = json.dumps(report, indent=2, ensure_ascii=False, default=str)
    if args.out_json:
        Path(args.out_json).write_text(text + "\n", encoding="utf-8")
    print(text)


if __name__ == "__main__":
    main()
