import copy
from contextlib import ExitStack
import os
from pathlib import Path
import unittest

import pandas as pd

from compare import ParityError
from export_imoexf import reference_modules as imo_modules
from export_usdrubf import reference_modules as usd_modules
from reaction_fixtures import configs, make_case as synthetic
from reaction_history import (EVIDENCE, SCHEMA, HistoryPlan, PriceContext, configurations,
                              make_case, validate_prices)
from reaction_reference import expected_case, frame_for, label

IMO, USD = os.environ.get("IMOEXF_STATE_HANDOFF"), os.environ.get("USDRUBF_STATE_HANDOFF")


def prices():
    case = synthetic(configs()[2], "weekend_context_no_trades", "mr_take_bo_eod")
    rows = []
    for h in case["history"]:
        b = h["value"]["bar"]
        rows.append(dict(ts=pd.Timestamp(label(b["close_time_utc"])), open=b["o"], high=b["h"],
                         low=b["l"], close=b["close"], volume=b["v"]))
    return pd.concat([pd.DataFrame(rows).set_index("ts"), frame_for(case)]).sort_index()


class HistoricalInputTests(unittest.TestCase):
    def test_raw_invalid_prices_duplicates_grid_and_timezone_fail_closed(self):
        good = prices()
        bads = [pd.concat([good, good.iloc[-1:]]), good.iloc[::-1], good.tz_localize("UTC")]
        for value in (float("nan"), float("inf"), -1.):
            bad = good.copy()
            bad.iloc[0, bad.columns.get_loc("low")] = value
            bads.append(bad)
        bad = good.copy()
        bad.index = bad.index+pd.Timedelta(seconds=1)
        bads.append(bad)
        for bad in bads:
            with self.assertRaises(ParityError):
                validate_prices(bad)

    def test_incomplete_execution_is_not_filled_or_skipped(self):
        f = prices().drop(pd.Timestamp("2026-10-26 23:40"))
        p = PriceContext(f, 7)
        self.assertNotIn("2026-10-26", p.weekdays)
        with self.assertRaises(ParityError):
            make_case(configs()[2], "weekday_only", "2026-10-26", p)

    def test_full_history_selection_ignores_future_service_and_incomplete_weekend(self):
        f = prices()
        service = f.iloc[:1].copy()
        service.index = pd.DatetimeIndex(["2026-10-25 08:50"])
        service[["open", "high", "low", "close"]] = 999.
        f = pd.concat([f, service]).sort_index()
        p = PriceContext(f, 7)
        self.assertEqual(p.prior("2026-10-26", "weekday_only"), ("2026-10-23", "2026-10-22"))
        self.assertEqual(p.prior("2026-10-26", "weekend_context_no_trades"), ("2026-10-25", "2026-10-24"))
        p = PriceContext(f.drop(pd.Timestamp("2026-10-25 18:50")), 7)
        self.assertEqual(p.prior("2026-10-26", "weekend_context_no_trades"), ("2026-10-24", "2026-10-23"))

    def test_pair_keeps_identical_history_execution_and_declares_nonproduction_clock(self):
        p = PriceContext(prices(), 7)
        a, b = [make_case(configs()[2], policy, "2026-10-26", p)
                for policy in ("weekday_only", "weekend_context_no_trades")]
        self.assertEqual(a["history"], b["history"])
        self.assertEqual(a["operations"], b["operations"])
        self.assertIn("historical", a["identity"]["receipt_stream"])
        self.assertIn("not_production", EVIDENCE)
        self.assertEqual(sum(op["kind"] == "restart" for op in a["operations"]), 2)
        for h in a["history"]:
            r = h["value"]
            self.assertEqual(r["received_at_utc"], r["bar"]["close_time_utc"]+600)
            self.assertEqual(r["received_at_utc"], r["durable_at_utc"])


@unittest.skipUnless(IMO and USD, "explicit immutable handoff locations required")
class HistoricalOracleTests(unittest.TestCase):
    def test_seven_frozen_profiles_and_full_raw_anchor_crosscheck(self):
        cfgs = configurations(Path(USD))
        self.assertEqual(len(cfgs), 7)
        self.assertEqual(cfgs[0]["costs"]["rate_per_component_per_side"], 0.)
        with imo_modules(Path(IMO)) as modules:
            p = PriceContext(prices(), 7)
            for policy in ("weekday_only", "weekend_context_no_trades"):
                c = make_case(cfgs[2], policy, "2026-10-26", p)
                result = expected_case(c, modules)
                p.assert_full_raw_context(result["frozen"], c["day"], policy)
                bad = copy.deepcopy(result["frozen"])
                bad["previous"]["close"] += 1.
                with self.assertRaises(ParityError):
                    p.assert_full_raw_context(bad, c["day"], policy)

    def test_historical_coverage_is_rebuilt_from_raw_not_mutually_truncated_traces(self):
        plan = HistoryPlan.__new__(HistoryPlan)
        plan.configs = configurations(Path(USD))[:3]
        plan.prices = {"imoexf_reaction07":PriceContext(prices(), 7)}
        days = ["2026-10-26"]
        f = plan.fixture(days)
        self.assertEqual(f["schema_version"], SCHEMA)
        mutations = []
        bad = copy.deepcopy(f)
        bad["cases"].pop()
        mutations.append(bad)
        bad = copy.deepcopy(f)
        bad["cases"][0]["operations"].pop()
        mutations.append(bad)
        bad = copy.deepcopy(f)
        bad["evidence_kind"] = "synthetic_numerical_reaction_not_production"
        mutations.append(bad)
        with ExitStack() as stack:
            modules = {"imoexf":stack.enter_context(imo_modules(Path(IMO)))}
            for bad in mutations:
                with self.assertRaises(ParityError):
                    plan.expected(bad, days, modules)


if __name__ == "__main__":
    unittest.main()
