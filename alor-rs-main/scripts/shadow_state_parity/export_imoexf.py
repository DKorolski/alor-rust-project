"""Export state from the hash-pinned IMOEXF reference without editing its bundle.

Read-only observer calls are injected into a copy of the reference AST. No signal,
state assignment, price or branching expression is replaced. The resulting trades
and events must still equal both the uninstrumented replay and frozen CSVs.
"""
from __future__ import annotations

import argparse
import ast
from contextlib import contextmanager
import gzip
import io
import json
import math
from pathlib import Path
import sys
import types

import pandas as pd

from compare import (BUNDLE_VERSION, SCHEMA_VERSION, Coverage, ParityError,
                     canonical_bytes, compare_traces, file_hash, loads, value_hash)

REFERENCE_SHA256 = "a469365d54844879ed63108bc99d45be18ace95d261627b57a94ceb1a4c18135"
BRANCHES = {
    167: "'context_not_ready' if not valid else 'pending_active' if pending is not None else 'position_active'",
    170: "'same_close_reentry_blocked'",
    177: "'mr_selected'",
    179: "'bo_disabled'",
    183: "'bo_wait' if ts < start+pd.Timedelta(hours=3) else 'bo_range_too_small'",
    190: "'bo_selected'",
    202: "'bo_no_eligible_side'",
    205: "'bo_terminal_or_missing_next_bar'",
    208: "'bo_next_open_at_eod'",
}


def observer_call(phase):
    return ast.parse(f"_state_trace({phase!r}, locals())").body[0]


def instrument(source):
    """Fail closed if the audited reference or any instrumentation site changes."""
    import hashlib

    if hashlib.sha256(source.encode()).hexdigest() != REFERENCE_SHA256:
        raise ParityError("Reference changed: re-review observer sites before exporting traces")
    tree = ast.parse(source)
    replay = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == "replay")
    replay.args.kwonlyargs.append(ast.arg(arg="_state_trace"))
    replay.args.kw_defaults.append(None)
    bar_loop = next(node for node in ast.walk(replay) if isinstance(node, ast.For) and node.lineno == 93)
    seen = set()

    class BranchObserver(ast.NodeTransformer):
        def visit_Continue(self, node):
            if node.lineno not in BRANCHES:
                raise ParityError(f"Unclassified reference branch: {node.lineno}")
            seen.add(node.lineno)
            return [ast.parse(f"_state_trace_branch = {BRANCHES[node.lineno]}").body[0], node]

    BranchObserver().visit(bar_loop)
    if seen != set(BRANCHES):
        raise ParityError("Missing reference branch observation site")
    body = []
    sites = set()
    for node in bar_loop.body[2:]:
        if isinstance(node, ast.If) and node.lineno == 96:
            node.body.append(observer_call("pending_applied"))
            sites.add("pending")
        body.append(node)
        if isinstance(node, ast.If) and node.lineno == 112:
            body.append(observer_call("observed"))
            sites.add("observation")
    if sites != {"pending", "observation"}:
        raise ParityError("Missing state transition observation site")
    body.append(ast.parse("_state_trace_branch = 'bo_selected'").body[0])
    bar_loop.body = [
        *bar_loop.body[:2],
        ast.parse("_state_trace_branch = 'not_evaluated'").body[0],
        observer_call("bar_start"),
        ast.Try(body=body, handlers=[], orelse=[], finalbody=[observer_call("bar_end")]),
    ]
    return ast.fix_missing_locations(tree)


@contextmanager
def reference_modules(handoff):
    manifest = loads((handoff / "manifest.json").read_text())
    for name, expected in manifest["file_sha256"].items():
        path = handoff / name
        if path.parent != handoff or not path.is_file() or file_hash(path) != expected:
            raise ParityError(f"Frozen manifest mismatch or unsafe path: {name}")
    if file_hash(handoff / "reference_model.py") != REFERENCE_SHA256:
        raise ParityError("Unsupported frozen reference version")
    if sys.flags.optimize:
        raise ParityError("Do not run with -O: frozen reference assertions are acceptance checks")
    names = ("reference_model", "_imoexf_state_reference", "_imoexf_state_verifier")
    saved = {name: sys.modules.get(name) for name in names}
    try:
        source = (handoff / "reference_model.py").read_text(encoding="utf-8")
        original = types.ModuleType(names[0])
        original.__file__ = str(handoff / "reference_model.py")
        sys.modules[names[0]] = original
        # SourceFileLoader may write __pycache__ into the immutable handoff.
        exec(compile(source, original.__file__, "exec"), original.__dict__)
        observed = types.ModuleType(names[1])
        observed.__file__ = original.__file__
        sys.modules[names[1]] = observed
        exec(compile(instrument(source), str(handoff / "reference_model.py"), "exec"), observed.__dict__)
        verifier = types.ModuleType(names[2])
        verifier.__file__ = str(handoff / "verify_reference.py")
        sys.modules[names[2]] = verifier
        exec(compile((handoff / "verify_reference.py").read_text(encoding="utf-8"),
                     verifier.__file__, "exec"), verifier.__dict__)
        yield original, observed, verifier
    finally:
        for name, module in saved.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module


def stamp(value):
    return pd.Timestamp(value).isoformat(timespec="seconds")


def finite(value):
    return float(value) if value is not None and math.isfinite(value) else None


def decision_id(owner, side, role, signal_ts, action_ts):
    return value_hash({"owner": owner, "side": side, "role": role,
                       "signal_ts_local": stamp(signal_ts), "action_ts_local": stamp(action_ts)})


def model_inputs(bars):
    if bars.empty or not bars.index.is_unique or not bars.index.is_monotonic_increasing:
        raise ParityError("Expected nonempty unique ordered model bars")
    result = []
    for row in bars.itertuples():
        if row.Index.tzinfo is not None:
            raise ParityError("Expected naive Europe/Moscow model labels")
        opening_hour = 9 if row.Index < pd.Timestamp("2026-07-14") else 7
        if (row.Index.dayofweek >= 5
                or row.Index.hour < opening_hour or (row.Index.hour == 23 and row.Index.minute > 49)
                or row.Index.minute % 10 or row.Index.second or row.Index.microsecond or row.Index.nanosecond):
            raise ParityError("Input violates the frozen regular 10m weekday feed")
        values = [float(getattr(row, key)) for key in ("open", "high", "low", "close", "volume")]
        if (not all(math.isfinite(value) for value in values) or row.low <= 0 or row.volume < 0
                or not row.low <= row.open <= row.high or not row.low <= row.close <= row.high):
            raise ParityError("Invalid OHLCV input")
        label = stamp(row.Index)
        payload = {"ts_local": label, **{key: float(getattr(row, key))
                                       for key in ("open", "high", "low", "close", "volume")}}
        result.append({"input_id": f"IMOEXF|10m|{label}", "input_sha256": value_hash(payload),
                       "bar_ts_local": label})
    return result


class Observer:
    def __init__(self, bars, identity, sink):
        self.identity, self.sink = identity, sink
        self.inputs = model_inputs(bars)
        self.by_label = {item["bar_ts_local"]: item for item in self.inputs}
        self.daily_inputs = {}
        for item in self.inputs:
            self.daily_inputs.setdefault(item["bar_ts_local"][:10], []).append(item)
        self.dates = list(self.daily_inputs)
        self.seq = 0

    def __call__(self, phase, state):
        cfg = state["cfg"]
        if (state["contract"] != "reaction_next_open" or state["hour"] != 7
                or state["daily_close_overrides"] or not cfg.mr
                or cfg.bo not in (None, "hybrid") or cfg.bo_k != .53):
            raise ParityError("State schema supports only the frozen MR/combo reaction07 contract")
        row, date, ctx = state["row"], state["date"], state["ctx"]
        label, day = stamp(row.Index), str(date.date())
        current = self.by_label[label]
        day_index = self.dates.index(day)
        previous_date = self.dates[day_index - 1] if day_index else None
        previous2_date = self.dates[day_index - 2] if day_index > 1 else None
        prior_inputs = self.daily_inputs.get(previous_date, [])
        previous = None if previous_date is None else {
            "session_date": previous_date, "high": float(ctx.prev_high),
            "low": float(ctx.prev_low), "close": float(ctx.prev_close),
            "range": float(ctx.prev_range), "bar_count": len(prior_inputs),
        }
        previous2 = None if previous2_date is None else {
            "session_date": previous2_date, "close": float(ctx.prev2_close),
        }
        pc, pr, pl, ret = state["pc"], state["pr"], state["pl"], state["ret"]
        count = state["i"] + (phase in ("observed", "bar_end"))
        raw_position, raw_pending = state["position"], state["pending"]
        position = None if raw_position is None else {
            "owner": raw_position["component"], "side": raw_position["side"],
            "entry_ts_local": stamp(raw_position["entry_ts"]),
            "entry_signal_ts_local": stamp(raw_position["entry_signal_ts"]),
            "entry_price": raw_position["entry_price"], "bars_held": raw_position["bars_held"],
            "entry_decision_id": decision_id(raw_position["component"], raw_position["side"], "entry",
                                             raw_position["entry_signal_ts"], raw_position["entry_ts"]),
            "entry_proxy_known_ts_local": stamp(raw_position["entry_ts"] + pd.Timedelta(minutes=10)),
        }
        pending = None
        if raw_pending is not None:
            owner = raw_pending.get("component") or raw_position["component"]
            side = raw_pending.get("side") or raw_position["side"]
            pending = {"role": raw_pending["role"], "owner": owner, "side": side,
                       "signal_ts_local": stamp(raw_pending["signal_ts"]),
                       "action_ts_local": stamp(raw_pending["when"]),
                       "reason": raw_pending.get("reason", f"{owner.lower()}_entry"),
                       "decision_id": decision_id(owner, side, raw_pending["role"],
                                                  raw_pending["signal_ts"], raw_pending["when"]),
                       "proxy_known": False}
        branch = state["_state_trace_branch"]
        suppression = [] if branch in ("not_evaluated", "mr_selected", "bo_selected") else [branch]
        cutoff_hour = 9 if day < "2026-07-14" else state["hour"]
        result = {
            "schema_version": SCHEMA_VERSION, "identity": self.identity,
            "event_seq": self.seq, "input_id": current["input_id"],
            "input_sha256": current["input_sha256"], "phase": phase,
            "timing": {"bar_ts_local": label, "decision_ready_ts_local": stamp(state["boundary"]),
                       "received_ts_utc": None, "processed_ts_utc": None,
                       "context_cutoff_ts_local": f"{day}T{cutoff_hour:02d}:00:00"},
            "context": {
                "status": "ready" if state["valid"] else "not_ready",
                "previous": previous, "previous2": previous2,
                "scale": finite(pr / pl) if state["valid"] else None,
                "previous_return": finite(ret),
                "provenance_sha256": value_hash(prior_inputs + self.daily_inputs.get(previous2_date, [])),
                "completeness": "reference_input_only_not_operationally_certified",
            },
            "counters": {"session_date": day, "mr_total": state["mr_entries"], "mr_limit": 2,
                         "bo_total": int(state["was_long"]) + int(state["was_short"]),
                         "bo_long_used": state["was_long"], "bo_short_used": state["was_short"]},
            "bo_observation": {
                "count": count, "first_ts_local": stamp(state["start"]) if count else None,
                "last_ts_local": stamp(state["records"][count - 1].Index) if count else None,
                "day_high": finite(state["day_high"]), "day_low": finite(state["day_low"]),
            },
            "position": position, "pending": pending,
            "levels": {
                "mr_entry_upper": finite(pc + .16 * pr), "mr_take_close_below": finite(pc - .020 * pr),
                "mr_stop_touch_above": finite(pc + .58 * pr),
                "bo_entry_long": finite(pc + state["cfg"].bo_k * pr),
                "bo_entry_short": finite(pc - state["cfg"].bo_k * pr),
                "bo_stop1_long": finite(pc + .51 * pr), "bo_stop1_short": finite(pc - .51 * pr),
                "bo_stop2_long": finite(pc - .35 * pr), "bo_stop2_short": finite(pc + .35 * pr),
            },
            "signals": {
                "branch": branch,
                "mr_price_setup": bool(state["valid"] and .005 < pr / pl < .075 and pc < row.close < pc + .16 * pr),
                "bo_long_price_setup": bool(state["valid"] and not (math.isfinite(ret) and ret < -.025)
                                            and row.close > pc + state["cfg"].bo_k * pr),
                "bo_short_price_setup": bool(state["valid"] and not (math.isfinite(ret) and ret > .025)
                                             and row.close < pc - state["cfg"].bo_k * pr),
                "suppression_reasons": suppression,
            },
        }
        self.sink(result)
        self.seq += 1


def identity_for(handoff, sleeve):
    return {
        "model_id": f"imoexf_author41_short_reaction07_v1/{sleeve}", "symbol": "IMOEXF",
        "feed_policy": "frozen_regular_weekday_transition_09_to_07",
        "context_policy": "previous_weekday_reference_history",
        "execution_policy": "reaction_next_open", "config_sha256": file_hash(handoff / "shadow_contract.json"),
        "reference_sha256": REFERENCE_SHA256, "timeframe": "10m", "timezone": "Europe/Moscow",
        "evidence_kind": "numerical_history",
    }


def export_bundle(handoff, output):
    handoff, output = Path(handoff).resolve(), Path(output).resolve()
    if output == handoff or handoff in output.parents:
        raise ParityError("Derivative artifacts must not be written into the frozen bundle")
    with reference_modules(handoff) as (original, observed, verifier):
        contract = verifier.check_manifest()
        baseline_trades, baseline_events = verifier.expected_replay()
        verifier.assert_parity(pd.read_csv(handoff / "expected_baseline_trades.csv"),
                               pd.read_csv(handoff / "expected_baseline_events.csv"),
                               baseline_trades, baseline_events)
        bars = pd.read_csv(handoff / "model_bars_07.csv", parse_dates=["ts_local"]).set_index("ts_local")
        inputs = model_inputs(bars)
        output.mkdir(parents=True, exist_ok=False)
        checks = []
        for sleeve in ("mr", "combo"):
            identity = identity_for(handoff, sleeve)
            manifest = {
                "bundle_version": BUNDLE_VERSION, "trace_schema": SCHEMA_VERSION,
                "identity": identity, "inputs": inputs, "inputs_sha256": value_hash(inputs),
                "reference_manifest_sha256": file_hash(handoff / "manifest.json"),
                "exporter_sha256": file_hash(Path(__file__)),
                "comparator_sha256": file_hash(Path(__file__).with_name("compare.py")),
                "model_input_file_sha256": file_hash(handoff / "model_bars_07.csv"),
            }
            directory = output / sleeve
            directory.mkdir()
            expected = directory / "expected_state_trace.jsonl.gz"
            coverage = Coverage(manifest)
            with expected.open("wb") as raw:
                with gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as compressed:
                    with io.TextIOWrapper(compressed, encoding="utf-8") as stream:
                        def sink(row):
                            coverage.accept(row)
                            stream.write(canonical_bytes(row).decode() + "\n")

                        observer = Observer(bars, identity, sink)
                        cfg = observed.Candidate(f"k016_t020_s058_{sleeve}", True,
                                                 "hybrid" if sleeve == "combo" else None, .53)
                        trades, events = observed.replay(bars, cfg, _state_trace=observer)
            coverage.finish()
            reference_cfg = original.Candidate(cfg.model_id, cfg.mr, cfg.bo, cfg.bo_k)
            original_trades, original_events = original.replay(bars, reference_cfg)
            pd.testing.assert_frame_equal(original_trades, trades, check_exact=True)
            pd.testing.assert_frame_equal(original_events, events, check_exact=True)
            manifest["expected_trace_sha256"] = file_hash(expected)
            manifest["trace_rows"] = observer.seq
            manifest["instrumentation_output_parity"] = True
            manifest_path = directory / "manifest.json"
            manifest_path.write_bytes(canonical_bytes(manifest) + b"\n")
            result = compare_traces(expected, expected, manifest_path)
            result.update(sleeve=sleeve, check="reference_integrity_not_rust_parity")
            checks.append(result)
        report = {"result": "PASS", "reference_model": contract["model_id"],
                  "frozen_trade_rows": len(baseline_trades), "frozen_event_rows": len(baseline_events),
                  "checks": checks, "rust_state_parity_certified": False,
                  "receipt_restart_certified": False, "deployment": False}
        (output / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
        return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--handoff", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New directory outside frozen handoff")
    args = parser.parse_args()
    print(json.dumps(export_bundle(args.handoff, args.output), indent=2))


if __name__ == "__main__":
    main()
