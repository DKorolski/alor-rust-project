"""Portable negative tests: no private handoff, pandas, broker or Redis needed."""
from copy import deepcopy
from pathlib import Path
import tempfile
import unittest

from compare import (BUNDLE_VERSION, IDENTITY, SCHEMA_VERSION, TRACE_SCHEMA, ParityError,
                     USDRUBF_SCHEMA_VERSION, USDRUBF_TRACE_SCHEMA,
                     canonical_bytes, compare_traces, file_hash, loads, value_hash)


def fixture_value(spec):
    if isinstance(spec, tuple):
        return None
    if isinstance(spec, dict):
        return {key: fixture_value(value) for key, value in spec.items()}
    if isinstance(spec, list):
        return []
    return {str: "fixture", int: 0, bool: False, float: 100.0}[spec]


def fixture_rows():
    row = fixture_value(TRACE_SCHEMA)
    row["schema_version"] = SCHEMA_VERSION
    row["identity"] = {key: "fixture" for key in IDENTITY}
    row["identity"]["evidence_kind"] = "numerical_history"
    row["input_id"] = "IMOEXF|10m|2026-07-15T07:00:00"
    row["input_sha256"] = "a" * 64
    row["timing"]["bar_ts_local"] = "2026-07-15T07:00:00"
    row["context"]["previous"] = fixture_value(TRACE_SCHEMA["context"]["previous"][1])
    row["context"]["previous2"] = fixture_value(TRACE_SCHEMA["context"]["previous2"][1])
    row["position"] = fixture_value(TRACE_SCHEMA["position"][1])
    row["pending"] = fixture_value(TRACE_SCHEMA["pending"][1])
    row["signals"]["suppression_reasons"] = ["position_active"]
    result = []
    for seq, phase in enumerate(("bar_start", "observed", "bar_end")):
        item = deepcopy(row)
        item.update(event_seq=seq, phase=phase)
        result.append(item)
    return result


class ComparatorTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.expected = self.root / "expected.jsonl"
        self.actual = self.root / "actual.jsonl"
        self.manifest = self.root / "manifest.json"
        self.rows = fixture_rows()
        self.inputs = [{"input_id": self.rows[0]["input_id"], "input_sha256": self.rows[0]["input_sha256"],
                        "bar_ts_local": self.rows[0]["timing"]["bar_ts_local"]}]
        self.write_pair(self.rows, self.rows)

    @staticmethod
    def write_rows(path, rows):
        path.write_bytes(b"".join(canonical_bytes(row) + b"\n" for row in rows))

    def write_pair(self, expected, actual):
        self.write_rows(self.expected, expected)
        self.write_rows(self.actual, actual)
        manifest = {"bundle_version": BUNDLE_VERSION, "trace_schema": self.rows[0]["schema_version"],
                    "identity": self.rows[0]["identity"], "inputs": self.inputs,
                    "inputs_sha256": value_hash(self.inputs),
                    "expected_trace_sha256": file_hash(self.expected)}
        self.manifest.write_bytes(canonical_bytes(manifest))

    def check(self):
        return compare_traces(self.expected, self.actual, self.manifest)

    def test_equal_complete_trace_passes_without_operational_claim(self):
        result = self.check()
        self.assertEqual(result["trace_rows"], 3)
        self.assertEqual(result["inputs"], 1)
        self.assertFalse(result["operational_receipt_restart_certified"])
        self.assertFalse(result["broker_isolation_certified"])

    def test_hidden_state_mutations_fail_even_without_trade_changes(self):
        for path, value in [
            (("context", "previous", "high"), 101.0),
            (("context", "previous2", "close"), 101.0),
            (("counters", "mr_total"), 1),
            (("bo_observation", "count"), 1),
            (("pending", "owner"), "BO"),
            (("position", "side"), "short"),
            (("signals", "suppression_reasons"), ["quota"]),
        ]:
            with self.subTest(path=path):
                actual = deepcopy(self.rows)
                target = actual[-1]
                for key in path[:-1]:
                    target = target[key]
                target[path[-1]] = value
                self.write_pair(self.rows, actual)
                with self.assertRaisesRegex(ParityError, r"row=2 input=.*\$\." + r"\.".join(path)):
                    self.check()

    def test_float_tolerance_only_applies_to_declared_price_fields(self):
        actual = deepcopy(self.rows)
        actual[-1]["position"]["entry_price"] += 5e-9
        self.write_pair(self.rows, actual)
        self.check()
        actual[-1]["position"]["entry_price"] += 1e-6
        self.write_pair(self.rows, actual)
        with self.assertRaisesRegex(ParityError, "entry_price"):
            self.check()
        for bad in (False, 0.0, 1e-9):
            actual = deepcopy(self.rows)
            actual[-1]["counters"]["mr_total"] = bad
            self.write_pair(self.rows, actual)
            with self.assertRaisesRegex(ParityError, "mr_total"):
                self.check()

    def test_missing_extra_duplicate_and_reordered_rows_fail(self):
        for actual in (self.rows[:-1], self.rows + self.rows[-1:], self.rows[::-1],
                       [self.rows[0], self.rows[0], *self.rows[1:]]):
            with self.subTest(actual=len(actual)):
                self.write_pair(self.rows, actual)
                with self.assertRaises(ParityError):
                    self.check()

    def test_matching_incomplete_traces_still_fail_input_coverage(self):
        for rows in ([], self.rows[:-1], [self.rows[0], self.rows[2]]):
            self.write_pair(rows, rows)
            with self.assertRaises(ParityError):
                self.check()
        self.inputs.append({**self.inputs[0], "input_id": "second_input"})
        self.write_pair(self.rows, self.rows)
        with self.assertRaisesRegex(ParityError, "all manifest inputs"):
            self.check()

    def test_matching_invalid_phase_order_fails(self):
        rows = deepcopy(self.rows)
        rows[1]["phase"] = "bar_end"
        rows[2]["phase"] = "observed"
        self.write_pair(rows, rows)
        with self.assertRaisesRegex(ParityError, "phases"):
            self.check()

    def test_missing_or_unknown_fields_fail_including_both_sides(self):
        for key in ("context", "counters", "bo_observation", "pending", "signals"):
            rows = deepcopy(self.rows)
            rows[-1].pop(key)
            self.write_pair(rows, rows)
            with self.assertRaisesRegex(ParityError, "missing/extra"):
                self.check()
        rows = deepcopy(self.rows)
        rows[-1]["secret_uncompared_state"] = 7
        self.write_pair(rows, rows)
        with self.assertRaisesRegex(ParityError, "missing/extra"):
            self.check()

    def test_hash_and_identity_mismatch_fail(self):
        self.expected.write_bytes(self.expected.read_bytes() + b"\n")
        with self.assertRaisesRegex(ParityError, "checksum"):
            self.check()
        for field in ("identity", "input_sha256"):
            actual = deepcopy(self.rows)
            if field == "identity":
                actual[0][field]["config_sha256"] = "other"
            else:
                actual[0][field] = "other"
            self.write_pair(self.rows, actual)
            with self.assertRaises(ParityError):
                self.check()

    def test_input_manifest_duplicate_or_hash_change_fails(self):
        self.inputs.append(deepcopy(self.inputs[0]))
        self.write_pair(self.rows, self.rows)
        with self.assertRaisesRegex(ParityError, "duplicate"):
            self.check()
        self.inputs.pop()
        self.write_pair(self.rows, self.rows)
        manifest = loads(self.manifest.read_text())
        manifest["inputs_sha256"] = "wrong"
        self.manifest.write_bytes(canonical_bytes(manifest))
        with self.assertRaisesRegex(ParityError, "checksum"):
            self.check()

    def test_numerical_trace_cannot_claim_receipt_evidence(self):
        rows = deepcopy(self.rows)
        rows[0]["timing"]["received_ts_utc"] = 12345
        self.write_pair(rows, rows)
        with self.assertRaisesRegex(ParityError, "fabricate"):
            self.check()

    def test_duplicate_json_keys_and_nonfinite_numbers_are_rejected(self):
        for raw in ('{"event_seq": 0, "event_seq": 0}', '{"price": NaN}', '{"price": Infinity}'):
            with self.assertRaises(ParityError):
                loads(raw)
        raw = self.actual.read_text().replace('"entry_price":100.0', '"entry_price":1e999', 1)
        self.actual.write_text(raw)
        with self.assertRaisesRegex(ParityError, "finite"):
            self.check()

    def test_blank_line_is_not_silently_skipped(self):
        self.actual.write_text("\n" + self.actual.read_text())
        with self.assertRaisesRegex(ParityError, "blank"):
            self.check()


class UsdrubfComparatorTests(unittest.TestCase):
    write_rows = staticmethod(ComparatorTests.write_rows)
    write_pair = ComparatorTests.write_pair
    check = ComparatorTests.check

    def setUp(self):
        ComparatorTests.setUp(self)
        for index, old in enumerate(self.rows):
            row = fixture_value(USDRUBF_TRACE_SCHEMA)
            for key in ("identity", "input_id", "input_sha256", "timing", "phase", "event_seq"):
                row[key] = old[key]
            row["schema_version"] = USDRUBF_SCHEMA_VERSION
            for key in ("position", "pending"):
                row[key] = fixture_value(USDRUBF_TRACE_SCHEMA[key][1])
            self.rows[index] = row
        self.write_pair(self.rows, self.rows)

    def test_usdrubf_schema_roundtrip_and_null_mr_quota(self):
        self.assertEqual(self.check()["inputs"], 1)
        self.assertIsNone(self.rows[0]["counters"]["mr_limit"])

    def test_usdrubf_hidden_state_and_window_drift_fail(self):
        for path, value in (
            (("context", "family"), "previous_day"),
            (("context", "gate_return"), .01),
            (("context", "warmup_return_fallback"), True),
            (("observation", "vwap_numerator"), 101.0),
            (("observation", "running_range"), 101.0),
            (("position", "captured_scale"), 101.0),
            (("position", "stop_price"), 76.0),
            (("position", "take_price"), 74.0),
            (("pending", "captured_anchor"), 75.0),
            (("counters", "mr_limit"), 2),
            (("counters", "bo_short_used"), True),
            (("windows", "mr_force_signal"), "10:50:00"),
        ):
            with self.subTest(path=path):
                actual = deepcopy(self.rows)
                actual[-1][path[0]][path[1]] = value
                self.write_pair(self.rows, actual)
                with self.assertRaisesRegex(ParityError, r"\.".join(path)):
                    self.check()

    def test_unknown_schema_and_cross_model_trace_fail(self):
        manifest = loads(self.manifest.read_text())
        manifest["trace_schema"] = "unknown"
        self.manifest.write_bytes(canonical_bytes(manifest))
        with self.assertRaisesRegex(ParityError, "Unsupported trace schema"):
            self.check()
        actual = deepcopy(self.rows)
        actual[0]["schema_version"] = SCHEMA_VERSION
        self.write_pair(self.rows, actual)
        with self.assertRaisesRegex(ParityError, "identity/schema mismatch"):
            self.check()

    def test_usdrubf_equal_truncation_and_future_price_fail(self):
        self.write_pair(self.rows[:-1], self.rows[:-1])
        with self.assertRaisesRegex(ParityError, "phases"):
            self.check()
        actual = deepcopy(self.rows)
        actual[0]["pending"]["entry_price"] = 75.0
        self.write_pair(self.rows, actual)
        with self.assertRaisesRegex(ParityError, "missing/extra"):
            self.check()


if __name__ == "__main__":
    unittest.main()
