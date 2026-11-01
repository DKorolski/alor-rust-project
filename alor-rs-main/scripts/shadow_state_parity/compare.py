"""Strict numerical state-trace gate. No broker, Redis or runtime dependencies."""
from __future__ import annotations

import argparse
import gzip
import hashlib
import itertools
import json
import math
from pathlib import Path

SCHEMA_VERSION = "imoexf_reaction_state_v1"
BUNDLE_VERSION = "shadow_state_bundle_v1"
ATOL = 1e-8


class ParityError(ValueError):
    pass


def nullable(spec):
    return ("nullable", spec)


IDENTITY = {name: str for name in (
    "model_id", "symbol", "feed_policy", "context_policy", "execution_policy",
    "config_sha256", "reference_sha256", "timeframe", "timezone", "evidence_kind",
)}
TRACE_SCHEMA = {
    "schema_version": str, "identity": IDENTITY, "event_seq": int,
    "input_id": str, "input_sha256": str, "phase": str,
    "timing": {
        "bar_ts_local": str, "decision_ready_ts_local": str,
        "received_ts_utc": nullable(int), "processed_ts_utc": nullable(int),
        "context_cutoff_ts_local": str,
    },
    "context": {
        "status": str, "previous": nullable({
            "session_date": str, "high": float, "low": float, "close": float,
            "range": float, "bar_count": int,
        }),
        "previous2": nullable({"session_date": str, "close": float}),
        "scale": nullable(float), "previous_return": nullable(float),
        "provenance_sha256": str, "completeness": str,
    },
    "counters": {"session_date": str, "mr_total": int, "mr_limit": int,
                 "bo_total": int, "bo_long_used": bool, "bo_short_used": bool},
    "bo_observation": {
        "count": int, "first_ts_local": nullable(str), "last_ts_local": nullable(str),
        "day_high": nullable(float), "day_low": nullable(float),
    },
    "position": nullable({
        "owner": str, "side": str, "entry_ts_local": str,
        "entry_signal_ts_local": str, "entry_price": float, "bars_held": int,
        "entry_decision_id": str, "entry_proxy_known_ts_local": str,
    }),
    "pending": nullable({
        "role": str, "owner": str, "side": str, "signal_ts_local": str,
        "action_ts_local": str, "reason": str,
        "decision_id": str, "proxy_known": bool,
    }),
    "levels": {name: nullable(float) for name in (
        "mr_entry_upper", "mr_take_close_below", "mr_stop_touch_above",
        "bo_entry_long", "bo_entry_short", "bo_stop1_long", "bo_stop1_short",
        "bo_stop2_long", "bo_stop2_short",
    )},
    "signals": {
        "branch": str, "mr_price_setup": bool,
        "bo_long_price_setup": bool, "bo_short_price_setup": bool,
        "suppression_reasons": [str],
    },
}

# Separate schemas keep model-specific semantics explicit (notably MR quotas).
USDRUBF_SCHEMA_VERSION = "usdrubf_reaction_state_v1"
USDRUBF_TRACE_SCHEMA = {
    **{key: TRACE_SCHEMA[key] for key in (
        "schema_version", "identity", "event_seq", "input_id", "input_sha256", "phase", "timing",
    )},
    "context": {
        "family": str, "previous": TRACE_SCHEMA["context"]["previous"],
        "previous2": TRACE_SCHEMA["context"]["previous2"],
        "mr_anchor": nullable(float), "bo_anchor": nullable(float),
        "scale": nullable(float), "gate_return": float,
        "previous_return": nullable(float), "two_prior_sessions_available": bool,
        "warmup_return_fallback": bool, "provenance_sha256": str,
    },
    "observation": {
        "count": int, "first_ts_local": str, "last_ts_local": str,
        "day_open": float, "running_range": float, "vwap_numerator": float,
        "cumulative_volume": float, "vwap": float,
    },
    "windows": {key: str for key in (
        "session_open", "mr_first_signal", "mr_last_signal", "mr_force_signal",
        "bo_first_signal", "bo_terminal_action",
    )},
    "counters": {
        "session_date": str, "mr_scheduled_total": int, "mr_limit": nullable(int),
        "bo_total": int, "bo_long_used": bool, "bo_short_used": bool,
    },
    "position": nullable({
        "owner": str, "side": str, "entry_ts_local": str,
        "entry_signal_ts_local": str, "entry_price": float,
        "entry_decision_id": str, "entry_proxy_known_ts_local": str,
        "captured_scale": float, "captured_anchor": float,
        "stop1": float, "stop2": float, "stop_price": nullable(float),
        "take_price": nullable(float),
    }),
    "pending": nullable({
        "role": str, "owner": str, "side": str, "signal_ts_local": str,
        "action_ts_local": str, "reason": str, "decision_id": str,
        "proxy_known": bool, "ambiguous_mr_bar": bool,
        "captured_scale": nullable(float), "captured_anchor": nullable(float),
        "stop1": nullable(float), "stop2": nullable(float),
    }),
    "signals": TRACE_SCHEMA["signals"],
}
SCHEMAS = {SCHEMA_VERSION: TRACE_SCHEMA, USDRUBF_SCHEMA_VERSION: USDRUBF_TRACE_SCHEMA}


def canonical_bytes(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def value_hash(value):
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


def file_hash(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ParityError(f"Duplicate JSON key: {key}")
        result[key] = value
    return result


def _nonfinite(value):
    raise ParityError(f"Non-finite JSON number: {value}")


def loads(text):
    return json.loads(text, object_pairs_hook=_unique_object, parse_constant=_nonfinite)


def read_jsonl(path):
    opener = gzip.open if str(path).endswith(".gz") else open
    with opener(path, "rt", encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if not line.strip():
                raise ParityError(f"{path}:{line_number}: blank trace row")
            try:
                yield loads(line)
            except (ValueError, TypeError) as error:
                raise ParityError(f"{path}:{line_number}: {error}") from error


def validate(value, spec, path="$"):
    if isinstance(spec, tuple):
        if value is not None:
            validate(value, spec[1], path)
    elif isinstance(spec, dict):
        if type(value) is not dict or value.keys() != spec.keys():
            raise ParityError(f"{path}: missing/extra fields; required={sorted(spec)}")
        for key, field in spec.items():
            validate(value[key], field, f"{path}.{key}")
    elif isinstance(spec, list):
        if type(value) is not list:
            raise ParityError(f"{path}: expected array")
        for index, item in enumerate(value):
            validate(item, spec[0], f"{path}[{index}]")
    elif spec is float:
        if type(value) not in (int, float) or not math.isfinite(value):
            raise ParityError(f"{path}: expected finite number")
    elif type(value) is not spec:
        raise ParityError(f"{path}: expected {spec.__name__}, got {type(value).__name__}")


def compare_value(expected, actual, spec, path="$"):
    if isinstance(spec, tuple):
        if expected is None or actual is None:
            if expected != actual:
                raise ParityError(f"{path}: expected {expected!r}, actual {actual!r}")
        else:
            compare_value(expected, actual, spec[1], path)
    elif isinstance(spec, dict):
        for key, field in spec.items():
            compare_value(expected[key], actual[key], field, f"{path}.{key}")
    elif isinstance(spec, list):
        if len(expected) != len(actual):
            raise ParityError(f"{path}: array length mismatch")
        for index, (left, right) in enumerate(zip(expected, actual)):
            compare_value(left, right, spec[0], f"{path}[{index}]")
    elif spec is float:
        if abs(expected - actual) > ATOL:
            raise ParityError(f"{path}: expected {expected!r}, actual {actual!r}")
    elif expected != actual:
        raise ParityError(f"{path}: expected {expected!r}, actual {actual!r}")


class Coverage:
    """Coverage comes from the input manifest, not only equality of two traces."""

    def __init__(self, manifest):
        if manifest.get("bundle_version") != BUNDLE_VERSION:
            raise ParityError("Unsupported bundle version")
        self.schema_version = manifest.get("trace_schema")
        if self.schema_version not in SCHEMAS:
            raise ParityError("Unsupported trace schema")
        self.schema = SCHEMAS[self.schema_version]
        self.inputs = manifest["inputs"]
        if not self.inputs or len({row["input_id"] for row in self.inputs}) != len(self.inputs):
            raise ParityError("Empty or duplicate input manifest")
        for row in self.inputs:
            validate(row, {"input_id": str, "input_sha256": str, "bar_ts_local": str})
        if value_hash(self.inputs) != manifest["inputs_sha256"]:
            raise ParityError("Input manifest checksum mismatch")
        self.identity = manifest["identity"]
        validate(self.identity, IDENTITY)
        if self.identity["evidence_kind"] != "numerical_history":
            raise ParityError("This schema does not certify operational receipt/restart traces")
        self.input_index = -1
        self.phases = []
        self.rows = 0

    def end_input(self):
        if self.phases not in (["bar_start", "observed", "bar_end"],
                               ["bar_start", "pending_applied", "observed", "bar_end"]):
            raise ParityError(f"Incomplete/duplicate/reordered phases: {self.phases}")

    def accept(self, row):
        validate(row, self.schema)
        if row["schema_version"] != self.schema_version or row["identity"] != self.identity:
            raise ParityError("Trace identity/schema mismatch")
        if row["event_seq"] != self.rows:
            raise ParityError(f"Non-contiguous event sequence at row {self.rows}")
        if row["phase"] == "bar_start":
            if self.input_index >= 0:
                self.end_input()
            self.input_index += 1
            self.phases = []
        if not 0 <= self.input_index < len(self.inputs):
            raise ParityError("Missing/extra input row")
        item = self.inputs[self.input_index]
        if (row["input_id"], row["input_sha256"], row["timing"]["bar_ts_local"]) != (
            item["input_id"], item["input_sha256"], item["bar_ts_local"],
        ):
            raise ParityError(f"Input order/hash mismatch at input {self.input_index}")
        if any(row["timing"][key] is not None for key in ("received_ts_utc", "processed_ts_utc")):
            raise ParityError("Numerical reference cannot fabricate receipt/processing evidence")
        self.phases.append(row["phase"])
        self.rows += 1

    def finish(self):
        self.end_input()
        if self.input_index + 1 != len(self.inputs):
            raise ParityError("Trace does not cover all manifest inputs")


def compare_traces(expected_path, actual_path, manifest_path):
    manifest = loads(Path(manifest_path).read_text(encoding="utf-8"))
    if file_hash(expected_path) != manifest["expected_trace_sha256"]:
        raise ParityError("Expected trace checksum mismatch")
    expected_coverage, actual_coverage = Coverage(manifest), Coverage(manifest)
    sentinel = object()
    for index, pair in enumerate(itertools.zip_longest(
        read_jsonl(expected_path), read_jsonl(actual_path), fillvalue=sentinel,
    )):
        left, right = pair
        try:
            if left is sentinel or right is sentinel:
                raise ParityError("Missing/extra trace rows")
            expected_coverage.accept(left)
            actual_coverage.accept(right)
            compare_value(left, right, expected_coverage.schema)
        except ParityError as error:
            input_id = left.get("input_id") if isinstance(left, dict) else "end_of_trace"
            raise ParityError(f"row={index} input={input_id}: {error}") from error
    expected_coverage.finish()
    actual_coverage.finish()
    return {"result": "PASS", "trace_rows": expected_coverage.rows,
            "inputs": len(expected_coverage.inputs), "scope": "numerical_state_trace",
            "operational_receipt_restart_certified": False,
            "broker_isolation_certified": False, "deployment": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--expected", type=Path, required=True)
    parser.add_argument("--actual", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = compare_traces(args.expected, args.actual, args.manifest)
    except (ParityError, KeyError, ValueError, OSError) as error:
        print(json.dumps({"result": "FAIL", "error": str(error)}))
        raise SystemExit(1) from error
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
