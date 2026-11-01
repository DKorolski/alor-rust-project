"""Read-only preflight of the legacy RI handoff; NEVER certify operational parity.

The old available_epoch field mixes a model clock with selected gateway evidence.
Do not convert it into a production ingress journal or invent fsync timestamps.
"""
from __future__ import annotations

import argparse
import csv
from datetime import datetime, timedelta, timezone
import hashlib
import json
import math
from pathlib import Path

SCHEMA = "ri_legacy_handoff_preflight_v1"
MSK = timezone(timedelta(hours=3))


def file_hash(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def inside(base: Path, relative: str) -> Path:
    path = (base / relative).resolve()
    if not path.is_relative_to(base.resolve()) or not path.is_file():
        raise ValueError(f"Missing or out-of-scope handoff file: {relative}")
    return path


def read_csv(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as stream:
        reader = csv.DictReader(stream)
        if not reader.fieldnames or len(set(reader.fieldnames)) != len(reader.fieldnames):
            raise ValueError(f"Missing/duplicate CSV columns: {path.name}")
        return list(reader)


def strict_json(text: str):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError(f"Duplicate JSON field: {key}")
            result[key] = value
        return result

    def nonfinite(value):
        raise ValueError(f"Non-finite JSON: {value}")

    return json.loads(text, object_pairs_hook=pairs, parse_constant=nonfinite)


def msk_epoch(raw: str) -> float:
    return datetime.fromisoformat(raw).replace(tzinfo=MSK).timestamp()


def audit(handoff: Path) -> dict:
    handoff = handoff.resolve()
    base = handoff.parent
    report = {
        "schema_version": SCHEMA,
        "status": "BLOCKED_OPERATIONAL_REFERENCE_INCOMPLETE",
        "artifact_integrity": "PASS",
        "operational_parity_certified": False,
        "numerical_replay_run": False,
        "source_files_modified": False,
        "symbols": [],
        "errors": [],
        "requirements_before_operational_certification": [
            "Original ordered receipts for ALL consumed bars, with sequence and source ID.",
            "Separate original received_at_utc and durable_at_utc evidence; no bar-close substitution.",
            "Durable cutoff barriers, explicit generation/activation session and exact config identity.",
            "Independent expected_state_trace per event: anchors, counters, observation, owner/pending, suppression.",
            "Expected actions for BOTH matched policy arms and the selected activation scope.",
            "Restart/outbound-publication acceptance on the complete runner, not only the old gate checkpoint.",
        ],
    }
    try:
        manifest_path = inside(handoff, "results/fixture_manifest.json")
        manifest = strict_json(manifest_path.read_text())
        if not isinstance(manifest, list) or {m["symbol"] for m in manifest} != {"RIU6", "RIZ6"} or len(manifest) != 2:
            raise ValueError("Expected one RIU6 and one RIZ6 manifest entry")
        producer = inside(base, "ri_anchor_correction_ab/run_ab.py")
        availability_path = inside(base, "results/history_gap_anchor_availability.csv")
        evidence = read_csv(availability_path)
        report["manifest_sha256"] = file_hash(manifest_path)
        report["producer_sha256"] = file_hash(producer)
        report["receipt_evidence_sha256"] = file_hash(availability_path)
        report["legacy_clock_contract"] = {
            "ordinary_bars": "model_available_time_bar_begin_plus_600_seconds",
            "closing_history_gap": "selected_gateway_ingested_time_rounded_up_to_seconds",
            "durability": "not_provided",
            "note": "Documented legacy run_ab.py construction, not original local collector receipts.",
        }
        report["state_trace_files_found"] = sorted(
            str(p.relative_to(handoff)) for p in handoff.rglob("*state*trace*") if p.is_file()
        )
        for entry in manifest:
            symbol = entry["symbol"]
            source = inside(base, entry["input_path"])
            expected = inside(handoff, f"results/{symbol}_expected_corrected_actions.jsonl")
            fixture = inside(handoff, f"results/{symbol}_closing_recovery_fixture.csv")
            if file_hash(source) != entry["input_file_sha256"]:
                raise ValueError(f"{symbol}: input hash mismatch")
            if file_hash(expected) != entry["expected_actions_sha256"]:
                raise ValueError(f"{symbol}: expected-action hash mismatch")
            rows = read_csv(source)
            if not rows:
                raise ValueError(f"{symbol}: empty input")
            required = {"epoch", "open", "high", "low", "close", "volume", "origin", "available_epoch"}
            if not required.issubset(rows[0]):
                raise ValueError(f"{symbol}: incomplete legacy input schema")
            gap = [row for row in rows if row["origin"] == "history_gap"]
            if gap != read_csv(fixture) or len(gap) != entry["fixture_rows"]:
                raise ValueError(f"{symbol}: closing fixture does not match input subset")
            seen = set()
            previous = None
            ordinary_clock_rows = 0
            for row in rows:
                ts, available = int(row["epoch"]), int(row["available_epoch"])
                if ts in seen or available < ts + 600:
                    raise ValueError(f"{symbol}: duplicate/unclosed input")
                seen.add(ts)
                key = (available, ts)
                if previous is not None and key < previous:
                    raise ValueError(f"{symbol}: input receipt-clock order regression")
                previous = key
                o, h, l, c, v = (float(row[k]) for k in ("open", "high", "low", "close", "volume"))
                if not all(map(math.isfinite, (o, h, l, c, v))) or not (0 < l <= min(o, c) <= max(o, c) <= h and v >= 0):
                    raise ValueError(f"{symbol}: invalid OHLCV")
                if row["origin"] != "history_gap" and available == ts + 600:
                    ordinary_clock_rows += 1
            matched = 0
            for row in gap:
                label = datetime.fromtimestamp(int(row["epoch"]), MSK).replace(tzinfo=None)
                candidates = [e for e in evidence if e["symbol"] == symbol and datetime.fromisoformat(e["bar_label"]) == label]
                if len(candidates) != 1:
                    raise ValueError(f"{symbol}: missing/ambiguous closing receipt evidence")
                e = candidates[0]
                ingested = msk_epoch(e["ingested"])
                if not (int(row["epoch"]) + 600 <= ingested < msk_epoch(e["next_weekday_model_open"])):
                    raise ValueError(f"{symbol}: invalid closing availability window")
                if int(row["available_epoch"]) != math.ceil(ingested) or float(row["close"]) != float(e["close"]):
                    raise ValueError(f"{symbol}: closing evidence mismatch")
                matched += 1
            actions = [strict_json(line) for line in expected.read_text().splitlines()]
            if not actions or any(not isinstance(a, dict) or not a.get("role") for a in actions):
                raise ValueError(f"{symbol}: invalid expected-action stream")
            report["symbols"].append({
                "symbol": symbol, "input_rows": len(rows), "expected_actions": len(actions),
                "input_sha256": file_hash(source), "expected_actions_sha256": file_hash(expected),
                "closing_fixture_sha256": file_hash(fixture), "closing_receipts_matched": matched,
                "ordinary_model_clock_rows": ordinary_clock_rows,
                "has_per_row_durability": "durable_at_utc" in rows[0],
                "has_source_sequence": {"sequence", "source_id"}.issubset(rows[0]),
                "retention_limitation": entry["retention_limitation"],
            })
    except (OSError, ValueError, KeyError, TypeError) as error:
        report["artifact_integrity"] = "FAIL"
        report["status"] = "FAILED_LEGACY_ARTIFACT_INTEGRITY"
        report["errors"].append(str(error))
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--handoff", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="NEW report file; never overwrite a frozen artifact")
    args = parser.parse_args()
    if args.output.resolve().is_relative_to(args.handoff.resolve()):
        parser.error("write reports outside the frozen handoff")
    report = audit(args.handoff)
    with args.output.open("x", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2, ensure_ascii=True, allow_nan=False)
        stream.write("\n")
    print(json.dumps(report, indent=2, allow_nan=False))
    return 1 if report["artifact_integrity"] == "FAIL" else 2


if __name__ == "__main__":
    raise SystemExit(main())
