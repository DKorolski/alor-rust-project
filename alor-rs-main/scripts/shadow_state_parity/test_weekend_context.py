import copy
import json
from pathlib import Path
import tempfile
import unittest

from compare import ParityError, file_hash
from run_weekend_context_parity import compare_rows, verify
from weekend_fixtures import make_case
from weekend_reference import Context, POLICIES, PROFILES, expected_rows


def fixture(cases):
    return dict(schema_version="weekend_context_fixture_v1",
                evidence_kind="synthetic_original_receipt_fixture_not_production", cases=cases)


class WeekendContextTests(unittest.TestCase):
    def run_case(self, scenario, policy=POLICIES[1], profile=PROFILES[0]):
        return expected_rows(fixture([make_case(profile, policy, scenario)]))

    def monday(self, rows):
        return next(f for f in rows[-1]["state"]["frozen"] if f["session_date"] == "2026-10-26")

    def test_hand_computed_weekend_and_tuesday_context_no_future_friday(self):
        left, right = self.run_case("on_time", POLICIES[0]), self.run_case("on_time")
        a, b = self.monday(left), self.monday(right)
        self.assertEqual((a["previous"]["session_date"], a["previous2"]["session_date"]), ("2026-10-23", "2026-10-22"))
        self.assertEqual((b["previous"]["session_date"], b["previous2"]["session_date"]), ("2026-10-25", "2026-10-24"))
        self.assertEqual(b["previous"]["high"], 200.)  # explicitly included 09:50 auction
        self.assertEqual(b["previous"]["range"], 98.)
        self.assertAlmostEqual(b["previous_return"], 104/103-1)
        self.assertAlmostEqual(right[-1]["state"]["frozen"][-1]["previous_return"], 108/104-1)
        self.assertAlmostEqual(left[-1]["state"]["frozen"][-1]["previous_return"], 108/105-1)
        first_friday = next(r["state"]["frozen"][0] for r in right if r["state"]["frozen"])
        self.assertEqual(first_friday, right[-1]["state"]["frozen"][0])

    def test_cutoff_not_first_receipt_and_late_data_never_enters_later_context(self):
        for p in PROFILES:
            for scenario in ("exact_cutoff", "late_before_first_event", "durability_crossing", "missing_receipt", "missing_durability", "after_freeze"):
                rows = self.run_case(scenario, profile=p)
                with self.subTest(profile=p, scenario=scenario):
                    self.assertEqual(self.monday(rows)["previous"]["session_date"], "2026-10-24")
                    self.assertEqual(rows[-1]["state"]["frozen"][-1]["previous2"]["session_date"], "2026-10-24")
            delayed = self.monday(self.run_case("delayed_processing", profile=p))
            self.assertEqual(delayed["previous"]["session_date"], "2026-10-25")
            self.assertEqual(delayed["previous"]["latest_durable_at_utc"], delayed["cutoff_utc"]-1)

    def test_completeness_dedupe_and_quarantine_projection(self):
        self.assertEqual(self.monday(self.run_case("sparse44"))["previous"]["distinct_bars"], 44)
        for name in ("sparse43", "first1010"):
            self.assertEqual(self.monday(self.run_case(name))["previous"]["session_date"], "2026-10-24")
        rows = self.run_case("duplicate_conflict")
        routes = [r["route"] for r in rows if r["route"]]
        self.assertIn(dict(kind="duplicate"), routes)
        self.assertIn(dict(kind="quarantined", reason="conflicting_receipt"), routes)
        self.assertIn(dict(kind="quarantined", reason="conflicting_quote"), routes)
        self.assertEqual(self.monday(rows)["previous"]["distinct_bars"], 55)
        missing = self.monday(self.run_case("weekday_incomplete", POLICIES[0]))
        self.assertEqual(missing["previous"]["session_date"], "2026-10-22")
        self.assertFalse(missing["ready"])

    def test_no_restart_equals_restart_and_future_timestamp_does_not_repair_past(self):
        case = make_case(PROFILES[0], POLICIES[1], "on_time")
        rows = expected_rows(fixture([case]))
        no_restart = copy.deepcopy(case)
        no_restart["operations"] = [o for o in no_restart["operations"] if o["kind"] != "restart"]
        self.assertEqual(rows[-1]["state"], expected_rows(fixture([no_restart]))[-1]["state"])
        for i, op in enumerate(case["operations"]):
            if op["kind"] == "restart" and i:
                self.assertEqual(rows[i]["state"], rows[i-1]["state"])

    def test_hidden_anchor_provenance_and_coverage_mutations_fail(self):
        f = fixture([make_case(PROFILES[0], POLICIES[1], "on_time")])
        rows = expected_rows(f)
        for field in ("close", "distinct_bars", "source_sequences", "latest_durable_at_utc"):
            bad = copy.deepcopy(rows)
            anchor = bad[-1]["state"]["frozen"][-1]["previous"]
            if field == "source_sequences":
                anchor[field] = anchor[field][:-1]
            else:
                anchor[field] += 1
            with self.subTest(field=field), self.assertRaises(ParityError):
                compare_rows(rows, bad)
        with tempfile.TemporaryDirectory() as d:
            src, expected, actual = (Path(d)/n for n in ("input.json", "expected.json", "actual.json"))
            src.write_text(json.dumps(f))
            expected.write_text(json.dumps(rows[:-1]))
            actual.write_text(json.dumps(dict(schema_version="weekend_context_actual_v1", version="weekend_context_cutoff_v1",
                evidence_kind=f["evidence_kind"], input_sha256=file_hash(src), binary_sha256="a"*64, rows=rows[:-1])))
            with self.assertRaises(ParityError):
                verify(src, expected, actual)

    def test_unsupported_identity_wrong_evidence_and_clock_regression_fail(self):
        c = make_case(PROFILES[0], POLICIES[1], "on_time")
        wrong = copy.deepcopy(c["identity"]); wrong["symbol"] = "RIZ6"
        with self.assertRaises(ParityError):
            Context(wrong)
        f = fixture([c]); f["evidence_kind"] = "production_receipts"
        with self.assertRaises(ParityError):
            expected_rows(f)
        c["operations"][-1]["now"] -= 1
        with self.assertRaises(ParityError):
            expected_rows(fixture([c]))

    def test_signed_zero_raw_quote_is_not_silently_deduplicated(self):
        case = make_case(PROFILES[0], POLICIES[1], "on_time")
        ctx = Context(case["identity"])
        r = copy.deepcopy(case["operations"][0]["value"])
        r["bar"]["v"] = 0.
        now = r["durable_at_utc"]
        self.assertEqual(ctx.ingest(r, now), dict(kind="context_only"))
        r["sequence"] += 1
        r["bar"]["v"] = -0.
        self.assertEqual(ctx.ingest(r, now), dict(kind="quarantined", reason="conflicting_receipt"))
        r["sequence"] += 1
        r["source_id"] = "different-source"
        self.assertEqual(ctx.ingest(r, now), dict(kind="quarantined", reason="conflicting_quote"))


if __name__ == "__main__":
    unittest.main()
