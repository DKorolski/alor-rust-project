import copy
import json
from pathlib import Path
import tempfile
import unittest

from compare import ParityError, file_hash
from ri_fixtures import fixtures
from ri_reference import POLICIES
from run_ri_fixture_parity import compare_rows, expected_rows, verify


class RiReferenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.f = fixtures()
        cls.rows = expected_rows(cls.f)

    def case(self, name, policy=POLICIES[0]):
        return [r["state"] for r in self.rows if r["case"] == name and r["policy"] == policy]

    def test_hand_computed_mr_quota_and_bo_levels(self):
        states = self.case("mr_shared_quota_and_bo_observation")
        self.assertEqual(states[1]["actions"][0]["pnl"], 298.)
        self.assertEqual(states[3]["actions"][0]["pnl"], 198.)
        self.assertEqual(states[4]["mr_total"], 2)
        self.assertEqual(states[4]["branches"][0], "mr:entry_quota")
        self.assertEqual(states[5]["observation_count"], 6)
        self.assertEqual((states[5]["long_level"], states[5]["short_level"]), (90840., 89160.))

    def test_recovery_changes_only_eligible_closing_context(self):
        left = self.case("closing_context_pair")
        right = self.case("closing_context_pair", POLICIES[1])
        self.assertEqual(left[0]["context"]["previous"]["close"], 90000.)
        self.assertEqual(right[0]["context"]["previous"]["close"], 90500.)
        self.assertEqual(left[0]["mr_total"], 1)
        self.assertEqual(right[0]["mr_total"], 0)
        self.assertEqual(right[0]["context"]["previous"]["bars"], 101)
        for r in self.case("missing_previous2"):
            self.assertEqual(r["actions"], [])
            self.assertIsNone(r["context"]["previous2"])

    def test_strict_thresholds_touch_priority_and_time_exit(self):
        short = self.case("short_strict_boundaries_and_stop_priority")
        self.assertEqual(short[0]["mr_total"], 0)
        self.assertEqual(short[1]["mr_total"], 0)
        self.assertEqual(short[3]["branches"][0], "mr:hold")
        self.assertEqual(short[4]["actions"][0]["reason"], "take_author_close")
        self.assertEqual(short[6]["actions"][0]["pnl"], -1501.)
        long = self.case("long_strict_take_and_stop_priority")
        self.assertEqual(long[1]["branches"][0], "mr:hold")
        self.assertEqual(long[4]["actions"][0]["reason"], "stop")
        window = self.case("mr_inclusive_window_no_forced_10_exit")
        self.assertEqual(window[-2]["branches"][0], "mr:hold")
        self.assertEqual(window[-1]["actions"][0]["reason"], "time_exit")
        for side in ("long", "short"):
            states = self.case("mr_breakeven_21_"+side)
            self.assertEqual(states[-2]["branches"][0], "mr:hold")
            self.assertEqual(states[-1]["actions"][0]["reason"], "breakeven_limit")
            self.assertEqual(states[-1]["actions"][0]["pnl"], -2.)

    def test_bo_quota_does_not_inherit_mr_quota_and_eod_is_terminal(self):
        for side in ("long", "short"):
            states = self.case("bo_three_entries_"+side)
            self.assertEqual(states[-1]["bo_total"], 3)
            self.assertEqual(states[-1]["mr_total"], 0)
            self.assertIsNone(states[-1]["bo"])
        terminal = self.case("bo_terminal_pending_entry")
        self.assertEqual(terminal[-3]["pending"]["kind"], "entry")
        self.assertIn("bo:pending_entry_terminal", terminal[-2]["branches"])
        self.assertEqual(terminal[-1]["bo_total"], 0)

    def test_no_overlap_observation_and_friday(self):
        for row in self.rows:
            s = row["state"]
            self.assertFalse(s["mr"] and s["bo"])
            self.assertEqual(s["observation_count"], row["index"]+1)
        priority = self.case("mr_priority_over_pending_bo")
        self.assertIn("bo:pending_entry_occupied", priority[8]["branches"])
        friday = self.case("friday_mr_allowed_bo_blocked")
        self.assertEqual(friday[-1]["bo_total"], 0)
        self.assertEqual(friday[-1]["mr_total"], 2)
        self.assertFalse(self.case("bo_first_hour_extreme")[-1]["trade_allowed"])

    def test_hidden_state_and_suppression_mutations_fail(self):
        for field, value in (("mr_total", 99), ("long_level", 90850.), ("branches", []), ("pending", None),
                             ("lifecycle_owner", "author42_bo"), ("lifecycle_side", "short")):
            altered = copy.deepcopy(self.rows)
            index = next(i for i, r in enumerate(altered) if r["state"][field] is not None)
            if field == "pending":
                index = next(i for i, r in enumerate(altered) if r["state"]["pending"])
            altered[index]["state"][field] = value
            with self.subTest(field=field), self.assertRaises(ParityError):
                compare_rows(self.rows, altered)

    def test_bad_inputs_and_equal_truncation_do_not_pass(self):
        bad = copy.deepcopy(self.f)
        bad["cases"][0]["events"][0]["origin"] = "history"
        with self.assertRaises(ParityError):
            expected_rows(bad)
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            source, expected, actual = (root/p for p in ("inputs.json", "expected.json", "actual.json"))
            source.write_text(json.dumps(self.f))
            expected.write_text(json.dumps(self.rows[:-1]))
            actual.write_text(json.dumps(dict(schema_version="ri_prospective_fixture_actual_v1",
                evidence_kind="synthetic_model_clock", adapter_version="ri_frozen_prospective_shadow_v2",
                input_sha256=file_hash(source), binary_sha256="a"*64, rows=self.rows[:-1])))
            with self.assertRaises(ParityError):
                verify(source, expected, actual)


if __name__ == "__main__":
    unittest.main()
