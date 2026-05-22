#!/usr/bin/env python3
"""Diagnose IMOEXF MR residuals against the filtered model-feed contract.

The saved source reference was produced before the current frozen contract was
fully clarified. This helper recomputes a standalone filtered-canonical MR from
raw/audit bars, then compares it with Rust actual trades and the saved source
reference. The goal is to separate:

- stale service-bar midpoint effects in the saved source reference;
- old calendar-zero riskgate effects;
- residual BO gap-flatten interaction with the hybrid event loop.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import pandas as pd


DEFAULT_MODEL_ID = "hybrid_mr_riskgate_high180_lb120__bo_new_k053"
DEFAULT_STANDALONE_MR_ID = "mr_riskgate_high180_lb120"
DEFAULT_SCENARIO = "base_realistic"
FIXED_MR_VARIANT = "regular_weekday_anchor|broad_005_050|high180_kl085_ks090_sl7"
MR_COST_POINTS = 0.1
SESSION_START = "09:00"
SESSION_END = "23:49"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--actual", required=True, help="Rust actual_trades_hybrid.csv")
    parser.add_argument("--reference", required=True, help="Saved source replay_trades.csv")
    parser.add_argument(
        "--source-mr",
        required=True,
        help="Saved source imoexf_mr_execution_economics_strategy_trades.csv",
    )
    parser.add_argument("--raw", required=True, help="Raw/audit CSV or parquet bars")
    parser.add_argument("--model-id", default=DEFAULT_MODEL_ID)
    parser.add_argument("--standalone-mr-id", default=DEFAULT_STANDALONE_MR_ID)
    parser.add_argument("--scenario", default=DEFAULT_SCENARIO)
    parser.add_argument("--out-json", help="Optional path to write the JSON report")
    return parser.parse_args()


def load_raw(path: Path) -> pd.DataFrame:
    if path.suffix.lower() == ".parquet":
        raw = pd.read_parquet(path)
    else:
        raw = pd.read_csv(path)
        if "Date" in raw.columns:
            raw["Date"] = pd.to_datetime(raw["Date"])
            raw = raw.set_index("Date")
        elif "datetime" in raw.columns:
            raw["datetime"] = pd.to_datetime(raw["datetime"])
            raw = raw.set_index("datetime")
        else:
            raise ValueError("CSV input must contain Date or datetime column")
    raw = raw.copy()
    raw.index = pd.to_datetime(raw.index)
    raw = raw.sort_index()
    return raw


def normalize_ohlc(raw: pd.DataFrame) -> pd.DataFrame:
    aliases = {
        "Open": "open",
        "High": "high",
        "Low": "low",
        "Close": "close",
        "open": "open",
        "high": "high",
        "low": "low",
        "close": "close",
    }
    out = raw.rename(columns={col: aliases[col] for col in raw.columns if col in aliases})
    missing = [col for col in ["open", "high", "low", "close"] if col not in out.columns]
    if missing:
        raise ValueError(f"missing required OHLC columns: {missing}")
    return out[["open", "high", "low", "close"]].copy()


def build_model_feed(raw: pd.DataFrame) -> pd.DataFrame:
    model = raw.between_time(SESSION_START, SESSION_END).copy()
    model = model[model.index.weekday < 5].copy()
    model["date"] = model.index.normalize()
    daily = (
        model.groupby("date")
        .agg(
            high=("high", "max"),
            low=("low", "min"),
            close=("close", "last"),
        )
        .reset_index()
    )
    daily["close_prev"] = daily["close"].shift(1)
    daily["range_prev"] = (daily["high"] - daily["low"]).shift(1)
    model = model.reset_index(names="datetime").merge(
        daily[["date", "close_prev", "range_prev"]], on="date", how="left"
    )
    model = model.set_index("datetime")
    model["run_high"] = model["high"].groupby(model["date"]).cummax()
    model["run_low"] = model["low"].groupby(model["date"]).cummin()
    model["run_mid"] = (model["run_high"] + model["run_low"]) / 2.0
    return model.drop(columns=["date"])


def simulate_fixed_high180(model: pd.DataFrame) -> pd.DataFrame:
    rows: list[dict[str, Any]] = []
    pos: dict[str, Any] | None = None
    for row in model.itertuples():
        ts = row.Index
        close = float(row.close)
        if pos is not None:
            side = pos["side"]
            reason = None
            if side == "long":
                if close >= pos["target_price"]:
                    reason = "midpoint_take"
                elif close <= pos["stop_price"]:
                    reason = "stop"
            else:
                if close <= pos["target_price"]:
                    reason = "midpoint_take"
                elif close >= pos["stop_price"]:
                    reason = "stop"
            hold_min = (ts - pos["entry_ts"]).total_seconds() / 60.0
            if reason is None and hold_min >= 180.0:
                reason = "time_stop"
            if reason is not None:
                gross = close - pos["entry_price"] if side == "long" else pos["entry_price"] - close
                rows.append(
                    {
                        "family": "MR",
                        "entry_ts": pos["entry_ts"],
                        "exit_ts": ts,
                        "side": side,
                        "entry_price": pos["entry_price"],
                        "exit_price": close,
                        "target_price": pos["target_price"],
                        "stop_price": pos["stop_price"],
                        "gross_points": gross,
                        "net_points": gross - MR_COST_POINTS,
                        "exit_reason": reason,
                    }
                )
                pos = None
            continue

        hhmm = ts.hour * 100 + ts.minute
        if hhmm < 900 or hhmm > 1159:
            continue
        if close == 0.0 or not pd.notna(row.close_prev) or not pd.notna(row.range_prev):
            continue
        prev = float(row.close_prev)
        rng = float(row.range_prev)
        if rng <= 0.0:
            continue
        rel = rng / close
        if not (0.005 < rel < 0.050):
            continue
        target = float(row.run_mid)
        if close < prev and close > prev - 0.085 * rng:
            if target <= close:
                continue
            stop = close - 7.0 * (target - close)
            pos = {
                "side": "long",
                "entry_ts": ts,
                "entry_price": close,
                "target_price": target,
                "stop_price": stop,
            }
        elif close > prev and close < prev + 0.090 * rng:
            if target >= close:
                continue
            stop = close + 7.0 * (close - target)
            pos = {
                "side": "short",
                "entry_ts": ts,
                "entry_price": close,
                "target_price": target,
                "stop_price": stop,
            }
    return pd.DataFrame(rows)


def apply_weekday_riskgate(fixed: pd.DataFrame, model: pd.DataFrame) -> tuple[pd.DataFrame, pd.Series]:
    dates = pd.Index(model.index.normalize().unique()).sort_values()
    if fixed.empty:
        shadow = pd.Series(0.0, index=dates)
        return fixed.copy(), shadow
    work = fixed.copy()
    work["exit_date"] = pd.to_datetime(work["exit_ts"]).dt.normalize()
    work["entry_date"] = pd.to_datetime(work["entry_ts"]).dt.normalize()
    daily = work.groupby("exit_date")["net_points"].sum().reindex(dates, fill_value=0.0)
    shadow = daily.rolling(120, min_periods=60).sum().shift(1)
    enabled = set(shadow[shadow > 0.0].index)
    return work[work["entry_date"].isin(enabled)].copy(), shadow


def read_actual(path: Path) -> pd.DataFrame:
    actual = pd.read_csv(path, parse_dates=["entry_ts", "exit_ts"])
    actual["family"] = actual["owner"].map(
        {"mean_reversion": "MR", "intraday_breakout": "BO"}
    )
    actual["side"] = actual["side"].astype(str).str.lower()
    return actual[actual["family"].isin(["MR", "BO"])].copy()


def read_reference(path: Path, model_id: str, scenario: str) -> pd.DataFrame:
    reference = pd.read_csv(path, parse_dates=["entry_ts", "exit_ts"])
    out = reference[
        (reference["model_id"] == model_id) & (reference["scenario"] == scenario)
    ].copy()
    out["side"] = out["side"].astype(str).str.lower()
    return out


def read_source_mr(path: Path) -> pd.DataFrame:
    source_mr = pd.read_csv(path, parse_dates=["entry_ts", "exit_ts"])
    out = source_mr[
        (source_mr["strategy"] == "riskgate_high180_lb120")
        & (source_mr["scenario"] == "maker_broker_1rub_rt")
    ].copy()
    out["family"] = "MR"
    out["side"] = out["side"].astype(str).str.lower()
    return out


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
    cols = ["entry_ts", "exit_ts", "side", "entry_price", "exit_price"]
    for optional in ["target_price", "exit_reason", "root_cause"]:
        if optional in df.columns:
            cols.append(optional)
    work = df.sort_values("entry_ts").head(limit)[cols].copy()
    for col in ["entry_ts", "exit_ts"]:
        work[col] = work[col].astype(str)
    return work.to_dict(orient="records")


def compare(reference: pd.DataFrame, actual: pd.DataFrame) -> dict[str, Any]:
    ref = add_keys(reference[reference["family"] == "MR"].copy())
    act = add_keys(actual[actual["family"] == "MR"].copy())
    ref_exact = set(ref["exact_key"])
    act_exact = set(act["exact_key"])
    ref_entry = set(ref["entry_key"])
    act_entry = set(act["entry_key"])
    missing = ref[~ref["exact_key"].isin(act_exact)].copy()
    extra = act[~act["exact_key"].isin(ref_exact)].copy()
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


def classify_saved_source_missing(
    saved_missing: pd.DataFrame,
    source_mr: pd.DataFrame,
    raw: pd.DataFrame,
    model: pd.DataFrame,
    canonical: pd.DataFrame,
    actual_all: pd.DataFrame,
) -> pd.DataFrame:
    source_by_key = {
        row["exact_key"]: row for row in add_keys(source_mr).to_dict(orient="records")
    }
    canonical_dates = set(pd.to_datetime(canonical["entry_ts"]).dt.normalize())
    rows = []
    for row in saved_missing.itertuples(index=False):
        source_row = source_by_key.get(row.exact_key)
        target = float(source_row["target_price"]) if source_row is not None else float("nan")
        filtered_mid = float(model.loc[row.entry_ts, "run_mid"]) if row.entry_ts in model.index else float("nan")
        day = pd.Timestamp(row.entry_ts).normalize()
        raw_day = raw[(raw.index.normalize() == day) & (raw.index.time < pd.Timestamp("09:00:00").time())]
        has_service_bar = bool(len(raw_day))
        bo_overlap = actual_all[
            (actual_all["family"] == "BO")
            & (actual_all["exit_ts"] == row.entry_ts)
            & (actual_all["entry_ts"].dt.normalize() < actual_all["exit_ts"].dt.normalize())
        ]
        cause = "unclassified_saved_source_drift"
        if not bo_overlap.empty:
            cause = "bo_gap_flatten_interaction"
        elif day not in canonical_dates:
            cause = "calendar_zero_riskgate"
        elif pd.notna(target) and pd.notna(filtered_mid) and abs(target - filtered_mid) > 1e-9:
            cause = "stale_service_bar_midpoint"
        rows.append(
            {
                "entry_ts": row.entry_ts,
                "exit_ts": row.exit_ts,
                "side": row.side,
                "entry_price": row.entry_price,
                "exit_price": row.exit_price,
                "target_price": target,
                "filtered_midpoint": filtered_mid,
                "has_service_bar": has_service_bar,
                "root_cause": cause,
            }
        )
    return pd.DataFrame(rows)


def classify_bo_gap_interaction(
    canonical_missing: pd.DataFrame,
    actual_extra: pd.DataFrame,
    actual_all: pd.DataFrame,
) -> pd.DataFrame:
    rows = []
    for row in canonical_missing.itertuples(index=False):
        shifted = actual_extra[
            (actual_extra["side"] == row.side)
            & (actual_extra["entry_price"].round(6) == round(row.entry_price, 6))
            & (actual_extra["exit_ts"] == row.exit_ts)
            & (actual_extra["exit_price"].round(6) == round(row.exit_price, 6))
            & (actual_extra["entry_ts"] > row.entry_ts)
        ]
        bo_overlap = actual_all[
            (actual_all["family"] == "BO")
            & (actual_all["exit_ts"] == row.entry_ts)
            & (actual_all["entry_ts"].dt.normalize() < actual_all["exit_ts"].dt.normalize())
        ]
        if not shifted.empty and not bo_overlap.empty:
            rows.append(
                {
                    "canonical_entry_ts": row.entry_ts,
                    "actual_entry_ts": shifted.iloc[0]["entry_ts"],
                    "exit_ts": row.exit_ts,
                    "side": row.side,
                    "entry_price": row.entry_price,
                    "exit_price": row.exit_price,
                    "root_cause": "bo_gap_flatten_interaction",
                }
            )
    return pd.DataFrame(rows)


def counts_by_cause(df: pd.DataFrame) -> dict[str, int]:
    if df.empty or "root_cause" not in df.columns:
        return {}
    return {str(k): int(v) for k, v in df["root_cause"].value_counts().sort_index().items()}


def main() -> None:
    args = parse_args()
    raw = normalize_ohlc(load_raw(Path(args.raw)))
    model = build_model_feed(raw)
    fixed = simulate_fixed_high180(model)
    canonical, shadow = apply_weekday_riskgate(fixed, model)
    canonical["family"] = "MR"
    canonical["side"] = canonical["side"].astype(str).str.lower()

    actual_all = add_keys(read_actual(Path(args.actual)))
    actual_mr = actual_all[actual_all["family"] == "MR"].copy()
    saved_hybrid = read_reference(Path(args.reference), args.model_id, args.scenario)
    saved_hybrid_mr = add_keys(saved_hybrid[saved_hybrid["family"] == "MR"].copy())
    saved_standalone = read_reference(
        Path(args.reference), args.standalone_mr_id, args.scenario
    )
    saved_standalone_mr = add_keys(saved_standalone[saved_standalone["family"] == "MR"].copy())
    source_mr = read_source_mr(Path(args.source_mr))
    canonical = add_keys(canonical)

    saved_report = compare(saved_hybrid_mr, actual_mr)
    canonical_report = compare(canonical, actual_mr)

    saved_missing = saved_hybrid_mr[
        ~saved_hybrid_mr["exact_key"].isin(set(actual_mr["exact_key"]))
    ].copy()
    saved_extra = actual_mr[
        ~actual_mr["exact_key"].isin(set(saved_hybrid_mr["exact_key"]))
    ].copy()
    canonical_missing = canonical[
        ~canonical["exact_key"].isin(set(actual_mr["exact_key"]))
    ].copy()
    canonical_extra = actual_mr[
        ~actual_mr["exact_key"].isin(set(canonical["exact_key"]))
    ].copy()

    saved_missing_classified = classify_saved_source_missing(
        saved_missing, source_mr, raw, model, canonical, actual_all
    )
    hybrid_merge_extra = saved_extra[
        saved_extra["exact_key"].isin(set(saved_standalone_mr["exact_key"]))
    ].copy()
    hybrid_merge_extra["root_cause"] = "source_hybrid_merge_bo_overlap"
    bo_gap = classify_bo_gap_interaction(canonical_missing, canonical_extra, actual_all)

    report = {
        "comparisons": {
            "saved_source_hybrid_mr_vs_rust_actual": saved_report,
            "filtered_canonical_mr_vs_rust_actual": canonical_report,
        },
        "root_cause_counts": {
            "saved_source_missing": counts_by_cause(saved_missing_classified),
            "saved_source_actual_extra": counts_by_cause(hybrid_merge_extra),
            "filtered_canonical_residual": counts_by_cause(bo_gap),
        },
        "root_cause_samples": {
            "saved_source_missing": sample_rows(saved_missing_classified),
            "saved_source_actual_extra": sample_rows(hybrid_merge_extra),
            "filtered_canonical_residual": sample_rows(bo_gap.rename(columns={"canonical_entry_ts": "entry_ts"})),
        },
        "riskgate": {
            "filtered_weekday_shadow_positive_dates": int((shadow > 0.0).sum()),
            "filtered_weekday_shadow_total_dates": int(shadow.notna().sum()),
        },
    }

    text = json.dumps(report, indent=2, ensure_ascii=False, default=str)
    if args.out_json:
        Path(args.out_json).write_text(text + "\n", encoding="utf-8")
    print(text)


if __name__ == "__main__":
    main()
