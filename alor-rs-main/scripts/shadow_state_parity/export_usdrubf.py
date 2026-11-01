"""Read-only numerical state export for the frozen USDRUBF reaction shortlist."""
from __future__ import annotations

import argparse
import ast
from contextlib import contextmanager
from dataclasses import asdict
import gzip
import hashlib
import io
import json
import math
from pathlib import Path
import sys
import types

import pandas as pd

from compare import (BUNDLE_VERSION, USDRUBF_SCHEMA_VERSION, Coverage, ParityError,
                     canonical_bytes, compare_traces, file_hash, loads, value_hash)

REFERENCE_SHA256 = "3c0546815af66802986bc30e224d4aaae78443464dc4eba0bb5191e62dcd4d20"
CONTRACT_SHA256 = "91d182b04f813505f2ae6259039bfd8b22c8197084a6698eac399d9c79ce5a5d"
BRANCHES = {
    148: "'pending_active' if pending is not None else 'position_active'",
    152: "'invalid_scale_or_price'",
    163: "'no_eligible_candidate'",
    168: "'entry_suppressed_terminal'",
}


def observer_call(phase):
    return ast.parse(f"_state_trace({phase!r}, locals())").body[0]


def instrument(source):
    if hashlib.sha256(source.encode()).hexdigest() != REFERENCE_SHA256:
        raise ParityError("Reference changed: re-review USDRUBF observer sites")
    tree = ast.parse(source)
    functions = [node for node in tree.body if isinstance(node, ast.FunctionDef)]
    features = next(node for node in functions if node.name == "features")
    features.args.kwonlyargs.append(ast.arg(arg="_features_trace"))
    features.args.kw_defaults.append(ast.Constant(value=None))
    features.body.insert(-1, ast.parse(
        "if _features_trace is not None:\n    _features_trace(locals())"
    ).body[0])
    core, wrapper = [node for node in functions if node.name == "replay"]
    for function in (core, wrapper):
        function.args.kwonlyargs.append(ast.arg(arg="_state_trace"))
        function.args.kw_defaults.append(None)
    feature_call = next(node for node in ast.walk(core)
                        if isinstance(node, ast.Call) and node.lineno == 82)
    feature_call.keywords.append(ast.keyword(
        arg="_features_trace", value=ast.parse("_state_trace.capture_features", mode="eval").body))
    wrapper.body[-1].value.keywords.append(ast.keyword(
        arg="_state_trace", value=ast.Name(id="_state_trace", ctx=ast.Load())))
    loop = next(node for node in ast.walk(core) if isinstance(node, ast.For) and node.lineno == 92)
    seen = set()

    class BranchObserver(ast.NodeTransformer):
        def visit_Continue(self, node):
            if node.lineno not in BRANCHES:
                raise ParityError(f"Unclassified reference branch: {node.lineno}")
            seen.add(node.lineno)
            return [ast.parse(f"_state_trace_branch = {BRANCHES[node.lineno]}").body[0], node]

    BranchObserver().visit(loop)
    if seen != set(BRANCHES):
        raise ParityError("Missing reference branch observation site")
    body, sites = [], set()
    for node in loop.body[1:]:
        if isinstance(node, ast.If) and node.lineno == 94:
            node.body.append(observer_call("pending_applied"))
            sites.add("pending")
        if isinstance(node, ast.If) and node.lineno == 125:
            body.append(observer_call("observed"))
            sites.add("observed")
        body.append(node)
    if sites != {"pending", "observed"}:
        raise ParityError("Missing reference state transition site")
    body.append(ast.parse("_state_trace_branch = component.lower() + '_selected'").body[0])
    loop.body = [loop.body[0], ast.parse("_state_trace_branch = 'not_evaluated'").body[0],
                 observer_call("bar_start"),
                 ast.Try(body=body, handlers=[], orelse=[], finalbody=[observer_call("bar_end")])]
    return ast.fix_missing_locations(tree)


@contextmanager
def reference_modules(handoff):
    handoff = handoff.resolve()
    if sys.flags.optimize:
        raise ParityError("Do not run with -O: frozen assertions are acceptance checks")
    manifest = loads((handoff / "manifest.json").read_text())
    for name, expected in manifest["file_sha256"].items():
        path = handoff / name
        if path.resolve().parent != handoff or not path.is_file() or file_hash(path) != expected:
            raise ParityError(f"Frozen manifest mismatch or unsafe path: {name}")
    for name, expected in (("reference_model.py", REFERENCE_SHA256),
                           ("shadow_contract.json", CONTRACT_SHA256)):
        if file_hash(handoff / name) != expected:
            raise ParityError(f"Unsupported frozen version: {name}")
    names = ("reference_model", "_usdrubf_state_reference", "_usdrubf_state_verifier")
    saved = {name: sys.modules.get(name) for name in names}
    try:
        modules = []
        for index, name in enumerate(names):
            path = handoff / ("verify_bundle.py" if index == 2 else "reference_model.py")
            source = path.read_text(encoding="utf-8")
            module = types.ModuleType(name)
            module.__file__ = str(path)
            sys.modules[name] = module
            # Never use a caching file loader inside the immutable handoff.
            exec(compile(instrument(source) if index == 1 else source, str(path), "exec"), module.__dict__)
            modules.append(module)
        modules[2].integrity()
        yield tuple(modules)
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


def decision_id(owner, side, role, signal, action):
    return value_hash(dict(owner=owner, side=side, role=role,
                           signal_ts_local=stamp(signal), action_ts_local=stamp(action)))


def model_inputs(bars):
    if bars.empty or not bars.index.is_unique or not bars.index.is_monotonic_increasing:
        raise ParityError("Expected nonempty unique ordered M10 inputs")
    result = []
    for row in bars.itertuples():
        ts = row.Index
        if ts.tzinfo is not None or ts.minute % 10 or ts.second or ts.microsecond or ts.nanosecond:
            raise ParityError("Expected naive Europe/Moscow M10 labels")
        values = {key: float(getattr(row, key)) for key in ("open", "high", "low", "close", "volume")}
        if (not all(math.isfinite(value) for value in values.values()) or row.low <= 0
                or not row.low <= row.open <= row.high or not row.low <= row.close <= row.high):
            raise ParityError("Invalid OHLCV input")
        # Negative volume is deliberately not rejected: the reference clips it to zero.
        label = stamp(ts)
        result.append(dict(input_id=f"USDRUBF|10m|{label}", bar_ts_local=label,
                           input_sha256=value_hash(dict(ts_local=label, **values))))
    return result


def identity_for(profile):
    engine = profile["engine"]
    hour = engine["context_hour"]
    return dict(model_id=engine["model_id"], symbol="USDRUBF", timeframe="10m",
                timezone="Europe/Moscow", evidence_kind="numerical_history",
                feed_policy=f"regular_weekdays_transition09_to{hour:02d}_2349",
                context_policy=engine["family"] + "_weekday_only",
                execution_policy="closed_bar_next_open_market_v1",
                config_sha256=profile["config_hash"], reference_sha256=REFERENCE_SHA256)


class Observer:
    def __init__(self, frame, profile, sink):
        self.profile, self.identity, self.sink = profile, identity_for(profile), sink
        self.inputs = model_inputs(frame)
        self.by_label = {item["bar_ts_local"]: item for item in self.inputs}
        self.daily_inputs = {}
        for item in self.inputs:
            self.daily_inputs.setdefault(item["bar_ts_local"][:10], []).append(item)
        self.dates = list(self.daily_inputs)
        self.contexts = {}
        self.seq, self.mr_scheduled_total = 0, 0
        self.current_day = None

    def capture_features(self, state):
        # These are the actual frozen engine intermediates, not a second feature engine.
        names = ("day_open", "scale_now", "numerator", "cumulative_volume", "vwap")
        self.features = pd.DataFrame({name: state[name] for name in names}).to_dict("index")
        daily = state["daily"]
        for index, day in enumerate(self.dates):
            prev = self.dates[index - 1] if index else None
            prev2 = self.dates[index - 2] if index > 1 else None
            previous = None
            if prev is not None:
                data = daily.loc[pd.Timestamp(prev)]
                previous = dict(session_date=prev, high=float(data.high), low=float(data.low),
                                close=float(data.close), range=float(data.high-data.low),
                                bar_count=len(self.daily_inputs[prev]))
            previous2 = None if prev2 is None else dict(
                session_date=prev2, close=float(daily.loc[pd.Timestamp(prev2), "close"]))
            provenance = [self.daily_inputs[d] for d in (prev2, prev) if d is not None]
            self.contexts[day] = dict(previous=previous, previous2=previous2,
                previous_return=finite(state["prior"].loc[pd.Timestamp(day), "return"]),
                two_prior_sessions_available=prev2 is not None,
                provenance_sha256=value_hash(provenance))

    def __call__(self, phase, state):
        cfg, row = state["cfg"], state["row"]
        if asdict(cfg) != self.profile["engine"]:
            raise ParityError("Observer config does not match the frozen profile identity")
        ts, day = row.Index, str(state["date"].date())
        if self.current_day != day:
            self.current_day, self.mr_scheduled_total = day, 0
        branch = state["_state_trace_branch"]
        if phase == "bar_end" and branch == "mr_selected":
            self.mr_scheduled_total += 1
        raw_position, raw_pending = state["position"], state["pending"]
        position = None
        if raw_position is not None:
            p = raw_position
            position = dict(owner=p["component"], side=p["side"], entry_ts_local=stamp(p["entry_ts"]),
                entry_signal_ts_local=stamp(p["entry_signal_ts"]), entry_price=float(p["entry_price"]),
                entry_decision_id=decision_id(p["component"], p["side"], "entry", p["entry_signal_ts"], p["entry_ts"]),
                entry_proxy_known_ts_local=stamp(p["entry_ts"] + pd.Timedelta(minutes=10)),
                captured_scale=float(p["scale"]), captured_anchor=float(p["anchor"]),
                stop1=float(p["stop1"]), stop2=float(p["stop2"]),
                stop_price=finite(p.get("stop_price")), take_price=finite(p.get("take_price")))
        pending = None
        if raw_pending is not None:
            p = raw_pending
            owner = p.get("component") or raw_position["component"]
            side = p.get("side") or raw_position["side"]
            signal = p["entry_signal_ts"] if p["role"] == "entry" else p["signal_ts"]
            pending = dict(role=p["role"], owner=owner, side=side, signal_ts_local=stamp(signal),
                action_ts_local=stamp(p["action_ts"]), reason=p.get("reason", owner.lower()+"_entry"),
                decision_id=decision_id(owner, side, p["role"], signal, p["action_ts"]),
                proxy_known=False, ambiguous_mr_bar=bool(p.get("ambiguous_mr_bar", False)),
                captured_scale=finite(p.get("scale")), captured_anchor=finite(p.get("anchor")),
                stop1=finite(p.get("stop1")), stop2=finite(p.get("stop2")))
        f = self.features[ts]
        context = self.contexts[day]
        scale = finite(row.scale)
        valid = scale is not None and scale > 0 and row.close > 0
        self.sink(dict(
            schema_version=USDRUBF_SCHEMA_VERSION, identity=self.identity, event_seq=self.seq,
            input_id=self.by_label[stamp(ts)]["input_id"], input_sha256=self.by_label[stamp(ts)]["input_sha256"],
            phase=phase,
            timing=dict(bar_ts_local=stamp(ts), decision_ready_ts_local=stamp(state["boundary"]),
                        context_cutoff_ts_local=stamp(state["date"] + pd.Timedelta(hours=state["opening"])),
                        received_ts_utc=None, processed_ts_utc=None),
            context=dict(**context, family=cfg.family, mr_anchor=finite(row.mr_anchor),
                         bo_anchor=finite(row.bo_anchor), scale=scale, gate_return=float(row.gate_return),
                         warmup_return_fallback=cfg.family == "previous_day" and context["previous_return"] is None),
            observation=dict(count=state["i"]+1, first_ts_local=stamp(state["records"][0].Index),
                             last_ts_local=stamp(ts), day_open=float(f["day_open"]),
                             running_range=float(f["scale_now"]), vwap_numerator=float(f["numerator"]),
                             cumulative_volume=float(f["cumulative_volume"]), vwap=float(f["vwap"])),
            windows=dict(session_open=f'{state["opening"]:02d}:00:00',
                         mr_first_signal=str(state["first_signal"]), mr_last_signal=str(state["cutoff"]),
                         mr_force_signal=str(state["force"]), bo_first_signal=f'{state["opening"]+cfg.bo_wait:02d}:00:00',
                         bo_terminal_action="23:30:00"),
            counters=dict(session_date=day, mr_scheduled_total=self.mr_scheduled_total, mr_limit=None,
                          bo_total=len(state["used"]), bo_long_used="long" in state["used"],
                          bo_short_used="short" in state["used"]),
            position=position, pending=pending,
            signals=dict(branch=branch,
                mr_price_setup=bool(valid and .006 < scale/row.close < .050 and 0 < row.close-row.mr_anchor < .035*scale),
                bo_long_price_setup=bool(valid and row.gate_return >= -.020 and row.close > row.bo_anchor+.45*scale),
                bo_short_price_setup=bool(valid and row.gate_return <= .020 and row.close < row.bo_anchor-.45*scale),
                suppression_reasons=[] if branch in ("not_evaluated", "mr_selected", "bo_selected") else [branch]),
        ))
        self.seq += 1


def export_bundle(handoff, output):
    handoff, output = handoff.resolve(), output.resolve()
    if output == handoff or handoff in output.parents or output.exists():
        raise ParityError("Output must be a NEW directory outside the frozen handoff")
    with reference_modules(handoff) as (original, observed, verifier):
        spec = verifier.integrity()
        baseline_t, baseline_e = verifier.reference_outputs()
        verifier.assert_parity(pd.read_csv(handoff / "expected_trades.csv"),
                               pd.read_csv(handoff / "expected_events.csv"), baseline_t, baseline_e)
        raw = pd.read_csv(handoff / "raw_model_input.csv", parse_dates=["ts_local"]).set_index("ts_local")
        raw_inputs = model_inputs(raw)
        output.mkdir(parents=True)
        checks, actual_t, actual_e = [], [], []
        for profile in spec["profiles"]:
            cfg = observed.Candidate(**profile["engine"])
            frame = original.prepare(raw, cfg.hour)
            inputs = model_inputs(frame)
            accepted = {item["input_id"] for item in inputs}
            rejected = [item for item in raw_inputs if item["input_id"] not in accepted]
            manifest = dict(bundle_version=BUNDLE_VERSION, trace_schema=USDRUBF_SCHEMA_VERSION,
                identity=identity_for(profile), inputs=inputs, inputs_sha256=value_hash(inputs),
                exporter_sha256=file_hash(Path(__file__)), comparator_sha256=file_hash(Path(__file__).with_name("compare.py")),
                model_input_file_sha256=file_hash(handoff / "raw_model_input.csv"),
                raw_input_count=len(raw_inputs), raw_inputs_sha256=value_hash(raw_inputs),
                excluded_inputs=rejected, excluded_inputs_sha256=value_hash(rejected),
                frozen_contract_sha256=CONTRACT_SHA256)
            directory = output / cfg.model_id
            directory.mkdir()
            expected = directory / "expected_state_trace.jsonl.gz"
            coverage = Coverage(manifest)
            with expected.open("wb") as file:
                with gzip.GzipFile(filename="", fileobj=file, mode="wb", mtime=0) as compressed:
                    with io.TextIOWrapper(compressed, encoding="utf-8") as stream:
                        def sink(row):
                            coverage.accept(row)
                            stream.write(canonical_bytes(row).decode()+"\n")

                        observer = Observer(frame, profile, sink)
                        trades, events = observed.replay(raw, cfg, _state_trace=observer)
            coverage.finish()
            reference_t, reference_e = original.replay(raw, original.Candidate(**profile["engine"]))
            pd.testing.assert_frame_equal(reference_t, trades, check_exact=True)
            pd.testing.assert_frame_equal(reference_e, events, check_exact=True)
            actual_t.append(original.account(trades.loc[trades.entry_ts.ge("2026-07-14")].copy()))
            actual_e.append(events.loc[events.ts.ge("2026-07-14")].copy())
            manifest.update(expected_trace_sha256=file_hash(expected), trace_rows=observer.seq,
                            instrumentation_output_parity=True)
            manifest_path = directory / "manifest.json"
            manifest_path.write_bytes(canonical_bytes(manifest)+b"\n")
            check = compare_traces(expected, expected, manifest_path)
            check.update(model_id=cfg.model_id, check="reference_integrity_not_rust_parity")
            checks.append(check)
        verifier.assert_parity(baseline_t, baseline_e, pd.concat(actual_t, ignore_index=True),
                               pd.concat(actual_e, ignore_index=True))
        report = dict(result="PASS", frozen_trade_rows=len(baseline_t), frozen_event_rows=len(baseline_e),
                      checks=checks, rust_state_parity_certified=False, receipt_restart_certified=False,
                      broker_isolation_certified=False, deployment=False)
        (output / "verification.json").write_text(json.dumps(report, indent=2)+"\n")
        return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--handoff", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New directory outside frozen handoff")
    args = parser.parse_args()
    print(json.dumps(export_bundle(args.handoff, args.output), indent=2))


if __name__ == "__main__":
    main()
