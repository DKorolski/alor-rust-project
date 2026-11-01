import copy
import os
from pathlib import Path
import unittest

from compare import ParityError
from reaction_compare import compare, projection
from reaction_fixtures import configs, fixtures, make_case, SCENARIOS
from reaction_reference import expected, frame_for

IMO = os.environ.get("IMOEXF_STATE_HANDOFF")
USD = os.environ.get("USDRUBF_STATE_HANDOFF")


class ReactionSchemaTests(unittest.TestCase):
    def test_exact_matrix_and_both_policies_share_all_price_inputs(self):
        f = fixtures()
        self.assertEqual(len(f["cases"]), 7*2*len(SCENARIOS))
        self.assertEqual(len({c["id"] for c in f["cases"]}), len(f["cases"]))
        for cfg in configs():
            for scenario in SCENARIOS:
                a, b = (make_case(cfg, p, scenario) for p in ("weekday_only", "weekend_context_no_trades"))
                self.assertEqual(a["history"], b["history"])
                self.assertEqual(a["operations"], b["operations"])
                self.assertEqual(a["identity"]["model_config_sha256"], b["identity"]["model_config_sha256"])
        ids = [make_case(c, "weekday_only", "mr_take_bo_eod")["identity"]["model_config_sha256"] for c in configs()]
        self.assertEqual(len(set(ids)), 7)

    def test_comparator_rejects_missing_rows_fields_types_and_nonfinite_numbers(self):
        good = [dict(owner="MR", count=2, value=0.52, pending=None)]
        for bad in ([], [dict(owner="MR", count=2., value=0.52, pending=None)],
                    [dict(owner="MR", count=2, value=float("nan"), pending=None)],
                    [dict(owner="MR", count=2, value=0.53, pending=None)],
                    [dict(owner="MR", count=2, value=0.52)]):
            with self.assertRaises(ParityError):
                compare(good, bad)
        with self.assertRaises(ParityError):
            projection(dict(engine="wrong", state={}), configs()[0])

    def test_missing_execution_bar_cannot_be_hidden_by_reference_truncation(self):
        case = make_case(configs()[0], "weekday_only", "mr_take_bo_eod")
        case["operations"].pop(0)
        with self.assertRaises(ParityError):
            frame_for(case)


@unittest.skipUnless(IMO and USD, "explicit immutable handoff locations required")
class ReactionReferenceTests(unittest.TestCase):
    def run_cases(self, cases):
        f = dict(schema_version="weekend_reaction_fixture_v1",
                 evidence_kind="synthetic_numerical_reaction_not_production", cases=cases)
        return expected(f, Path(IMO), Path(USD))

    def test_hand_computed_mr_fill_prices_and_bo_only_never_opens_mr(self):
        cases = [make_case(c, "weekday_only", "mr_take_bo_eod") for c in configs()]
        for cfg, result in zip(configs(), self.run_cases(cases)):
            trades = result["rows"][-1]["model"]["trades"]
            if cfg.get("sleeve") == "bo":
                self.assertTrue(trades)
                self.assertTrue(all(t["owner"] == "BO" for t in trades))
                self.assertEqual(result["rows"][-1]["model"]["mr_counter"], 0)
            else:
                t = trades[0]
                self.assertEqual((t["owner"], t["side"], t["entry_price"], t["exit_price"]), ("MR", "short", 100.12, 99.6))
                self.assertAlmostEqual(t["gross_points"], .52)

    def test_weekend_cutoff_is_not_first_event_and_late_context_cannot_rewrite_decisions(self):
        cfg = configs()[2]
        cases = [make_case(cfg, "weekend_context_no_trades", s) for s in
                 ("mr_take_bo_eod", "late_sunday_close", "after_freeze_close", "pre_transition")]
        ontime, late, after, pre = self.run_cases(cases)
        self.assertEqual(ontime["frozen"]["previous"]["session_date"], "2026-10-25")
        self.assertEqual(late["frozen"]["previous"]["session_date"], "2026-10-24")
        self.assertEqual(after["frozen"]["previous"]["session_date"], "2026-10-24")
        self.assertEqual(pre["frozen"]["previous"]["session_date"], "2026-07-12")
        receipt = next(r for r in after["rows"] if r["kind"] == "receipt")
        self.assertEqual(receipt["route"]["reason"], "behind_frozen_barrier")
        self.assertEqual(receipt["model"], after["rows"][0]["model"])
        self.assertEqual(pre["rows"][0]["model"]["observation"]["first"], "2026-07-13T09:00:00")

    def test_restarts_have_no_new_events_and_missing_eod_fails(self):
        c = make_case(configs()[2], "weekday_only", "mr_take_bo_eod")
        result = self.run_cases([c])[0]
        for i, r in enumerate(result["rows"]):
            if r["kind"] == "restart":
                self.assertEqual(r["model"], result["rows"][i-1]["model"])
        c["operations"] = c["operations"][:-1]
        with self.assertRaises(ParityError):
            self.run_cases([c])

    def test_ownership_quotas_context_and_suppression_mutations_are_visible(self):
        c = make_case(configs()[2], "weekday_only", "mr_bo_overlap")
        rows = self.run_cases([c])[0]["rows"]
        candidates = [(i, r) for i, r in enumerate(rows) if r["model"]["pending"]]
        i, _ = candidates[0]
        for field in ("branch", "mr_counter", "previous", "pending", "observation"):
            bad = copy.deepcopy(rows)
            bad[i]["model"][field] = None
            with self.subTest(field=field), self.assertRaises(ParityError):
                compare(rows, bad)


if __name__ == "__main__":
    unittest.main()
