#!/usr/bin/env python3
"""Review RI Author41/42 shadow decision journal.

The RI Author41/42 pre-GO contour is intentionally shadow-only. This helper
turns the append-only JSONL decision journal into a compact operator readout
and, with ``--strict-pre-go``, fails on evidence that would be unsafe before a
formal GO/NO-GO decision.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from dataclasses import dataclass
from datetime import date
from pathlib import Path
from typing import Any


EXPECTED_PRE_GO_DECISIONS = {
    "shadow_recorded",
    "shadow_path_active",
    "shadow_path_superseded",
    "prospective_intent_suppressed",
    "intent_suppressed",
    "manual_intervention_required",
}
EXPECTED_EXECUTION_PATH = "action_scoped_only"


@dataclass(frozen=True)
class Review:
    path: Path
    from_date: date | None
    to_date: date | None
    total_rows: int
    bad_lines: list[str]
    counts: dict[str, Counter[str]]
    shadow_path_replayed_keys: dict[str, int]
    final_active_shadow_path_rows: list[dict[str, Any]]
    final_superseded_shadow_path_rows: list[dict[str, Any]]
    final_active_shadow_pnl_points: float
    final_prospective_exit_rows: list[dict[str, Any]]
    final_prospective_pnl_points: float
    live_evidence_rows: list[dict[str, Any]]
    unexpected_decision_rows: list[dict[str, Any]]
    unexpected_execution_path_rows: list[dict[str, Any]]
    tail_rows: list[dict[str, Any]]

    @property
    def violations(self) -> list[str]:
        violations: list[str] = []
        if self.bad_lines:
            violations.append(f"bad_json_lines={len(self.bad_lines)}")
        if self.live_evidence_rows:
            violations.append(f"live_emission_evidence_rows={len(self.live_evidence_rows)}")
        if self.unexpected_decision_rows:
            violations.append(
                f"unexpected_adapter_decision_rows={len(self.unexpected_decision_rows)}"
            )
        if self.unexpected_execution_path_rows:
            violations.append(
                f"unexpected_execution_path_rows={len(self.unexpected_execution_path_rows)}"
            )
        return violations


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("journal", help="Path to RI Author41/42 JSONL decision journal.")
    parser.add_argument(
        "--from-date",
        type=date.fromisoformat,
        help="Optional inclusive model-signal start date (YYYY-MM-DD) for path economics.",
    )
    parser.add_argument(
        "--to-date",
        type=date.fromisoformat,
        help="Optional inclusive model-signal end date (YYYY-MM-DD) for path economics.",
    )
    parser.add_argument("--tail", type=int, default=10, help="Number of recent rows to print.")
    parser.add_argument(
        "--strict-pre-go",
        action="store_true",
        help="Exit non-zero on pre-GO safety violations.",
    )
    parser.add_argument("--out-md", help="Optional path for a Markdown review report.")
    parser.add_argument("--out-json", help="Optional path for a machine-readable JSON summary.")
    return parser.parse_args()


def load_jsonl(path: Path) -> tuple[list[dict[str, Any]], list[str]]:
    rows: list[dict[str, Any]] = []
    bad_lines: list[str] = []
    with path.open("r", encoding="utf-8") as handle:
        for line_no, raw_line in enumerate(handle, start=1):
            line = raw_line.strip()
            if not line:
                continue
            try:
                value = json.loads(line)
            except json.JSONDecodeError as exc:
                bad_lines.append(f"{line_no}: {exc}")
                continue
            if not isinstance(value, dict):
                bad_lines.append(f"{line_no}: expected object, got {type(value).__name__}")
                continue
            rows.append(value)
    return rows, bad_lines


def value(row: dict[str, Any], key: str) -> str:
    raw = row.get(key)
    if raw is None:
        return "none"
    return str(raw)


def number(row: dict[str, Any], key: str) -> float:
    raw = row.get(key)
    if raw is None:
        return 0.0
    try:
        return float(raw)
    except (TypeError, ValueError):
        return 0.0


def in_economic_scope(row: dict[str, Any], from_date: date | None, to_date: date | None) -> bool:
    raw = value(row, "model_signal_ts_local")
    if raw == "none":
        return from_date is None and to_date is None
    try:
        signal_date = date.fromisoformat(raw[:10])
    except ValueError:
        return False
    return (from_date is None or signal_date >= from_date) and (
        to_date is None or signal_date <= to_date
    )


def build_review(
    path: Path, tail: int, from_date: date | None, to_date: date | None
) -> Review:
    if from_date and to_date and from_date > to_date:
        raise ValueError("--from-date must not be later than --to-date")
    rows, bad_lines = load_jsonl(path)
    counts: dict[str, Counter[str]] = {
        "adapter_decision": Counter(value(row, "adapter_decision") for row in rows),
        "component": Counter(value(row, "component") for row in rows),
        "role": Counter(value(row, "role") for row in rows),
        "entry_exit_reason": Counter(value(row, "entry_exit_reason") for row in rows),
        "no_overlap_decision": Counter(value(row, "no_overlap_decision") for row in rows),
        "execution_path": Counter(value(row, "execution_path") for row in rows),
        "candidate_order_style": Counter(value(row, "candidate_order_style") for row in rows),
        "candidate_intent_class": Counter(value(row, "candidate_intent_class") for row in rows),
    }

    shadow_path_rows = [
        row
        for row in rows
        if value(row, "adapter_decision")
        in {"shadow_path_active", "shadow_path_superseded"}
        and value(row, "decision_key") != "none"
    ]
    shadow_path_key_counts = Counter(value(row, "decision_key") for row in shadow_path_rows)
    shadow_path_replayed_keys = {
        key: count for key, count in shadow_path_key_counts.items() if count > 1
    }
    # JSONL is intentionally append-only. A restart may append an already active
    # path again, so economics use the latest status for each decision key.
    latest_shadow_path_by_key = {
        value(row, "decision_key"): row for row in shadow_path_rows
    }
    scoped_shadow_path_rows = [
        row
        for row in latest_shadow_path_by_key.values()
        if in_economic_scope(row, from_date, to_date)
    ]
    final_active_shadow_path_rows = [
        row
        for row in scoped_shadow_path_rows
        if value(row, "adapter_decision") == "shadow_path_active"
    ]
    final_superseded_shadow_path_rows = [
        row
        for row in scoped_shadow_path_rows
        if value(row, "adapter_decision") == "shadow_path_superseded"
    ]
    counts["final_shadow_path_status"] = Counter(
        value(row, "adapter_decision") for row in scoped_shadow_path_rows
    )
    prospective_exit_rows = [
        row
        for row in rows
        if value(row, "adapter_decision") == "prospective_intent_suppressed"
        and value(row, "role") == "exit"
        and value(row, "decision_key") != "none"
    ]
    latest_prospective_exit_by_key = {
        value(row, "decision_key"): row for row in prospective_exit_rows
    }
    final_prospective_exit_rows = [
        row
        for row in latest_prospective_exit_by_key.values()
        if in_economic_scope(row, from_date, to_date)
    ]
    counts["final_prospective_component"] = Counter(
        value(row, "component") for row in final_prospective_exit_rows
    )
    live_evidence_rows = [
        row
        for row in rows
        if row.get("request_id") is not None or row.get("broker_order_id") is not None
    ]
    unexpected_decision_rows = [
        row
        for row in rows
        if value(row, "adapter_decision") not in EXPECTED_PRE_GO_DECISIONS
    ]
    unexpected_execution_path_rows = [
        row
        for row in rows
        if value(row, "candidate_intent_class") != "none"
        and value(row, "execution_path") != EXPECTED_EXECUTION_PATH
    ]

    return Review(
        path=path,
        from_date=from_date,
        to_date=to_date,
        total_rows=len(rows),
        bad_lines=bad_lines,
        counts=counts,
        shadow_path_replayed_keys=shadow_path_replayed_keys,
        final_active_shadow_path_rows=final_active_shadow_path_rows,
        final_superseded_shadow_path_rows=final_superseded_shadow_path_rows,
        final_active_shadow_pnl_points=sum(
            number(row, "shadow_pnl_points") for row in final_active_shadow_path_rows
        ),
        final_prospective_exit_rows=final_prospective_exit_rows,
        final_prospective_pnl_points=sum(
            number(row, "shadow_pnl_points") for row in final_prospective_exit_rows
        ),
        live_evidence_rows=live_evidence_rows,
        unexpected_decision_rows=unexpected_decision_rows,
        unexpected_execution_path_rows=unexpected_execution_path_rows,
        tail_rows=rows[-max(tail, 0) :] if tail else [],
    )


def fmt_counter(counter: Counter[str]) -> str:
    if not counter:
        return "none"
    return ", ".join(f"{key}={count}" for key, count in counter.most_common())


def compact_row(row: dict[str, Any]) -> str:
    fields = [
        ("bar", "bar_ts_local"),
        ("scheduled", "candidate_scheduled_ts_local"),
        ("adapter", "adapter_decision"),
        ("component", "component"),
        ("role", "role"),
        ("side", "side"),
        ("reason", "entry_exit_reason"),
        ("path", "execution_path"),
        ("key", "decision_key"),
        ("request_id", "request_id"),
        ("broker_order_id", "broker_order_id"),
    ]
    return " ".join(f"{label}={value(row, key)}" for label, key in fields)


def render_markdown(review: Review, strict_pre_go: bool) -> str:
    status = "PASS" if not review.violations else "FAIL"
    lines = [
        "# RI Author41/42 Journal Review",
        "",
        f"- Journal: `{review.path}`",
        f"- Rows: `{review.total_rows}`",
        f"- Economics scope: `{review.from_date or 'all'}..{review.to_date or 'all'}`",
        f"- Strict pre-GO: `{strict_pre_go}`",
        f"- Status: `{status}`",
        "",
        "## Counts",
        "",
    ]
    for name, counter in review.counts.items():
        lines.append(f"- {name}: `{fmt_counter(counter)}`")
    lines.extend(
        [
            "",
            "## Safety Checks",
            "",
            f"- Bad JSON lines: `{len(review.bad_lines)}`",
            f"- Replayed raw path keys: `{len(review.shadow_path_replayed_keys)}`",
            f"- Final-active path rows: `{len(review.final_active_shadow_path_rows)}`",
            f"- Final-superseded path rows: `{len(review.final_superseded_shadow_path_rows)}`",
            f"- Final-active path PnL: `{review.final_active_shadow_pnl_points:.6g}`",
            f"- Final prospective exit rows: `{len(review.final_prospective_exit_rows)}`",
            f"- Final prospective PnL: `{review.final_prospective_pnl_points:.6g}`",
            f"- Live emission evidence rows: `{len(review.live_evidence_rows)}`",
            f"- Unexpected adapter decisions: `{len(review.unexpected_decision_rows)}`",
            f"- Unexpected execution paths: `{len(review.unexpected_execution_path_rows)}`",
            "",
        ]
    )
    if review.shadow_path_replayed_keys:
        lines.append("## Replayed Raw Path Keys")
        lines.append("")
        for key, count in sorted(review.shadow_path_replayed_keys.items()):
            lines.append(f"- `{key}`: `{count}`")
        lines.append("")
    if review.violations:
        lines.append("## Violations")
        lines.append("")
        for violation in review.violations:
            lines.append(f"- `{violation}`")
        lines.append("")
    lines.append("## Recent Rows")
    lines.append("")
    if review.tail_rows:
        for row in review.tail_rows:
            lines.append(f"- `{compact_row(row)}`")
    else:
        lines.append("- none")
    lines.append("")
    return "\n".join(lines)


def render_json(review: Review, strict_pre_go: bool) -> dict[str, Any]:
    return {
        "journal": str(review.path),
        "from_date": review.from_date.isoformat() if review.from_date else None,
        "to_date": review.to_date.isoformat() if review.to_date else None,
        "rows": review.total_rows,
        "strict_pre_go": strict_pre_go,
        "status": "PASS" if not review.violations else "FAIL",
        "violations": review.violations,
        "counts": {name: dict(counter) for name, counter in review.counts.items()},
        "bad_json_lines": review.bad_lines,
        "replayed_raw_shadow_path_decision_keys": review.shadow_path_replayed_keys,
        "final_active_shadow_path_rows": len(review.final_active_shadow_path_rows),
        "final_superseded_shadow_path_rows": len(review.final_superseded_shadow_path_rows),
        "final_active_shadow_pnl_points": review.final_active_shadow_pnl_points,
        "final_prospective_exit_rows": len(review.final_prospective_exit_rows),
        "final_prospective_pnl_points": review.final_prospective_pnl_points,
        "live_emission_evidence_rows": len(review.live_evidence_rows),
        "unexpected_adapter_decision_rows": len(review.unexpected_decision_rows),
        "unexpected_execution_path_rows": len(review.unexpected_execution_path_rows),
        "recent_rows": review.tail_rows,
    }


def main() -> int:
    args = parse_args()
    path = Path(args.journal)
    if not path.exists():
        print(f"journal not found: {path}", file=sys.stderr)
        return 2
    try:
        review = build_review(path, args.tail, args.from_date, args.to_date)
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 2
    markdown = render_markdown(review, args.strict_pre_go)
    print(markdown)

    if args.out_md:
        Path(args.out_md).write_text(markdown, encoding="utf-8")
    if args.out_json:
        Path(args.out_json).write_text(
            json.dumps(render_json(review, args.strict_pre_go), indent=2, ensure_ascii=False)
            + "\n",
            encoding="utf-8",
        )

    if args.strict_pre_go and review.violations:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
