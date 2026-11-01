import copy
import json
from pathlib import Path
import tempfile
import unittest

from compare import ParityError, file_hash
from ri_fixtures import bar, warmup
from ri_history import EVIDENCE, SCHEMA, cutoff, expected_rows, route, validate_fixture
from ri_reference import POLICIES, context
from run_ri_history_parity import verify


def fixture(day="2026-10-13", events=None):
    bars = warmup(day, gap=True) + (events if events is not None else [
        bar(day, "07:00", 89900), bar(day, "07:10", 90200), bar(day, "23:00", 90200)])
    return dict(schema_version=SCHEMA, evidence_kind=EVIDENCE, streams=[dict(
        symbol="RIZ6", source_sha256="a"*64, first_decision_session=day,
        inputs=[dict(bar=b, available_epoch=b["close_time_utc"]+600) for b in bars])])


class RiHistoryTests(unittest.TestCase):
    def test_old_session_anchors_and_service_weekend_exclusion(self):
        bars = warmup("2026-07-14")
        c = context(bars, POLICIES[0])
        self.assertEqual(c["previous"]["date"], "2026-07-13")
        self.assertEqual(c["previous"]["bars"], 88)
        self.assertEqual(c["previous2"]["date"], "2026-07-10")
        noisy = bars + [bar("2026-07-13", "08:50", 99999), bar("2026-07-12", "10:00", 99999)]
        self.assertEqual(context(noisy, POLICIES[1]), c)
        self.assertEqual(cutoff("2026-07-13"), bar("2026-07-13", "09:00", 1)["close_time_utc"])
        self.assertEqual(cutoff("2026-07-14"), bar("2026-07-14", "07:00", 1)["close_time_utc"])

    def test_delayed_closing_before_cutoff_vs_after_cutoff(self):
        f = fixture()
        gap = f["streams"][0]["inputs"][200]
        gap["available_epoch"] = cutoff("2026-10-13")-1
        rows = expected_rows(f)
        before = [r for r in rows if r["policy"] == POLICIES[1] and r["observed"]][0]
        self.assertEqual(before["state"]["context"]["previous"]["close"], 90500.)
        gap["available_epoch"] = cutoff("2026-10-13")+1
        # First event arrives at 07:10, but its context must still exclude late history.
        self.assertEqual(route(gap, POLICIES[1]), "quarantined:at_or_after_cutoff")
        after = [r for r in expected_rows(f) if r["policy"] == POLICIES[1] and r["observed"]][0]
        self.assertEqual(after["state"]["context"]["previous"]["close"], 90000.)

    def test_weekend_and_service_are_audited_without_observation(self):
        f = fixture()
        inputs = f["streams"][0]["inputs"]
        for b in [bar("2026-10-11", "10:00", 99999), bar("2026-10-13", "06:50", 99999)]:
            inputs.append(dict(bar=b, available_epoch=b["close_time_utc"]+600))
        inputs.sort(key=lambda i:(i["available_epoch"], i["bar"]["close_time_utc"]))
        rows = expected_rows(f)
        skipped = [r for r in rows if r["route"] == "quarantined:non_model_bar"]
        self.assertEqual(len(skipped), 4)
        self.assertTrue(all(not r["observed"] for r in skipped))

    def test_no_silent_flat_reset_on_next_day_or_truncated_tail(self):
        f = fixture(events=[bar("2026-10-13", "07:00", 89900)])
        with self.assertRaisesRegex(ParityError, "end of"):
            expected_rows(f)
        b = bar("2026-10-14", "07:00", 90000)
        f["streams"][0]["inputs"].append(dict(bar=b, available_epoch=b["close_time_utc"]+600))
        with self.assertRaisesRegex(ParityError, "2026-10-14"):
            expected_rows(f)

    def test_duplicate_order_nonfinite_and_wrong_identity_fail(self):
        for change in ("duplicate", "order", "nan", "symbol", "evidence"):
            f = fixture()
            items = f["streams"][0]["inputs"]
            if change == "duplicate":
                items.append(copy.deepcopy(items[-1]))
            elif change == "order":
                items[0], items[1] = items[1], items[0]
            elif change == "nan":
                items[0]["bar"]["close"] = float("nan")
            elif change == "symbol":
                items[0]["bar"]["symbol"] = "RIU6"
            else:
                f["evidence_kind"] = "original_durable_receipts"
            with self.subTest(change=change), self.assertRaises(ParityError):
                validate_fixture(f)

    def test_comparator_rederives_coverage_and_rejects_state_route_mutations(self):
        f = fixture()
        rows = expected_rows(f)
        for change in ("state", "route", "coverage", "recovery"):
            actual = dict(schema_version="ri_numerical_history_actual_v1", evidence_kind=EVIDENCE,
                          adapter_version="ri_frozen_prospective_shadow_v2", input_sha256="",
                          binary_sha256="a"*64, recovery_verified=True, rows=copy.deepcopy(rows))
            expected = rows
            if change == "state":
                next(r for r in actual["rows"] if r["observed"])["state"]["mr_total"] += 1
            elif change == "route":
                actual["rows"][0]["route"] = "duplicate_quote"
            elif change == "coverage":
                expected, actual["rows"] = rows[:-1], actual["rows"][:-1]
            else:
                actual["recovery_verified"] = False
            with tempfile.TemporaryDirectory() as d:
                source, ep, ap = (Path(d)/p for p in ("input.json", "expected.json", "actual.json"))
                source.write_text(json.dumps(f))
                actual["input_sha256"] = file_hash(source)
                ep.write_text(json.dumps(expected))
                ap.write_text(json.dumps(actual))
                with self.subTest(change=change), self.assertRaises(ParityError):
                    verify(source, ep, ap)


if __name__ == "__main__":
    unittest.main()
