#!/usr/bin/env python3
"""Summarize IMOEXF hybrid shadow trade CSVs for promotion review.

The script intentionally works with both legacy CSVs and the enriched
closed-trade CSVs produced by strategy-runtime after 2026-09-04.
Legacy rows will be reported with strategy_component=UNKNOWN.
"""

from __future__ import annotations

import argparse
import csv
import json
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Iterable


@dataclass(frozen=True)
class TradeRow:
    source: str
    entry_ts_utc: int
    exit_ts_utc: int
    entry_dt_local: str
    exit_dt_local: str
    session_date: str
    week_start: str
    symbol: str
    strategy_component: str
    side: str
    qty: float
    entry_price: float
    exit_price: float
    pnl_gross: float
    pnl_net: float
    exit_timing_label: str

    @property
    def dedupe_key(self) -> tuple[object, ...]:
        return (
            self.source,
            self.entry_ts_utc,
            self.exit_ts_utc,
            self.symbol,
            self.strategy_component,
            self.side,
            round(self.qty, 12),
            round(self.entry_price, 12),
            round(self.exit_price, 12),
            round(self.pnl_gross, 12),
            round(self.pnl_net, 12),
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--input",
        action="append",
        required=True,
        help="Path to a shadow trades CSV. Can be passed multiple times.",
    )
    parser.add_argument("--since", help="Filter by local entry date, inclusive, YYYY-MM-DD.")
    parser.add_argument("--until", help="Filter by local entry date, inclusive, YYYY-MM-DD.")
    parser.add_argument("--tz-offset-hours", type=int, default=3)
    parser.add_argument(
        "--rub-per-point",
        type=float,
        default=10.0,
        help="Approximate RUB multiplier for one price point * one contract of IMOEXF.",
    )
    parser.add_argument("--output-md", help="Write a markdown report to this path.")
    parser.add_argument("--output-json", help="Write machine-readable summary JSON to this path.")
    return parser.parse_args()


def as_int(raw: str | None) -> int:
    if raw is None or raw == "":
        return 0
    return int(float(raw))


def as_float(raw: str | None) -> float:
    if raw is None or raw == "":
        return 0.0
    return float(raw)


def local_dt(ts_utc: int, offset_hours: int) -> datetime:
    return datetime.fromtimestamp(ts_utc, tz=timezone.utc).astimezone(
        timezone(timedelta(hours=offset_hours))
    )


def week_start(day: datetime) -> str:
    return (day.date() - timedelta(days=day.weekday())).isoformat()


def parse_hyb_comment_owner(comment: str | None) -> str | None:
    if not comment or not comment.startswith("HYB|"):
        return None
    for part in comment.split("|")[1:]:
        if part == "o=MR":
            return "MR"
        if part == "o=BO":
            return "BO"
    return None


def infer_component(row: dict[str, str]) -> str:
    for field in ("strategy_component", "component", "owner"):
        value = row.get(field, "").strip().upper()
        if value in {"MR", "BO"}:
            return value
    for field in ("entry_comment", "exit_comment", "comment"):
        owner = parse_hyb_comment_owner(row.get(field))
        if owner:
            return owner
    return "UNKNOWN"


def exit_timing_label(entry_local: datetime, exit_local: datetime) -> str:
    if entry_local.date() == exit_local.date():
        return "same_day"
    exit_minutes = exit_local.hour * 60 + exit_local.minute
    if 7 * 60 <= exit_minutes <= 7 * 60 + 20:
        return "next_session_07xx_gap_exit"
    if 9 * 60 + 20 <= exit_minutes <= 9 * 60 + 40:
        return "next_session_0930_legacy_exit"
    return "cross_day_other"


def source_name(path: Path) -> str:
    stem = path.stem
    prefix = "moex_early_session_imoexf_trades_"
    if stem.startswith(prefix):
        return stem[len(prefix) :]
    return stem


def load_rows(path: Path, offset_hours: int) -> list[TradeRow]:
    rows: list[TradeRow] = []
    with path.open(newline="") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            entry_ts = as_int(row.get("entry_ts_utc"))
            exit_ts = as_int(row.get("exit_ts_utc"))
            if entry_ts <= 0 or exit_ts <= 0:
                continue
            entry_local = local_dt(entry_ts, offset_hours)
            exit_local = local_dt(exit_ts, offset_hours)
            pnl_gross = as_float(row.get("pnl_gross"))
            pnl_net = as_float(row.get("pnl_net"))
            rows.append(
                TradeRow(
                    source=source_name(path),
                    entry_ts_utc=entry_ts,
                    exit_ts_utc=exit_ts,
                    entry_dt_local=entry_local.strftime("%Y-%m-%d %H:%M:%S"),
                    exit_dt_local=exit_local.strftime("%Y-%m-%d %H:%M:%S"),
                    session_date=entry_local.date().isoformat(),
                    week_start=week_start(entry_local),
                    symbol=row.get("symbol", ""),
                    strategy_component=infer_component(row),
                    side=row.get("side", ""),
                    qty=as_float(row.get("qty")),
                    entry_price=as_float(row.get("entry_price")),
                    exit_price=as_float(row.get("exit_price")),
                    pnl_gross=pnl_gross,
                    pnl_net=pnl_net,
                    exit_timing_label=exit_timing_label(entry_local, exit_local),
                )
            )
    return rows


def filter_rows(rows: Iterable[TradeRow], since: str | None, until: str | None) -> list[TradeRow]:
    result = []
    for row in rows:
        if since and row.session_date < since:
            continue
        if until and row.session_date > until:
            continue
        result.append(row)
    return result


def dedupe_rows(rows: Iterable[TradeRow]) -> list[TradeRow]:
    seen: set[tuple[object, ...]] = set()
    result = []
    for row in sorted(rows, key=lambda item: (item.source, item.entry_ts_utc, item.exit_ts_utc)):
        if row.dedupe_key in seen:
            continue
        seen.add(row.dedupe_key)
        result.append(row)
    return result


def aggregate(rows: list[TradeRow], rub_per_point: float) -> dict[str, object]:
    total_pnl = sum(row.pnl_net for row in rows)
    wins = sum(1 for row in rows if row.pnl_net > 0)
    losses = sum(1 for row in rows if row.pnl_net < 0)
    by_component: dict[str, dict[str, object]] = {}
    by_source: dict[str, dict[str, object]] = {}
    by_week: dict[str, dict[str, object]] = {}
    by_exit_timing: dict[str, int] = dict(Counter(row.exit_timing_label for row in rows))

    for group_name, target in (
        ("strategy_component", by_component),
        ("source", by_source),
        ("week_start", by_week),
    ):
        grouped: dict[str, list[TradeRow]] = defaultdict(list)
        for row in rows:
            grouped[getattr(row, group_name)].append(row)
        for key, group in sorted(grouped.items()):
            pnl = sum(row.pnl_net for row in group)
            target[key] = {
                "trades": len(group),
                "pnl_net_points": round(pnl, 8),
                "pnl_net_rub_approx": round(pnl * rub_per_point, 2),
                "wins": sum(1 for row in group if row.pnl_net > 0),
                "losses": sum(1 for row in group if row.pnl_net < 0),
            }

    return {
        "trades": len(rows),
        "pnl_net_points": round(total_pnl, 8),
        "pnl_net_rub_approx": round(total_pnl * rub_per_point, 2),
        "wins": wins,
        "losses": losses,
        "by_component": by_component,
        "by_source": by_source,
        "by_week": by_week,
        "by_exit_timing": by_exit_timing,
    }


def markdown_report(rows: list[TradeRow], summary: dict[str, object]) -> str:
    lines = [
        "# IMOEXF Shadow Observation Summary",
        "",
        "## Totals",
        "",
        f"- Trades: {summary['trades']}",
        f"- Net PnL points: {summary['pnl_net_points']}",
        f"- Approx RUB: {summary['pnl_net_rub_approx']}",
        f"- Wins/Losses: {summary['wins']} / {summary['losses']}",
        "",
        "## By Source",
        "",
        "| Source | Trades | Net points | Approx RUB | Wins | Losses |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for source, stats in summary["by_source"].items():
        lines.append(
            f"| {source} | {stats['trades']} | {stats['pnl_net_points']} | "
            f"{stats['pnl_net_rub_approx']} | {stats['wins']} | {stats['losses']} |"
        )

    lines.extend(
        [
            "",
            "## By Component",
            "",
            "| Component | Trades | Net points | Approx RUB | Wins | Losses |",
            "|---|---:|---:|---:|---:|---:|",
        ]
    )
    for component, stats in summary["by_component"].items():
        lines.append(
            f"| {component} | {stats['trades']} | {stats['pnl_net_points']} | "
            f"{stats['pnl_net_rub_approx']} | {stats['wins']} | {stats['losses']} |"
        )

    lines.extend(
        [
            "",
            "## By Week",
            "",
            "| Week Start | Trades | Net points | Approx RUB | Wins | Losses |",
            "|---|---:|---:|---:|---:|---:|",
        ]
    )
    for week, stats in summary["by_week"].items():
        lines.append(
            f"| {week} | {stats['trades']} | {stats['pnl_net_points']} | "
            f"{stats['pnl_net_rub_approx']} | {stats['wins']} | {stats['losses']} |"
        )

    lines.extend(
        [
            "",
            "## Exit Timing Labels",
            "",
            "| Label | Count |",
            "|---|---:|",
        ]
    )
    for label, count in sorted(summary["by_exit_timing"].items()):
        lines.append(f"| {label} | {count} |")

    unknown_components = sum(1 for row in rows if row.strategy_component == "UNKNOWN")
    if unknown_components:
        lines.extend(
            [
                "",
                "## Notes",
                "",
                f"- {unknown_components} rows have `strategy_component=UNKNOWN`; this is expected for legacy CSVs generated before enriched HYB comment export.",
            ]
        )
    return "\n".join(lines) + "\n"


def main() -> None:
    args = parse_args()
    rows: list[TradeRow] = []
    for raw_path in args.input:
        rows.extend(load_rows(Path(raw_path), args.tz_offset_hours))
    rows = dedupe_rows(filter_rows(rows, args.since, args.until))
    summary = aggregate(rows, args.rub_per_point)
    payload = {
        "summary": summary,
        "rows": [asdict(row) for row in rows],
    }

    if args.output_json:
        Path(args.output_json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.output_json).write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n")
    report = markdown_report(rows, summary)
    if args.output_md:
        Path(args.output_md).parent.mkdir(parents=True, exist_ok=True)
        Path(args.output_md).write_text(report)
    else:
        print(report, end="")


if __name__ == "__main__":
    main()
