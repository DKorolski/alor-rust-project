#!/usr/bin/env python3
"""Write one consolidated IMOEXF parity review report.

This script intentionally does not run replay itself. It aggregates the JSON
outputs from:

- `imoexf_primary_parity_diff.py`
- `imoexf_mr_residual_diagnostic.py`
- `imoexf_bo_execution_contract_diagnostic.py`
- optionally `build_imoexf_filtered_bundle.py` metadata

The generated report is a review/promotion-gate readout, not an automatic
production acceptance.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


DEFAULT_STATUS = "SIGNAL_NEAR_PARITY / EXECUTION_CONTRACT_DRIFT_EXPLICIT"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--parity-json", required=True)
    parser.add_argument("--mr-json", required=True)
    parser.add_argument("--bo-json", required=True)
    parser.add_argument("--out-md", required=True)
    parser.add_argument("--bundle-metadata-json")
    parser.add_argument("--status", default=DEFAULT_STATUS)
    parser.add_argument(
        "--accept-bo-gap-flatten",
        action="store_true",
        help="Mark Rust bo_gap_flatten as accepted runtime contract.",
    )
    return parser.parse_args()


def load_json(path: str | Path) -> dict[str, Any]:
    return json.loads(Path(path).read_text(encoding="utf-8"))


def get_count(data: dict[str, Any], *path: str, default: Any = "n/a") -> Any:
    cur: Any = data
    for key in path:
        if not isinstance(cur, dict) or key not in cur:
            return default
        cur = cur[key]
    return cur


def fmt_counts(items: dict[str, Any]) -> str:
    if not items:
        return "none"
    return ", ".join(f"{key}={value}" for key, value in items.items())


def metadata_lines(metadata: dict[str, Any] | None) -> list[str]:
    if not metadata:
        return ["- Bundle metadata: not provided."]
    validation = metadata.get("validation", {})
    return [
        f"- Rows: `{validation.get('rows', 'n/a')}`",
        f"- Date range: `{validation.get('first_ts', 'n/a')}` -> `{validation.get('last_ts', 'n/a')}`",
        f"- Weekend rows: `{validation.get('weekend_rows', 'n/a')}`",
        f"- Pre-session rows: `{validation.get('pre_session_rows', 'n/a')}`",
        f"- Post-session rows: `{validation.get('post_session_rows', 'n/a')}`",
        f"- Non-monotonic rows: `{validation.get('non_monotonic_rows', 'n/a')}`",
        f"- Regular model contract: `{validation.get('regular_model_contract', 'n/a')}`",
    ]


def main() -> None:
    args = parse_args()
    parity = load_json(args.parity_json)
    mr = load_json(args.mr_json)
    bo = load_json(args.bo_json)
    metadata = load_json(args.bundle_metadata_json) if args.bundle_metadata_json else None

    parity_mr = parity["layers"]["MR"]
    parity_bo = parity["layers"]["BO"]
    mr_saved = mr["comparisons"]["saved_source_hybrid_mr_vs_rust_actual"]
    mr_filtered = mr["comparisons"]["filtered_canonical_mr_vs_rust_actual"]
    bo_fill = bo["fill_level"]
    bo_entry = bo["signal_level"]["entry_signal"]
    bo_entry_exit = bo["signal_level"]["entry_exit_signal"]
    bo_date_side_diffs = bo["signal_level"]["date_side_diffs"]
    bo_exec = bo["execution_contract_classes"]
    bo_residual = bo["residual_after_signal_shift"]

    bo_gap_status = (
        "ACCEPTED_RUNTIME_CONTRACT" if args.accept_bo_gap_flatten else "PENDING_TEAM_DECISION"
    )
    promotion_status = (
        "READY_FOR_EXTENDED_MICRO_SOAK_AFTER_OPERATIONAL_GATES"
        if args.accept_bo_gap_flatten
        else "NOT_READY_FOR_PROMOTION_UNTIL_BO_GAP_FLATTEN_DECISION"
    )

    lines: list[str] = [
        "# IMOEXF Primary Parity Review Report",
        "",
        "## Status",
        "",
        f"- Current status: `{args.status}`",
        f"- BO gap-flatten decision: `{bo_gap_status}`",
        f"- Promotion status: `{promotion_status}`",
        "",
        "## Model Feed Contract",
        "",
        "- Prepared/model feed must contain only regular tradable bars: Monday-Friday `09:00..23:49`.",
        "- Raw/audit feed may contain service bars such as `08:50`, but those bars must not update MR, BO, riskgate, entry/exit, or parity state.",
        *metadata_lines(metadata),
        "",
        "## Layer Summary",
        "",
        "### Saved Source Reference vs Rust Replay",
        "",
        f"- MR: source `{parity_mr['reference']}`, Rust `{parity_mr['actual']}`, exact `{parity_mr['exact_common']}`, missing/extra `{parity_mr['reference_missing_exact']} / {parity_mr['actual_extra_exact']}`.",
        f"- BO: source `{parity_bo['reference']}`, Rust `{parity_bo['actual']}`, exact `{parity_bo['exact_common']}`, missing/extra `{parity_bo['reference_missing_exact']} / {parity_bo['actual_extra_exact']}`.",
        "",
        "## MR Read",
        "",
        "- Saved source MR drift is mostly stale-reference drift, not a Rust MR signal failure.",
        f"- Saved-source MR vs Rust: `{mr_saved['reference']}` vs `{mr_saved['actual']}`, exact `{mr_saved['exact_common']}`, missing/extra `{mr_saved['reference_missing_exact']} / {mr_saved['actual_extra_exact']}`.",
        f"- Filtered canonical MR vs Rust: `{mr_filtered['reference']}` vs `{mr_filtered['actual']}`, exact `{mr_filtered['exact_common']}`, missing/extra `{mr_filtered['reference_missing_exact']} / {mr_filtered['actual_extra_exact']}`.",
        f"- Saved-source missing causes: `{fmt_counts(mr['root_cause_counts'].get('saved_source_missing', {}))}`.",
        f"- Saved-source actual-extra causes: `{fmt_counts(mr['root_cause_counts'].get('saved_source_actual_extra', {}))}`.",
        f"- Filtered canonical residual causes: `{fmt_counts(mr['root_cause_counts'].get('filtered_canonical_residual', {}))}`.",
        "",
        "## BO Read",
        "",
        "- BO fill-level exact parity is expected to be poor while comparing Backtrader next-bar fills with Rust close-bar/event-loop actions.",
        f"- Fill-level: source `{bo_fill['reference']}`, Rust `{bo_fill['actual']}`, exact `{bo_fill['exact_common']}`, missing/extra `{bo_fill['reference_missing_exact']} / {bo_fill['actual_extra_exact']}`.",
        f"- After source timestamp normalization (`-{bo['source_next_bar_minutes']}m`), entry-signal common `{bo_entry['common']} / {bo_entry['reference']}`.",
        f"- After source timestamp normalization (`-{bo['source_next_bar_minutes']}m`), entry+exit-signal common `{bo_entry_exit['common']} / {bo_entry_exit['reference']}`.",
        f"- Date+side count diffs after normalization: `{len(bo_date_side_diffs)}`.",
        f"- Source cross-day reference carry: `{get_count(bo_exec, 'source_cross_day_reference_carry', 'count')}`.",
        f"- Rust cross-day gap-flatten: `{get_count(bo_exec, 'rust_cross_day_gap_flatten', 'count')}`.",
        f"- Residual after signal shift: missing/extra `{bo_residual['missing_count']} / {bo_residual['extra_count']}`.",
        "",
        "## Required Decision",
        "",
        "- Preferred live contract is Rust close-bar/no-overnight behavior.",
        "- `bo_gap_flatten` should be accepted explicitly if the team agrees that BO must not carry through non-tradable gaps.",
        "- Do not tune Rust toward Backtrader cross-day carry unless the team intentionally chooses replay-fill parity over live safety semantics.",
        "",
        "## Promotion Gate",
        "",
        "- Rebuild the official filtered bundle through `2026-04-21`.",
        "- Run `hybrid_replay --profile imoexf_primary_riskgate_k053 --assert-gap-flatten` on the official bundle.",
        "- Publish this report from official artifacts, not temporary `/tmp` outputs.",
        "- Record the `bo_gap_flatten` decision.",
        "- If accepted and clean, start extended micro soak at live size `1` with explicit MR/BO attribution monitoring.",
        "",
    ]

    Path(args.out_md).write_text("\n".join(lines), encoding="utf-8")
    print(args.out_md)


if __name__ == "__main__":
    main()
