#!/usr/bin/env python3
"""Build the filtered IMOEXF hybrid replay bundle from raw/audit bars.

The frozen IMOEXF hybrid model feed is not the full raw feed. Raw/audit data may
contain service bars such as 08:50, but the prepared model feed must include
only regular tradable weekday bars from 09:00 through 23:49.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import pandas as pd


SESSION_START = "09:00"
SESSION_END = "23:49"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw", required=True, help="Raw/audit CSV or parquet bars")
    parser.add_argument("--out-dir", required=True, help="Directory for prepared_hybrid.csv")
    parser.add_argument(
        "--allow-weekends",
        action="store_true",
        help="Keep weekend rows. Default contract excludes weekends.",
    )
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


def normalize_columns(raw: pd.DataFrame) -> pd.DataFrame:
    aliases = {
        "Open": "open",
        "High": "high",
        "Low": "low",
        "Close": "close",
        "Volume": "volume",
        "open": "open",
        "high": "high",
        "low": "low",
        "close": "close",
        "volume": "volume",
    }
    out = raw.rename(columns={col: aliases[col] for col in raw.columns if col in aliases})
    required = ["open", "high", "low", "close"]
    missing = [col for col in required if col not in out.columns]
    if missing:
        raise ValueError(f"missing required columns: {missing}")
    if "volume" not in out.columns:
        out["volume"] = 0
    return out[["open", "high", "low", "close", "volume"]].copy()


def filter_model_session(raw: pd.DataFrame, allow_weekends: bool) -> pd.DataFrame:
    out = raw.between_time(SESSION_START, SESSION_END).copy()
    if not allow_weekends:
        out = out[out.index.weekday < 5].copy()
    return out


def add_regular_weekday_anchors(model: pd.DataFrame) -> pd.DataFrame:
    work = model.copy()
    work["date"] = work.index.normalize()
    daily = (
        work.groupby("date")
        .agg(
            high_prev_source=("high", "max"),
            low_prev_source=("low", "min"),
            close_prev_source=("close", "last"),
        )
        .reset_index()
    )
    daily["high_prev"] = daily["high_prev_source"].shift(1)
    daily["low_prev"] = daily["low_prev_source"].shift(1)
    daily["close_prev"] = daily["close_prev_source"].shift(1)
    daily["dayrangeprev"] = daily["high_prev"] - daily["low_prev"]
    anchors = daily[["date", "high_prev", "low_prev", "close_prev", "dayrangeprev"]]
    out = work.reset_index(names="datetime").merge(anchors, on="date", how="left")
    out = out.drop(columns=["date"])
    return out


def validate(prepared: pd.DataFrame) -> dict[str, object]:
    dt = pd.to_datetime(prepared["datetime"])
    weekend_rows = int((dt.dt.weekday >= 5).sum())
    pre_start_rows = int((dt.dt.time < pd.Timestamp(f"{SESSION_START}:00").time()).sum())
    post_end_rows = int((dt.dt.time > pd.Timestamp(f"{SESSION_END}:59").time()).sum())
    non_monotonic = int((dt.diff().dropna() <= pd.Timedelta(0)).sum())
    return {
        "rows": int(len(prepared)),
        "first_ts": str(dt.min()),
        "last_ts": str(dt.max()),
        "weekend_rows": weekend_rows,
        "pre_session_rows": pre_start_rows,
        "post_session_rows": post_end_rows,
        "non_monotonic_rows": non_monotonic,
        "regular_model_contract": weekend_rows == 0
        and pre_start_rows == 0
        and post_end_rows == 0
        and non_monotonic == 0,
    }


def main() -> None:
    args = parse_args()
    raw_path = Path(args.raw)
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    raw = normalize_columns(load_raw(raw_path))
    model = filter_model_session(raw, args.allow_weekends)
    prepared = add_regular_weekday_anchors(model)
    prepared["datetime"] = pd.to_datetime(prepared["datetime"]).dt.strftime("%Y-%m-%d %H:%M:%S")

    cols = [
        "datetime",
        "open",
        "high",
        "low",
        "close",
        "volume",
        "high_prev",
        "low_prev",
        "close_prev",
        "dayrangeprev",
    ]
    prepared[cols].to_csv(out_dir / "prepared_hybrid.csv", index=False)

    metadata = {
        "raw_path": str(raw_path),
        "session_start": SESSION_START,
        "session_end": SESSION_END,
        "allow_weekends": bool(args.allow_weekends),
        "validation": validate(prepared),
    }
    (out_dir / "metadata.json").write_text(
        json.dumps(metadata, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(metadata, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
