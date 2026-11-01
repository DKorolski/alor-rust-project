"""Bounded continuous paired-runner parity + local resource/restart measurements."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path

from compare import ParityError, file_hash, loads
from continuous_reaction import EVIDENCE, CLOCK, fixture, expected, verify_trace
from export_imoexf import reference_modules as imo_modules
from export_usdrubf import reference_modules as usd_modules
from reaction_compare import compare
from reaction_fixtures import identity
from reaction_history import HistoryPlan, model_label
from run_reaction_history import compact, csv_new, source_hashes
from run_ri_fixture_parity import write_new
from process_probe import measured_run, select_profiles


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--imoexf-raw", type=Path, required=True)
    parser.add_argument("--imoexf-handoff", type=Path, required=True)
    parser.add_argument("--usdrubf-handoff", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--start", default="2026-10-19")
    parser.add_argument("--end", default="2026-10-26")
    parser.add_argument("--test-opt-level",type=int,choices=(0,1,2),default=1)
    parser.add_argument("--profiles",nargs="+",help="explicit subset of the five approved paired profiles")
    parser.add_argument("--process-recovery",action="store_true",help="retain owned generations and compare restore in a fresh test process")
    args = parser.parse_args()
    workspace = Path(__file__).resolve().parents[2]
    out = args.output.resolve()
    for path in (args.imoexf_raw.parent, args.imoexf_handoff, args.usdrubf_handoff):
        if out.is_relative_to(path.resolve()): raise ParityError("output inside immutable source")
    sources = source_hashes(workspace)
    references = {str((p/"manifest.json").resolve()):file_hash(p/"manifest.json") for p in (args.imoexf_handoff,args.usdrubf_handoff)}
    plan = HistoryPlan(args.imoexf_raw,args.usdrubf_handoff,args.start,args.end)
    configs = select_profiles(plan.configs[:5],args.profiles,model_label)
    bundles = []
    for cfg in configs:
        prices = plan.prices[identity(cfg,"weekday_only")["profile"]]
        bundles.append((cfg,prices,fixture(cfg,prices,args.start,args.end)))
    out.mkdir()
    write_new(out/"run_contract.json", dict(evidence_kind=EVIDENCE, clock=CLOCK, dates=plan.dates,
        configs=configs,sources=sources,inputs=plan.inputs,references=references,production_receipts_certified=False,
        historical_labels_are_fixture_live=True,redis_poll_certified=False,fee_economics_parity=False,deployment=False,
        test_opt_level=args.test_opt_level,process_recovery=args.process_recovery,
        generations_retained=args.process_recovery))
    cmd = ["cargo","test","--offline","--config",f"profile.test.opt-level={args.test_opt_level}",
           "-p","strategy-runtime","--lib","reaction_continuous_export_actual",
           "--","--ignored","--nocapture","--test-threads=1"]
    results, economics, binaries = [], [], set()
    env = {k:v for k,v in os.environ.items() if k not in ("REDIS_URL","RI_SHADOW_REDIS_URL") and not k.startswith("REACTION_")}
    try:
        if args.process_recovery:
            # Measure the test process itself, excluding Cargo/compiler memory.
            build = cmd[:8]+["--no-run","--message-format=json"]
            with (out/"build_test.log").open("x") as log:
                build_result = measured_run(build,workspace,env,log)
            if build_result["exit_code"]: raise ParityError("candidate build failed")
            executables = set()
            for line in (out/"build_test.log").read_text().splitlines():
                try: message = json.loads(line)
                except ValueError: continue
                if message.get("reason")=="compiler-artifact" and message.get("executable") and message["target"]["name"]=="strategy_runtime":
                    executables.add(message["executable"])
            if len(executables)!=1: raise ParityError("ambiguous test binary")
            cmd = [executables.pop(),"reaction_continuous_export_actual","--ignored","--nocapture","--test-threads=1"]
            write_new(out/"build_measurement.json",build_result)
        with ExitStack() as stack:
            modules = dict(imoexf=stack.enter_context(imo_modules(args.imoexf_handoff)),
                           usdrubf=stack.enter_context(usd_modules(args.usdrubf_handoff)))
            for cfg, prices, f in bundles:
                directory = out/model_label(cfg); directory.mkdir()
                source = directory/"input.json"; want = directory/"expected.jsonl"
                compact(source,f)
                # Oracle never reads candidate output. Original/instrumented model
                # trades/events are checked equal by run_reference on every day.
                with want.open("x") as stream:
                    for row in expected(f,prices,modules[cfg["engine"]]):
                        json.dump(row,stream,separators=(",",":"),allow_nan=False); stream.write("\n")
                env.update(REACTION_CONTINUOUS_INPUT=str(source),REACTION_CONTINUOUS_OUTPUT=str(directory/"candidate"))
                if args.process_recovery: env["REACTION_RETAIN_GENERATION"]="1"
                with (directory/"candidate_test.log").open("x") as log:
                    measurement = measured_run(cmd,workspace,env,log)
                write_new(directory/"process_measurement.json",measurement)
                if measurement["exit_code"]: raise ParityError(f"candidate failed: {directory/'candidate_test.log'}")
                resource = loads((directory/"candidate/report.json").read_text())
                compare("EXPORTED_CONTINUOUS_CANDIDATE",resource["result"])
                compare(EVIDENCE,resource["evidence_kind"])
                compare(file_hash(source),resource["input_sha256"])
                compare(len(plan.dates),resource["completed_sessions"])
                compare(len(f["operations"])-1,resource["last_input_index"])
                for key in ("deployment","production_receipts_certified","redis_poll_certified","numerical_parity_certified"):
                    compare(False,resource[key])
                recovery = None
                if args.process_recovery:
                    restore_cmd=[cmd[0],"reaction_continuous_process_recovery_actual",*cmd[2:]]
                    with (directory/"recovery_test.log").open("x") as log:
                        restore_measurement=measured_run(restore_cmd,workspace,env,log)
                    write_new(directory/"recovery_measurement.json",restore_measurement)
                    if restore_measurement["exit_code"]: raise ParityError("new-process recovery failed")
                    recovered=loads((directory/"candidate/process_recovery.json").read_text())
                    compare("PASS_NEW_PROCESS_RESTORE",recovered["result"])
                    for key in ("input_sha256","binary_sha256","directory_bytes","directory_files"):
                        compare(resource[key],recovered[key])
                    compare(resource["recovery_witness"],recovered["witness"])
                    compare(True,recovered["new_process"]); compare(False,recovered["cold_filesystem_cache"])
                    if measurement["pid"]==restore_measurement["pid"]: raise ParityError("not a new process")
                    recovery=dict(report=recovered,process=restore_measurement)
                # Rebuild coverage from raw prices, not matching two truncated files.
                compare(fixture(cfg,prices,args.start,args.end),loads(source.read_text()))
                with want.open() as stream:
                    rows = []
                    for ref in expected(f,prices,modules[cfg["engine"]]):
                        saved = stream.readline()
                        if not saved: raise ParityError("truncated saved reference")
                        compare(ref,loads(saved),"saved_reference"); rows.append(ref)
                    if stream.read().strip(): raise ParityError("extra saved reference")
                stats,trades = verify_trace(f,rows,directory/"candidate/trace.jsonl")
                compare(len(plan.dates)*101*2,stats["state_rows"])
                compare(len(plan.dates),stats["completed_sessions"]); compare(3,stats["restarts"])
                economics += trades; binaries.add(resource["binary_sha256"])
                paths=[source,want,directory/"candidate/trace.jsonl",directory/"candidate/report.json",directory/"process_measurement.json"]
                if recovery: paths += [directory/"candidate/process_recovery.json",directory/"recovery_measurement.json"]
                hashes = {str(p.relative_to(out)):file_hash(p) for p in paths}
                results.append(dict(model=model_label(cfg),stats=stats,resources={k:v for k,v in resource.items() if k not in ("daily",)},
                    process_measurement=measurement,process_recovery=recovery,artifacts=hashes))
                print(f"PASS {model_label(cfg)}: {stats}, bytes={resource['directory_bytes']}, final_restart_sec={resource['restarts'][-1]['duration_sec']:.3f}",flush=True)
        if len(binaries)!=1 or sources != source_hashes(workspace): raise ParityError("binary/source changed during run")
        for path,digest in {**plan.inputs,**references}.items(): compare(digest,file_hash(Path(path)),"immutable_input")
        for t in economics:
            if t["entry"][:10]!=t["day"] or t["exit"][:10]!=t["day"]: raise ParityError("cross-day round")
    except Exception as exc:
        write_new(out/"failure.json",dict(result="FAIL_CONTINUOUS_REACTION",completed_profiles=len(results),error=str(exc),deployment=False))
        raise
    csv_new(out/"trades.csv",economics)
    report = dict(result="PASS_BOUNDED_CONTINUOUS_PAIRED_REACTION",evidence_kind=EVIDENCE,clock=CLOCK,
        dates=plan.dates,profiles=len(results),arm_sessions=len(plan.dates)*len(results)*2,
        state_rows=sum(r["stats"]["state_rows"] for r in results),rounds=len(economics),
        independent_context_state_decision_comparison=True,uninstrumented_reference_outputs_equal=True,
        deduplicated_event_trade_deltas=True,production_receipts_certified=False,redis_poll_certified=False,
        fee_economics_parity=False,long_generation_capacity_certified=False,vps_capacity_certified=False,
        deployment=False,automatic_promotion=False,test_opt_level=args.test_opt_level,binary_sha256=binaries.pop(),results=results,sources=sources,
        process_recovery_compared=args.process_recovery,generations_retained=args.process_recovery,
        inputs=plan.inputs,references=references,command=cmd,outputs={"trades.csv":file_hash(out/"trades.csv")})
    write_new(out/"verification.json",report)
    print({k:report[k] for k in ("result","profiles","arm_sessions","state_rows","rounds")})


if __name__ == "__main__": main()
