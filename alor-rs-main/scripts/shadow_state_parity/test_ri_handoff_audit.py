import csv
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import tempfile
import unittest

from audit_ri_handoff import audit, file_hash, inside, strict_json


class HandoffPreflightTests(unittest.TestCase):
    def fixture(self, root):
        base = root / "analysis"
        handoff = base / "ri_context_handoff"
        (handoff / "results").mkdir(parents=True)
        (base / "results").mkdir()
        ab = base / "ri_anchor_correction_ab"
        (ab / "results").mkdir(parents=True)
        (ab / "run_ab.py").write_text("# fixture: ordinary available_epoch is epoch + 600\n")
        columns = ["epoch", "open", "high", "low", "close", "volume", "origin", "available_epoch"]
        manifest = []
        for symbol in ("RIU6", "RIZ6"):
            source = ab / "results" / f"{symbol}.csv"
            with source.open("w", newline="") as stream:
                writer = csv.writer(stream)
                writer.writerow(columns)
                writer.writerow([1, 100, 101, 99, 100, 10, "live", 601])
            actions = handoff / "results" / f"{symbol}_expected_corrected_actions.jsonl"
            actions.write_text('{"role":"entry"}\n')
            (handoff / "results" / f"{symbol}_closing_recovery_fixture.csv").write_text(",".join(columns) + "\n")
            manifest.append({"symbol": symbol, "input_path": str(source.relative_to(base)),
                             "input_file_sha256": file_hash(source), "expected_actions_sha256": file_hash(actions),
                             "fixture_rows": 0, "retention_limitation": "synthetic test"})
        (handoff / "results/fixture_manifest.json").write_text(json.dumps(manifest))
        (base / "results/history_gap_anchor_availability.csv").write_text("symbol,bar_label,close,ingested,next_weekday_model_open\n")
        return handoff

    def test_valid_legacy_actions_cannot_certify_receipts_or_state(self):
        with tempfile.TemporaryDirectory() as d:
            h = self.fixture(Path(d))
            report = audit(h)
            self.assertEqual(report["artifact_integrity"], "PASS")
            self.assertEqual(report["status"], "BLOCKED_OPERATIONAL_REFERENCE_INCOMPLETE")
            self.assertFalse(report["operational_parity_certified"])
            self.assertEqual([s["ordinary_model_clock_rows"] for s in report["symbols"]], [1, 1])
            (h / "expected_state_trace.jsonl").write_text("{}\n")
            self.assertFalse(audit(h)["operational_parity_certified"], "a named placeholder is not an independent oracle")

    def test_changed_frozen_input_fails_integrity(self):
        with tempfile.TemporaryDirectory() as d:
            h = self.fixture(Path(d))
            p = h.parent / "ri_anchor_correction_ab/results/RIU6.csv"
            p.write_text(p.read_text().replace("601", "602"))
            self.assertEqual(audit(h)["artifact_integrity"], "FAIL")

    def test_closing_receipt_requires_unique_matching_timely_evidence(self):
        with tempfile.TemporaryDirectory() as d:
            h = self.fixture(Path(d))
            ts = int(datetime(2026, 10, 29, 23, 40, tzinfo=timezone(timedelta(hours=3))).timestamp())
            source = h.parent / "ri_anchor_correction_ab/results/RIU6.csv"
            row = f"{ts},100,101,99,100,10,history_gap,{ts + 661}\n"
            source.write_text(source.read_text() + row)
            fixture = h / "results/RIU6_closing_recovery_fixture.csv"
            fixture.write_text(fixture.read_text() + row)
            manifest_path = h / "results/fixture_manifest.json"
            manifest = json.loads(manifest_path.read_text())
            manifest[0].update(input_file_sha256=file_hash(source), fixture_rows=1)
            manifest_path.write_text(json.dumps(manifest))
            evidence_path = h.parent / "results/history_gap_anchor_availability.csv"
            header = evidence_path.read_text()
            valid = "RIU6,2026-10-29 23:40:00,100,2026-10-29 23:51:00.200000,2026-10-30 07:00:00\n"
            evidence_path.write_text(header + valid)
            report = audit(h)
            self.assertEqual(report["artifact_integrity"], "PASS", report["errors"])
            self.assertEqual(report["symbols"][0]["closing_receipts_matched"], 1)
            for bad in (
                "",
                valid + valid,
                valid.replace("23:51:00.200000", "23:51:02.200000"),
                valid.replace(",100,", ",99,"),
                valid.replace("2026-10-29 23:51:00.200000", "2026-10-30 07:00:00"),
            ):
                with self.subTest(evidence=bad):
                    evidence_path.write_text(header + bad)
                    self.assertEqual(audit(h)["artifact_integrity"], "FAIL")

    def test_missing_does_not_default_to_empty_pass(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(audit(Path(d) / "absent")["artifact_integrity"], "FAIL")

    def test_outside_paths_duplicate_keys_and_nonfinite_are_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            h = self.fixture(Path(d))
            with self.assertRaises(ValueError):
                inside(h, "../ri_anchor_correction_ab/run_ab.py")
        for raw in ('{"a":1,"a":2}', '{"x":NaN}'):
            with self.assertRaises(ValueError):
                strict_json(raw)


if __name__ == "__main__":
    unittest.main()
