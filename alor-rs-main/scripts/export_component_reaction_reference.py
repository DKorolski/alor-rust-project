"""Pinned Python feature/event/state oracle on small synthetic M10 fixtures.

No prices are downloaded and no research artifact is modified. Research imports
that load caches/plots are avoided by executing the exact selected function bodies;
the compiler's path-only dependency is supplied explicitly, not a model substitute.
"""
from __future__ import annotations

import argparse
import ast
import builtins
import copy
import hashlib
import json
from pathlib import Path
import re
import sys
import types

from export_shadow_calendar_reference import CALENDAR_SHA, SESSIONS_SHA, load_reference, pinned

COMPILER_SHA = "c68fe4b11bcf19eda6a8b38469470ad5594101647435c3ecf96b7a0730fec4f4"
MODEL_SHA = "a469365d54844879ed63108bc99d45be18ace95d261627b57a94ceb1a4c18135"
PROTOCOL_SHA = "1cfdbecbdba680f75771886148ff67d2c94506a49711ab57f1a8f624e2c5a6e1"
OI_IO_SHA = "783ce6a06aa47b4678085ce8e69ca783619ddefa62e16564c7c5e5d6ce444779"
OI_PROTOCOL_SHA = "c3c5bd863a458e5d52c4e995e24cab1f769f09b90c3b7e74b12c42d29f7461b1"
STATE_COLUMNS = (
    "label", "observed_bars", "mr_entries", "used_long", "used_short", "day_high", "day_low",
    "first_high", "first_low", "bo_long_level", "bo_short_level", "author_allowed",
    "position_owner", "position_side", "entry_signal", "entry_at", "entry_price", "bars_held",
    "pending_role", "pending_owner", "pending_side", "pending_signal", "pending_at", "pending_reason",
    "decision_site", "mr_predicate", "bo_long_predicate", "bo_short_predicate",
)


def epoch(value):
    if value is None:
        return None
    import pandas as pd
    return int(pd.Timestamp(value).tz_localize("Europe/Moscow").timestamp())


def function_tree(source, names):
    nodes = [n for n in ast.parse(source).body if isinstance(n, ast.FunctionDef) and n.name in names]
    if len(nodes) != len(names) or {n.name for n in nodes} != set(names):
        raise ValueError("pinned function inventory changed")
    return ast.Module(body=nodes, type_ignores=[])


class TagContinues(ast.NodeTransformer):
    def __init__(self):
        self.parents = []

    def visit_If(self, node):
        self.parents.append(ast.unparse(node.test))
        node = self.generic_visit(node)
        self.parents.pop()
        return node

    def visit_Continue(self, node):
        sites = (("position is not None or pending is not None", "busy_or_invalid"),
                 ("closed_at_close", "same_close_reentry_block"), ("cfg.mr", "mr_selected"),
                 ("cfg.bo is None", "bo_disabled_or_gate"), ("ts < start", "bo_wait_or_range"),
                 ("side is not None and source", "source_mark_bo"), ("side is None", "bo_no_eligible_side"),
                 ("ts.time() >= eod", "bo_terminal"), ("not source and boundary.time() >= eod", "bo_next_open_eod"))
        site = next((name for parent in reversed(self.parents) for prefix, name in sites
                     if parent.startswith(prefix)), None)
        if site is None:
            raise ValueError(f"unclassified original continue: {self.parents}")
        assignment = ast.Assign(targets=[ast.Name(id="__fixture_site", ctx=ast.Store())], value=ast.Constant(site))
        return [ast.copy_location(assignment, node), node]


def instrument(source, filename, predicates):
    tree = ast.parse(source)
    replay = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "replay")
    conditions = [n.test for n in ast.walk(replay) if isinstance(n, ast.If)]
    snippets = ("cfg.mr and valid", "row.close > bpc + cfg.bo_k * bpr", "row.close < bpc - cfg.bo_k * bpr")
    for name, snippet in zip(("mr", "long", "short"), snippets):
        chosen = [n for n in conditions if snippet in ast.unparse(n)]
        if len(chosen) != 1:
            raise ValueError(f"predicate inventory changed: {name}")
        predicates[name] = builtins.compile(ast.Expression(copy.deepcopy(chosen[0])), filename, "eval")
    loops = [n for n in ast.walk(replay) if isinstance(n, ast.For) and ast.unparse(n.target) == "(i, row)"]
    if len(loops) != 1:
        raise ValueError("bar-loop inventory changed")
    loop = loops[0]
    tagged = []
    tagger = TagContinues()
    for node in loop.body:
        changed = tagger.visit(node)
        tagged.extend(changed if isinstance(changed, list) else [changed])
    reset = ast.parse("__fixture_site = None").body[0]
    callback = ast.parse("__fixture_emit(locals())").body[0]
    loop.body = [reset, ast.Try(body=tagged, handlers=[], orelse=[], finalbody=[callback])]
    return builtins.compile(ast.fix_missing_locations(tree), filename, "exec")


def resolve_model(research):
    """Select the unique pinned external model without publishing account paths."""
    relative = "*/analysis/live_baseline_revalidation/imoexf_author_comparison/model.py"
    matches = [path for path in sorted(research.glob(relative))
               if hashlib.sha256(path.read_bytes()).hexdigest() == MODEL_SHA]
    if len(matches) != 1:
        raise ValueError("exactly one pinned external model is required")
    return matches[0]


def load_kernels(research):
    import numpy as np
    cal, session_functions, pd = load_reference(research / "moex_calendar_context_2026_11_15")
    comp = research / "moex_component_context_2026_11_15"
    oi = research / "moex_oi_gate_overnight_2026_11_15"
    model = resolve_model(research)
    pinned(model, MODEL_SHA)
    compiler_source = pinned(comp / "component_models.py", COMPILER_SHA).decode()
    protocol = json.loads(pinned(comp / "protocol.json", PROTOCOL_SHA))
    oi_source = pinned(oi / "research_io.py", OI_IO_SHA).decode()
    oi_protocol = json.loads(pinned(oi / "protocol.json", OI_PROTOCOL_SHA))

    def compiler_env(engine_compile):
        return dict(pd=pd, np=np, re=re, sys=sys, types=types, CFG=protocol,
                    data=types.SimpleNamespace(previous=types.SimpleNamespace(IMO=model.parent)),
                    __engine_compile=engine_compile)

    def load_compiler(source, module, engine_compile):
        tree = function_tree(source, ("build",))
        count = 0
        class ReplaceCompile(ast.NodeTransformer):
            def visit_Call(self, node):
                nonlocal count
                if isinstance(node.func, ast.Name) and node.func.id == "compile":
                    count += 1
                    node.func.id = "__engine_compile"
                return self.generic_visit(node)
        tree = ReplaceCompile().visit(tree)
        if count != 1:
            raise ValueError("compiler execution site changed")
        module.__dict__.update(compiler_env(engine_compile))
        exec(builtins.compile(ast.fix_missing_locations(tree), "pinned-build-function", "exec"), module.__dict__)

    def build(variant, ledgers, gate, traced):
        predicates = {}
        engine_compile = (lambda text, path, mode: instrument(text, path, predicates)) if traced else builtins.compile
        if gate is None:
            compiler = types.ModuleType("fixture_component_compiler")
            load_compiler(compiler_source, compiler, engine_compile)
            result = compiler.build("IMOEXF", variant, ledgers)
        else:
            # Execute the pinned OI compiler wrapper, replacing only its module-
            # load boundary to avoid cache/plot imports. Its gate insertion is unchanged.
            tree = function_tree(oi_source, ("gated_engine",))
            exec_sites = 0
            class ReplaceExec(ast.NodeTransformer):
                def visit_Expr(self, node):
                    nonlocal exec_sites
                    if isinstance(node.value, ast.Call) and isinstance(node.value.func, ast.Name) and node.value.func.id == "exec":
                        exec_sites += 1
                        return ast.copy_location(ast.parse("__load_compiler(source, compiler)").body[0], node)
                    return self.generic_visit(node)
            env = dict(COMP=comp, sys=sys, types=types,
                       __load_compiler=lambda text, module: load_compiler(text, module, engine_compile))
            tree = ReplaceExec().visit(tree)
            if exec_sites != 1:
                raise ValueError("OI compiler load boundary changed")
            exec(builtins.compile(ast.fix_missing_locations(tree), "pinned-oi-wrapper", "exec"), env)
            result = env["gated_engine"](variant, ledgers, gate)
        return result, predicates

    costs = dict(CFG=oi_protocol)
    exec(builtins.compile(function_tree(oi_source, ("leg_cost",)), "pinned-leg-cost", "exec"), costs)
    return cal, session_functions, pd, build, costs["leg_cost"]


def emit_state(module, predicates, records):
    def emit(f):
        p, a = f["position"], f["pending"]
        owner = a.get("component", p["component"] if p else None) if a else None
        side = a.get("side", p["side"] if p else None) if a else None
        evaluate = lambda name: bool(eval(predicates[name], module.__dict__, f))
        records.append([
            epoch(f["ts"]), f["i"]+1, f["mr_entries"], f["was_long"], f["was_short"],
            float(f["day_high"]), float(f["day_low"]), float(f["records"][0].high), float(f["records"][0].low),
            f["long_level"], f["short_level"], bool(f["author_allowed"]),
            p["component"] if p else None, p["side"] if p else None,
            epoch(p["entry_signal_ts"]) if p else None, epoch(p["entry_ts"]) if p else None,
            p["entry_price"] if p else None, p["bars_held"] if p else None,
            a["role"] if a else None, owner, side, epoch(a["signal_ts"]) if a else None,
            epoch(a["when"]) if a else None, a.get("reason") if a else None,
            f["__fixture_site"] or "bo_selected", evaluate("mr"),
            bool(f["bvalid"]) and evaluate("long"), bool(f["bvalid"]) and evaluate("short"),
        ])
    return emit


def replay_reference(pd, build, leg_cost, variant, ledgers, bars, allowed=None):
    traced, predicates = build(variant, ledgers, allowed, True)
    rows = []
    traced.__fixture_emit = emit_state(traced, predicates, rows)
    cfg = traced.Candidate("author41_existing_bo053", True, "hybrid", .53)
    trades, events = traced.replay(bars, cfg, hour=7, contract="reaction_next_open")
    control, _ = build(variant, ledgers, allowed, False)
    ct, ce = control.replay(bars, control.Candidate(cfg.model_id, True, "hybrid", .53),
                            hour=7, contract="reaction_next_open")
    pd.testing.assert_frame_equal(trades, ct)
    pd.testing.assert_frame_equal(events, ce)
    result = dict(expected_state_trace=rows, events=[], trades=[])
    for r in events.to_dict("records"):
        result["events"].append(dict(role=r["role"], ts_utc=epoch(r["ts"]),
            component=r["component"], side=r["side"],
            signal_ts_utc=epoch(r.get("signal_ts")) if pd.notna(r.get("signal_ts")) else None,
            price=float(r["price"]) if pd.notna(r.get("price")) else None))
    for r in trades.to_dict("records"):
        friction = leg_cost(r["entry_price"], "IMOEXF") + leg_cost(r["exit_price"], "IMOEXF")
        result["trades"].append(dict(component=r["component"], side=r["side"],
            entry_ts_utc=epoch(r["entry_ts"]), entry_signal_ts_utc=epoch(r["entry_signal_ts"]),
            entry_price=r["entry_price"], bars_held=r["bars_held"], exit_ts_utc=epoch(r["exit_ts"]),
            exit_signal_ts_utc=epoch(r["exit_signal_ts"]), exit_price=r["exit_price"],
            exit_reason=r["exit_reason"], gross_points=r["gross_points"], friction_rub=friction,
            net_x1_rub=r["gross_points"]*10-friction, net_x2_rub=r["gross_points"]*10-2*friction))
    return result


def export(research):
    cal, sf, pd, build, leg_cost = load_kernels(research)
    first, target = "2026-11-05", "2026-11-09"
    sessions = []
    for day in pd.date_range(first, target):
        s = cal.session(day, "IMOEXF")
        sessions.append(dict(date=str(day.date()), exchange_trade_date=None, kind=s.kind,
            context_open_minute=s.open_minute, close_minute=s.end_minute,
            main_open_minute=s.main_open, main_close_minute=s.main_end,
            breaks=[dict(start_minute=a, end_minute=b) for a,b in s.breaks], note=s.note or s.kind))
    definition = dict(schema_version="moex_shadow_calendar_v1", calendar_version="pinned-component-fixture-20261115-v1",
        reference_sha256=CALENDAR_SHA, symbol="IMOEXF", first_date=first, last_date=target, sessions=sessions)
    contexts, histories = {}, {}
    for name in ("regular", "mr_flat", "bo_flat", "missing_dsvd"):
        frames = []
        for day, price, spread in (("2026-11-05",100.,2.),("2026-11-06",100.,2.),
                                  ("2026-11-07",102.,3.),("2026-11-08",103.,4.)):
            if name == "mr_flat" and day == "2026-11-06": spread = 0.
            if name == "bo_flat" and day == "2026-11-08": spread = 0.
            labels = cal.session(day,"IMOEXF").expected_labels()
            if name == "missing_dsvd" and day == "2026-11-08": labels = labels[:-1]
            frames.append(pd.DataFrame(dict(open=price,high=price+spread,low=price-spread,close=price,volume=1.,origin="live"), index=labels))
        raw = pd.concat(frames).sort_index()
        raw.index.name = "ts_local"
        histories[name] = raw
        contexts[name] = [[epoch(ts),float(r.open),float(r.high),float(r.low),float(r.close),float(r.volume)] for ts,r in raw.iterrows()]

    series = {}
    for name in ("rounds", "mr_hold", "terminal", "stops", "quota", "mr_ambiguous", "breakeven"):
        labels = cal.session(target,"IMOEXF").expected_labels()
        records = []
        for i, ts in enumerate(labels):
            price = 100.
            if name == "rounds":
                price = (100.2,100.2,99.8,100.,100.3,100.3,103.,100.)[i] if i < 8 else (105. if i<36 else (108. if i<48 else (103. if i<60 else (98. if i<72 else 97.))))
            elif name == "mr_hold":
                price = 100.1 if i<2 else (100.3 if i<18 else 102.15)
            elif name == "terminal":
                price = 110. if ts.time() >= pd.Timestamp("23:20").time() else 100.
            elif name == "stops":
                price = 100. if i<18 else (108. if i<23 else (94. if i<30 else 110.))
            elif name == "quota":
                price = (100.2,100.2,99.8,99.8)[i%4] if i<16 else (100. if i<24 else 108.)
            elif name == "mr_ambiguous":
                price = 100.2 if i<2 else 99.8
            elif name == "breakeven":
                price = 100.1 if i<2 else 100.2
            spread = .01
            low = price-spread
            if name == "mr_hold" and i>=2: low = price-.005
            if name == "breakeven" and i>=2: low = 100.05
            high = 103. if name == "mr_ambiguous" and i==2 else price+spread
            records.append(dict(ts=ts,open=price,high=high,low=low,close=price,volume=1.,origin="live"))
        frame = pd.DataFrame(records).set_index("ts")
        frame.index.name="ts_local"
        series[name] = frame

    cases = []
    for context_name, raw in histories.items():
        for series_name, day in series.items():
            if context_name != "regular" and series_name != "rounds": continue
            inventory, _ = sf["inspect_sessions"](pd.concat([raw,day]),"IMOEXF")
            ledgers = {k:sf["build_ledger"](inventory,k=="B") for k in ("A","B")}
            for arm, variant, gate in (("IMO_AA","mr_A_bo_A",None),("IMO_AB","mr_A_bo_B",None),
                ("IMO_AB_OI_OFF","mr_A_bo_B",None),("IMO_AB_OI_TRUE","mr_A_bo_B",True),("IMO_AB_OI_FALSE","mr_A_bo_B",False)):
                m = ledgers["A"].loc[lambda f:f.date.eq(pd.Timestamp(target))].iloc[0]
                b = ledgers["A" if variant=="mr_A_bo_A" else "B"].loc[lambda f:f.date.eq(pd.Timestamp(target))].iloc[0]
                ready = bool(m.ready and b.ready)
                def features(row):
                    if not row.ready: return None
                    return [float(row["prev_"+k]) for k in ("close","high","low","range")]+[float(row.prev2_close),float(row.prev_return)]
                case = dict(id=f"{context_name}/{series_name}/{arm}", context=context_name, series=series_name,
                    arm="IMO_AB_OI" if arm.startswith("IMO_AB_OI") else arm, gate=gate,
                    ready=ready, mr_previous=str(m.prev_date.date()),mr_previous2=str(m.prev2_date.date()),
                    bo_previous=str(b.prev_date.date()),bo_previous2=str(b.prev2_date.date()),
                    mr_inputs=features(m),bo_inputs=features(b),expected_state_trace=[],events=[],trades=[])
                if ready:
                    allowed = None if gate is None else {pd.Timestamp(target): gate}
                    case.update(replay_reference(pd, build, leg_cost, variant, ledgers, day, allowed))
                cases.append(case)
    return dict(schema_version="imoexf_component_reaction_fixture_v1", evidence_kind="synthetic_pinned_reference_not_prospective_pit",
        source_hashes=dict(calendar=CALENDAR_SHA,sessions=SESSIONS_SHA,compiler=COMPILER_SHA,model=MODEL_SHA,
                           component_protocol=PROTOCOL_SHA,oi_io=OI_IO_SHA,oi_protocol=OI_PROTOCOL_SHA),
        definition=definition, history_columns=["ts_utc","o","h","l","close","v"], histories=contexts,
        series={name:[[epoch(ts),float(r.open),float(r.high),float(r.low),float(r.close),float(r.volume)] for ts,r in f.iterrows()]
                for name,f in series.items()}, state_columns=STATE_COLUMNS, cases=cases)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--research",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    args=parser.parse_args()
    value=export(args.research)
    with args.output.open("x",encoding="utf-8") as f:
        json.dump(value,f,separators=(",",":"),allow_nan=False)
        f.write("\n")
    print(json.dumps(dict(cases=len(value["cases"]),states=sum(len(c["expected_state_trace"]) for c in value["cases"]))))


if __name__ == "__main__": main()
