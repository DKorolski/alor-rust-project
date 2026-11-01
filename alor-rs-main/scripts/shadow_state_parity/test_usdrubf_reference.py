"""Opt-in USDRUBF frozen-reference checks; no deployment or broker interaction."""
from contextlib import ExitStack
import os
from pathlib import Path
import unittest

HANDOFF = os.environ.get("USDRUBF_STATE_HANDOFF")


@unittest.skipUnless(HANDOFF, "Set USDRUBF_STATE_HANDOFF to the frozen shortlist directory")
class UsdrubfReferenceTests(unittest.TestCase):
    def setUp(self):
        from export_usdrubf import reference_modules

        stack = ExitStack()
        self.addCleanup(stack.close)
        self.original, self.observed, self.verifier = stack.enter_context(reference_modules(Path(HANDOFF)))
        self.profiles = self.verifier.integrity()["profiles"]

    def bars(self):
        import pandas as pd

        rows = []
        for date in ("2026-07-10", "2026-07-13", "2026-07-14"):
            opening = "09:00" if date < "2026-07-14" else "07:00"
            for i, ts in enumerate(pd.date_range(date+" "+opening, date+" 23:40", freq="10min")):
                close, high, low = 75.0, 76.0, 74.0
                if date == "2026-07-14":
                    high, low = 75.05, 74.95
                    if i in (0, 4, 8):
                        close = 75.02
                    if i in (2, 6, 10):
                        low = 74.6
                    if i == 6:
                        high = 76.0
                    if i == 24:
                        close, high = 76.2, 76.3
                rows.append(dict(ts_local=ts, open=close, high=high, low=low, close=close, volume=100.0))
        return pd.DataFrame(rows).set_index("ts_local")

    def replay(self, raw=None, index=0):
        import pandas as pd
        from compare import BUNDLE_VERSION, USDRUBF_SCHEMA_VERSION, Coverage, value_hash
        from export_usdrubf import Observer

        raw = self.bars() if raw is None else raw
        profile = self.profiles[index]
        cfg = self.observed.Candidate(**profile["engine"])
        rows = []
        observer = Observer(self.original.prepare(raw, cfg.hour), profile, rows.append)
        trades, events = self.observed.replay(raw, cfg, _state_trace=observer)
        original_t, original_e = self.original.replay(raw, self.original.Candidate(**profile["engine"]))
        pd.testing.assert_frame_equal(trades, original_t, check_exact=True)
        pd.testing.assert_frame_equal(events, original_e, check_exact=True)
        coverage = Coverage(dict(bundle_version=BUNDLE_VERSION, trace_schema=USDRUBF_SCHEMA_VERSION,
                                 identity=observer.identity, inputs=observer.inputs,
                                 inputs_sha256=value_hash(observer.inputs)))
        for row in rows:
            coverage.accept(row)
        coverage.finish()
        return rows, trades, events

    def test_all_profiles_preserve_original_actions_and_complete_state(self):
        for index in range(6):
            with self.subTest(profile=index):
                rows, _, _ = self.replay(index=index)
                self.assertIsNone(rows[-1]["position"])
                self.assertIsNone(rows[-1]["pending"])

    def test_no_inherited_mr_cap_and_stop_priority_on_double_touch(self):
        rows, trades, _ = self.replay()
        mr = trades.loc[(trades.component == "MR") & trades.entry_ts.ge("2026-07-14")]
        self.assertEqual(len(mr), 3)
        self.assertEqual(list(mr.exit_reason), ["mr_take", "mr_stop", "mr_take"])
        self.assertTrue(mr.iloc[1].ambiguous_mr_bar)
        self.assertEqual(rows[-1]["counters"]["mr_scheduled_total"], 3)
        self.assertIsNone(rows[-1]["counters"]["mr_limit"])
        self.assertEqual(rows[-1]["counters"]["bo_total"], 1)

    def test_captured_scale_prices_and_no_future_price_in_pending(self):
        rows, _, _ = self.replay()
        pending = [r["pending"] for r in rows if r["pending"] and r["pending"]["role"] == "entry"]
        self.assertTrue(pending)
        for p in pending:
            self.assertFalse(p["proxy_known"])
            self.assertNotIn("entry_price", p)
            self.assertNotIn("take_price", p)
            self.assertLess(p["signal_ts_local"], p["action_ts_local"])
        row = next(r for r in rows if r["timing"]["bar_ts_local"] == "2026-07-14T07:10:00"
                   and r["phase"] == "observed")
        self.assertEqual(row["position"]["captured_scale"], 2.0)
        self.assertAlmostEqual(row["position"]["stop_price"], 75.86)
        self.assertAlmostEqual(row["position"]["take_price"], 74.68)
        self.assertEqual(row["position"]["entry_proxy_known_ts_local"], "2026-07-14T07:20:00")
        self.assertEqual(row["observation"]["count"], 2)
        self.assertEqual(row["position"]["owner"], "MR")

    def test_windows_distinguish_h3_h4_and_wait2_wait4(self):
        for index, force, bo in ((0, "09:50:00", "11:00:00"), (1, "10:50:00", "11:00:00"),
                                 (2, "11:50:00", "11:00:00"), (4, "09:50:00", "09:00:00")):
            rows, _, _ = self.replay(index=index)
            self.assertEqual(rows[-1]["windows"]["mr_force_signal"], force)
            self.assertEqual(rows[-1]["windows"]["bo_first_signal"], bo)
        rows, _, _ = self.replay()
        second = next(r for r in rows if r["timing"]["bar_ts_local"] == "2026-07-13T09:00:00")
        self.assertTrue(second["context"]["warmup_return_fallback"])
        self.assertFalse(second["context"]["two_prior_sessions_available"])
        self.assertEqual(second["context"]["gate_return"], 0.0)
        self.assertFalse(rows[-1]["context"]["warmup_return_fallback"])

    def test_vwap_prefix_and_captured_scale_do_not_follow_future_range(self):
        raw = self.bars()
        raw.loc["2026-07-14 07:00", ["open", "close", "high", "low"]] = [75.01, 75.01, 76., 74.]
        raw.loc["2026-07-14 07:10", "low"] = 73.5
        rows, _, _ = self.replay(raw, index=3)
        entry = next(r for r in rows if r["timing"]["bar_ts_local"] == "2026-07-14T07:10:00"
                     and r["phase"] == "observed")
        self.assertEqual(entry["position"]["captured_scale"], 2.0)
        self.assertEqual(entry["context"]["scale"], 2.5)
        self.assertAlmostEqual(entry["position"]["captured_anchor"], (76+74+75.01)/3)
        raw.loc["2026-07-14 20:00", "high"] = 90.0
        changed, _, _ = self.replay(raw, index=3)
        before = lambda data: [r for r in data if r["timing"]["bar_ts_local"] < "2026-07-14T20:00:00"]
        self.assertEqual(before(rows), before(changed))

    def test_zero_negative_volume_reference_fallback_is_preserved(self):
        raw = self.bars()
        raw.loc["2026-07-14", "volume"] = -1.0
        rows, _, _ = self.replay(raw, index=3)
        current = [r for r in rows if r["timing"]["bar_ts_local"].startswith("2026-07-14")]
        self.assertTrue(current)
        for row in current:
            self.assertEqual(row["observation"]["cumulative_volume"], 0.0)
            self.assertEqual(row["observation"]["vwap_numerator"], 0.0)
            self.assertEqual(row["context"]["mr_anchor"], row["observation"]["vwap"])

    def test_service_weekend_and_early09_bars_cannot_change_state(self):
        import pandas as pd

        raw = self.bars()
        for index in (0, 2, 3):
            rows, _, _ = self.replay(raw, index=index)
            changed = raw.copy()
            for ts in ("2026-07-13 08:50", "2026-07-14 06:50", "2026-07-12 12:00"):
                changed.loc[pd.Timestamp(ts)] = [150., 200., 100., 150., 100000.]
            changed = changed.sort_index()
            if index == 2:
                changed.loc["2026-07-14 07:00":"2026-07-14 08:50", ["open", "high", "low", "close"]] = [150., 200., 100., 150.]
            changed_rows, _, _ = self.replay(changed.sort_index(), index=index)
            self.assertEqual(rows, changed_rows)

    def test_missing_bar_wrapper_and_reference_hash_are_not_bypassed(self):
        import pandas as pd
        from compare import ParityError
        from export_usdrubf import instrument

        with self.assertRaisesRegex(AssertionError, "Incomplete model session"):
            self.replay(self.bars().drop(pd.Timestamp("2026-07-14 12:00")))
        with self.assertRaises(ParityError):
            instrument((Path(HANDOFF)/"reference_model.py").read_text()+"\n")

    def test_terminal_entry_guard_cannot_create_overnight_position(self):
        raw = self.bars()
        raw.loc["2026-07-14", ["open", "high", "low", "close"]] = [75., 75.05, 74.95, 75.]
        raw.loc["2026-07-14 23:20", ["open", "high", "low", "close"]] = [76.2, 76.3, 76.1, 76.2]
        rows, trades, events = self.replay(raw)
        blocked = events.loc[events.role == "entry_suppressed_terminal"]
        self.assertEqual(len(blocked), 1)
        self.assertEqual(str(blocked.iloc[0].signal_ts), "2026-07-14 23:20:00")
        self.assertFalse(trades.entry_ts.ge("2026-07-14").any())
        self.assertIsNone(rows[-1]["position"])
        self.assertIsNone(rows[-1]["pending"])
        self.assertTrue(any(r["signals"]["branch"] == "entry_suppressed_terminal" for r in rows))

    def test_profile_cannot_silently_change_under_frozen_identity(self):
        from dataclasses import replace
        from compare import ParityError
        from export_usdrubf import Observer

        raw = self.bars()
        profile = self.profiles[0]
        cfg = self.observed.Candidate(**profile["engine"])
        observer = Observer(self.original.prepare(raw, cfg.hour), profile, lambda row: None)
        with self.assertRaisesRegex(ParityError, "config does not match"):
            self.observed.replay(raw, replace(cfg, bo_wait=2), _state_trace=observer)

    def test_frozen_sources_never_use_a_bytecode_writing_loader(self):
        from importlib.machinery import SourceFileLoader
        from unittest.mock import patch
        from export_usdrubf import reference_modules

        handoff = Path(HANDOFF).resolve()
        before = set(handoff.rglob("*"))
        get_code = SourceFileLoader.get_code

        def guarded(loader, name):
            if Path(loader.path).resolve().parent == handoff:
                self.fail("Frozen module must be compiled in memory")
            return get_code(loader, name)

        with patch.object(SourceFileLoader, "get_code", guarded):
            with reference_modules(handoff):
                pass
        self.assertEqual(before, set(handoff.rglob("*")))


if __name__ == "__main__":
    unittest.main()
