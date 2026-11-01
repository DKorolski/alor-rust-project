"""Opt-in tests against the private frozen reference; exports are derivative only."""
import os
from contextlib import ExitStack
from pathlib import Path
import unittest

HANDOFF = os.environ.get("IMOEXF_STATE_HANDOFF")


@unittest.skipUnless(HANDOFF, "Set IMOEXF_STATE_HANDOFF to the frozen engineer_handoff directory")
class ReferenceTests(unittest.TestCase):
    def setUp(self):
        from export_imoexf import reference_modules

        stack = ExitStack()
        self.addCleanup(stack.close)
        self.original, self.observed, self.verifier = stack.enter_context(reference_modules(Path(HANDOFF)))

    def synthetic_bars(self, hold_to_sixth=False):
        import pandas as pd

        records = []
        for date in ("2026-07-01", "2026-07-02"):
            for i, label in enumerate(pd.date_range(date + " 09:00", date + " 12:20", freq="10min")):
                close = 1000.0
                if date.endswith("02"):
                    if hold_to_sixth:
                        close = 1002.0 if i < 6 else 998.0 if i == 6 else 1000.0
                    else:
                        close = {0: 1002.0, 1: 998.0, 2: 1002.0, 3: 998.0}.get(i, 1000.0)
                    if i == 18:
                        close = 1040.0
                    elif i == 19:
                        close = 970.0
                records.append({"ts_local": label, "open": close, "close": close,
                                "high": 1020.0 if date.endswith("01") else close + 1,
                                "low": 980.0 if date.endswith("01") else close - 1,
                                "volume": 100.0})
        return pd.DataFrame(records).set_index("ts_local")

    def replay(self, hold=False):
        import pandas as pd
        from export_imoexf import Observer, identity_for

        bars = self.synthetic_bars(hold)
        rows = []
        observer = Observer(bars, identity_for(Path(HANDOFF), "combo"), rows.append)
        cfg = self.observed.Candidate("fixture", True, "hybrid", .53)
        trades, events = self.observed.replay(bars, cfg, _state_trace=observer)
        original_trades, original_events = self.original.replay(bars, cfg)
        pd.testing.assert_frame_equal(trades, original_trades, check_exact=True)
        pd.testing.assert_frame_equal(events, original_events, check_exact=True)
        return rows, trades, events

    def test_state_export_does_not_change_actions_and_tracks_shared_quota(self):
        rows, trades, events = self.replay()
        self.assertEqual(len(trades), 3)
        self.assertEqual(len(events), 6)
        self.assertEqual(rows[-1]["counters"]["mr_total"], 2)
        self.assertEqual(rows[-1]["counters"]["bo_total"], 1)
        self.assertIsNone(rows[-1]["position"])
        self.assertIsNone(rows[-1]["pending"])

    def test_bo_observation_advances_while_mr_owns_the_slot(self):
        rows, _, _ = self.replay(hold=True)
        sixth = next(row for row in rows if row["timing"]["bar_ts_local"] == "2026-07-02T09:50:00"
                     and row["phase"] == "bar_end")
        self.assertEqual(sixth["position"]["owner"], "MR")
        self.assertEqual(sixth["bo_observation"]["count"], 6)
        self.assertEqual(sixth["bo_observation"]["last_ts_local"], "2026-07-02T09:50:00")
        self.assertEqual(sixth["levels"]["bo_entry_long"], 1021.2)
        self.assertIn("position_active", sixth["signals"]["suppression_reasons"])

    def test_pending_does_not_contain_future_fill_price(self):
        rows, _, _ = self.replay()
        pending = [row["pending"] for row in rows if row["pending"]]
        self.assertTrue(pending)
        for state in pending:
            self.assertEqual(set(state), {"role", "owner", "side", "signal_ts_local", "action_ts_local",
                                          "reason", "decision_id", "proxy_known"})
            self.assertFalse(state["proxy_known"])
            self.assertLess(state["signal_ts_local"], state["action_ts_local"])

    def test_instrumentation_rejects_changed_reference(self):
        from compare import ParityError
        from export_imoexf import instrument

        source = (Path(HANDOFF) / "reference_model.py").read_text()
        with self.assertRaises(ParityError):
            instrument(source.replace("mr_entries < 2", "mr_entries < 3"))

    def test_frozen_sources_never_use_a_bytecode_writing_loader(self):
        from importlib.machinery import SourceFileLoader
        from unittest.mock import patch
        from export_imoexf import reference_modules

        handoff = Path(HANDOFF).resolve()
        existing = set(handoff.rglob("*"))
        real_get_code = SourceFileLoader.get_code

        def guarded_get_code(loader, name):
            if Path(loader.path).resolve().parent == handoff:
                self.fail("Frozen handoff must be compiled in memory, not via a caching loader")
            return real_get_code(loader, name)

        with patch.object(SourceFileLoader, "get_code", guarded_get_code):
            with reference_modules(handoff) as (original, observed, verifier):
                self.assertIsNotNone(original.replay)
                self.assertIsNotNone(observed.replay)
                self.assertEqual(verifier.HERE, handoff)
        self.assertEqual(existing, set(handoff.rglob("*")))

    def test_trace_cannot_silently_relabel_another_execution_or_clock_contract(self):
        from compare import ParityError
        from export_imoexf import Observer, identity_for

        bars = self.synthetic_bars()
        cfg = self.observed.Candidate("fixture", True, "hybrid", .53)
        for kwargs in ({"hour": 9}, {"contract": "source_marks"}):
            observer = Observer(bars, identity_for(Path(HANDOFF), "combo"), lambda row: None)
            with self.assertRaisesRegex(ParityError, "reaction07 contract"):
                self.observed.replay(bars, cfg, _state_trace=observer, **kwargs)

    def test_feed_rejects_service_weekend_and_duplicate_inputs(self):
        import pandas as pd
        from compare import ParityError
        from export_imoexf import model_inputs

        bars = self.synthetic_bars()
        for label in ("2026-07-01 08:50", "2026-07-04 10:00", "2026-07-01 09:01"):
            bad = bars.iloc[:1].copy()
            bad.index = pd.DatetimeIndex([label])
            with self.assertRaises(ParityError):
                model_inputs(bad)
        with self.assertRaises(ParityError):
            model_inputs(pd.concat([bars.iloc[:1], bars.iloc[:1]]))

    def test_input_hash_and_prior_context_are_independent_of_future_session(self):
        from export_imoexf import Observer, identity_for

        bars = self.synthetic_bars()
        identity = identity_for(Path(HANDOFF), "combo")
        left, right = [], []
        cfg = self.observed.Candidate("fixture", True, "hybrid", .53)
        self.observed.replay(bars, cfg, _state_trace=Observer(bars, identity, left.append))
        changed = bars.copy()
        changed.loc["2026-07-02", "volume"] = 200.0
        changed.loc["2026-07-02 09:00", "high"] += .25
        self.observed.replay(changed, cfg, _state_trace=Observer(changed, identity, right.append))
        self.assertEqual([row for row in left if row["timing"]["bar_ts_local"].startswith("2026-07-01")],
                         [row for row in right if row["timing"]["bar_ts_local"].startswith("2026-07-01")])


if __name__ == "__main__":
    unittest.main()
