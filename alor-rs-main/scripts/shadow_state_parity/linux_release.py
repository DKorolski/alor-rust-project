"""Pinned-source Linux release fixture probe; NOT a deploy or VPS capacity gate."""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

from process_probe import measured_run

PROFILES = ("imoexf_combo", "usdrubf_previous_day07_h4_w4_shadow_v1")
CRATES = ("alor-types", "alor-protocol", "alor-gateway", "strategy-runtime", "standalone-cws-stability-probe")
SCHEMA = "reaction_linux_release_probe_v1"


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for block in iter(lambda: f.read(65536), b""):
            h.update(block)
    return h.hexdigest()


def read(path):
    with Path(path).open() as f:
        return json.load(f)


def write(path, value):
    with Path(path).open("x") as f:
        json.dump(value, f, indent=2, allow_nan=False)
        f.write("\n")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def checked_copy(source, target, limit, expected=None):
    require(source.is_file() and not source.is_symlink(), "regular source file required")
    require(source.stat().st_size <= limit, "source size limit")
    before = digest(source)
    require(expected is None or before == expected, "accepted input hash mismatch")
    target.parent.mkdir(parents=True, exist_ok=True)
    with source.open("rb") as src, target.open("xb") as dst:
        shutil.copyfileobj(src, dst, 65536)
    require(digest(target) == before == digest(source), "source changed during snapshot")
    return before


def source_files(workspace):
    paths = [workspace / "Cargo.toml", workspace / "Cargo.lock"]
    for crate in CRATES:
        paths.append(workspace / crate / "Cargo.toml")
        paths.extend((workspace / crate / "src").rglob("*.rs"))
    # The library tests compile this fixed public fixture and the two examples.
    paths += [workspace / "strategy-runtime/tests/fixtures/ri_canonical07_transition_2026_07_10_15.csv"]
    paths += list((workspace / "configs/examples").glob("reaction-shadow.*.toml"))
    paths += [workspace / "scripts/shadow_state_parity" / n for n in ("linux_release.py", "process_probe.py")]
    require(len(paths) <= 2000, "source file-count bound")
    return sorted(paths)


def source_provenance(workspace, sources):
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=workspace)

    head = git("rev-parse", "HEAD").decode().strip()
    prefix = git("rev-parse", "--show-prefix").decode().strip()
    committed = set(git("ls-tree", "-rz", "--name-only", "--full-tree", head).decode().split("\0"))
    unmatched = []
    for name, expected in sources.items():
        path = prefix + name
        if path not in committed or hashlib.sha256(git("show", head + ":" + path)).hexdigest() != expected:
            unmatched.append(name)
    clean = not git("status", "--porcelain", "--untracked-files=all")
    require(git("rev-parse", "HEAD").decode().strip() == head, "HEAD changed during provenance check")
    return dict(git_head=head, git_worktree_clean=clean,
                uncommitted_source_paths=sorted(unmatched),
                source_kind=("manifest_pinned_clean_commit_snapshot" if clean and not unmatched else
                             "manifest_pinned_dirty_worktree_snapshot_not_scoped_commit"))


def prepare(workspace, evidence, output):
    workspace, evidence, output = workspace.resolve(), evidence.resolve(), output.resolve()
    require(not output.exists() and not output.is_relative_to(workspace)
            and not output.is_relative_to(evidence), "fresh external output required")
    prior = read(evidence / "verification.json")
    require(prior["result"] == "PASS_BOUNDED_CONTINUOUS_PAIRED_REACTION", "accepted reference required")
    results = {r["model"]: r for r in prior["results"]}
    require(set(results) == set(PROFILES), "explicit two-profile evidence required")
    selected = source_files(workspace)
    before = source_provenance(workspace, {str(p.relative_to(workspace)): digest(p) for p in selected})
    output.mkdir()
    sources, fixtures = {}, {}
    for file in selected:
        require(file.resolve().is_relative_to(workspace), "source escaped workspace")
        name = str(file.relative_to(workspace))
        sources[name] = checked_copy(file, output / "source" / name, 8 * 1024 * 1024)
    require(sum((output / "source" / n).stat().st_size for n in sources) <= 64 * 1024 * 1024,
            "source snapshot too large")
    for name in PROFILES:
        for filename in ("input.json", "expected.jsonl"):
            relative = name + "/" + filename
            fixtures[relative] = checked_copy(evidence / relative, output / "fixtures" / relative,
                                              64 * 1024 * 1024, results[name]["artifacts"][relative])
    require(all(digest(workspace / n) == h for n, h in sources.items()), "workspace changed during prepare")
    provenance = source_provenance(workspace, sources)
    require(before == provenance, "source provenance changed during snapshot")
    manifest = dict(schema=SCHEMA, **provenance,
                    sources=sources, fixtures=fixtures, reference_verification_sha256=digest(evidence / "verification.json"),
                    expected_stats={n:results[n]["stats"] for n in PROFILES}, dates=prior["dates"],
                    deployment=False, vps_capacity_certified=False, production_receipts_certified=False)
    write(output / "manifest.json", manifest)
    print(json.dumps(dict(snapshot=str(output), files=len(sources), profiles=list(PROFILES)), indent=2))


def validate_snapshot(bundle):
    manifest = read(bundle / "manifest.json")
    require(manifest["schema"] == SCHEMA, "wrong probe schema")
    require(manifest["sources"], "empty source manifest")
    require(set(manifest["fixtures"]) == {p + "/" + f for p in PROFILES
            for f in ("input.json", "expected.jsonl")}, "incomplete fixture manifest")
    require(set(manifest["expected_stats"]) == set(PROFILES), "incomplete profile manifest")
    for category, dirname in (("sources", "source"), ("fixtures", "fixtures")):
        for relative, expected in manifest[category].items():
            p = Path(relative)
            require(not p.is_absolute() and ".." not in p.parts, "snapshot path escape")
            path = bundle / dirname / p
            require(path.is_file() and all(not item.is_symlink() for item in
                    (path, *path.parents)), "invalid snapshot file")
            require(digest(path) == expected, "snapshot hash mismatch: " + relative)
    return manifest


def validate_release_metadata(bins, report, restored, exported, recovery):
    require(bins["target"] == "x86_64-unknown-linux-gnu" and bins["profile"] == "release"
            and bins["debug_assertions"] is False, "Linux release binary required")
    for item in (report, restored):
        require(item["os"] == "linux" and item["arch"] == "x86_64"
                and item["debug_assertions"] is False, "Linux release execution required")
        require(item["production_receipts_certified"] is False
                and item["deployment"] is False, "fixture scope required")
    require(restored["new_process"] is True and restored["cold_filesystem_cache"] is False,
            "new process is not cold filesystem evidence")
    status = restored["witness"]["status"]
    require(status["order_emission_allowed"] is False and status["session_complete"] is True,
            "complete shadow-only generation required")
    for item in (exported, recovery):
        require(item["exit_code"] == 0 and item["pid"] > 0 and item["maxrss_bytes"] > 0
                and math.isfinite(item["wall_sec"]) and item["wall_sec"] > 0,
                "successful measured execution required")
        require(item["cold_filesystem_cache"] is False, "unproven cold filesystem claim")
    require(exported["pid"] != recovery["pid"], "distinct exporter/restore processes required")


def inner(bundle, output, registry):
    require(platform.system() == "Linux", "Linux probe only")
    manifest = validate_snapshot(bundle)
    require(not output.exists(), "fresh result directory required")
    output.mkdir()
    workspace = bundle / "source"
    env = dict(os.environ, CARGO_HOME="/cargo", CARGO_TARGET_DIR="/probe/target", CARGO_NET_OFFLINE="true")
    for key in list(env):
        if "REDIS" in key or key.startswith("REACTION_"):
            del env[key]
    # A private ephemeral copy avoids mutating the host's Cargo cache or locks.
    shutil.copytree(registry, Path(env["CARGO_HOME"]) / "registry")
    tools = {name:subprocess.check_output([name, "-Vv" if name == "rustc" else "--version"], text=True).strip()
             for name in ("rustc", "cargo")}
    machine = dict(platform=platform.platform(), machine=platform.machine(), toolchain=tools,
                   docker_desktop=True, emulated_x86_64=True, vps_capacity_certified=False,
                   filesystem="Docker named volume, not host shared source mount",
                   cpu_limit=2, memory_limit_bytes=4*1024**3, network="none")
    write(output / "environment.json", machine)
    write(output / "source_manifest.json", manifest)

    def run(command, label, child_env=env, timeout=1200):
        print("START " + label, flush=True)
        with (output / (label + ".log")).open("x") as log:
            measured = measured_run(command, workspace, child_env, log, timeout=timeout)
        write(output / (label + ".measurement.json"), measured)
        require(measured["exit_code"] == 0, label + " failed; preserve log")
        print("PASS " + label + " " + json.dumps(measured), flush=True)
        return measured

    try:
        build = ["cargo", "test", "--offline", "--locked", "--release", "-j", "2",
                 "-p", "strategy-runtime", "--lib", "--no-run", "--message-format=json"]
        run(build, "build_test", timeout=1800)
        binaries = []
        for line in (output / "build_test.log").read_text().splitlines():
            try: item = json.loads(line)
            except ValueError: continue
            if item.get("reason") == "compiler-artifact" and item.get("executable") and item["target"]["name"] == "strategy_runtime":
                binaries.append(item["executable"])
        require(len(set(binaries)) == 1, "ambiguous release test binary")
        binary = binaries[0]
        run(["cargo", "build", "--offline", "--locked", "--release", "-j", "2",
             "-p", "strategy-runtime", "--bin", "reaction_shadow_worker"], "build_worker", timeout=1800)
        worker = Path(env["CARGO_TARGET_DIR"]) / "release/reaction_shadow_worker"
        write(output / "binaries.json", dict(test_sha256=digest(binary), worker_sha256=digest(worker),
              worker_bytes=worker.stat().st_size, test_bytes=Path(binary).stat().st_size,
              target="x86_64-unknown-linux-gnu", profile="release", debug_assertions=False))
        run([binary, "reaction_service", "--nocapture", "--test-threads=1"], "service_unit")
        for name in PROFILES:
            directory = output / name
            directory.mkdir()
            local_env = dict(env, REACTION_CONTINUOUS_INPUT=str(bundle / "fixtures" / name / "input.json"),
                             REACTION_CONTINUOUS_OUTPUT=str(directory / "candidate"), REACTION_RETAIN_GENERATION="1")
            run([binary, "reaction_continuous_export_actual", "--ignored", "--nocapture", "--test-threads=1"],
                name + ".export", local_env)
            run([binary, "reaction_continuous_process_recovery_actual", "--ignored", "--nocapture", "--test-threads=1"],
                name + ".restore", local_env)
        require(validate_snapshot(bundle) == manifest, "probe source changed")
        write(output / "execution_complete.json", dict(schema=SCHEMA, result="LINUX_RELEASE_EXECUTED_PENDING_HOST_PARITY",
              deployment=False, vps_capacity_certified=False, cold_filesystem_cache=False))
    except BaseException as error:
        write(output / "failure.json", dict(schema=SCHEMA, result="FAIL_LINUX_RELEASE_PROBE", error=str(error),
              deployment=False, vps_capacity_certified=False))
        raise


def verify(bundle, output):
    # Host-side independent comparator needs pandas; Linux execution is stdlib-only.
    from continuous_reaction import verify_trace, EVIDENCE
    from reaction_compare import compare
    manifest = validate_snapshot(bundle)
    compare(manifest, read(output / "source_manifest.json"))
    require(not (output / "failure.json").exists(), "failed execution cannot be accepted")
    require(read(output / "execution_complete.json")["result"] == "LINUX_RELEASE_EXECUTED_PENDING_HOST_PARITY", "incomplete execution")
    bins = read(output / "binaries.json")
    results = []
    for name in PROFILES:
        fixture_path = bundle / "fixtures" / name / "input.json"
        directory = output / name / "candidate"
        report, restored = read(directory / "report.json"), read(directory / "process_recovery.json")
        exported = read(output / (name + ".export.measurement.json"))
        recovery = read(output / (name + ".restore.measurement.json"))
        validate_release_metadata(bins, report, restored, exported, recovery)
        compare("EXPORTED_CONTINUOUS_CANDIDATE", report["result"])
        compare("PASS_NEW_PROCESS_RESTORE", restored["result"])
        for entry in (report, restored):
            compare(EVIDENCE, entry["evidence_kind"])
            compare(digest(fixture_path), entry["input_sha256"])
            compare(bins["test_sha256"], entry["binary_sha256"])
            compare(False, entry["debug_assertions"])
            compare(False, entry["deployment"])
        compare(report["recovery_witness"], restored["witness"])
        for k in ("directory_bytes", "directory_files"):
            compare(report[k], restored[k])
        with (bundle / "fixtures" / name / "expected.jsonl").open() as f:
            stats, _ = verify_trace(read(fixture_path), [json.loads(line) for line in f], directory / "trace.jsonl")
        compare(manifest["expected_stats"][name], stats)
        results.append(dict(model=name, stats=stats, report=report, restore=restored,
                            export_measurement=exported, restore_measurement=recovery,
                            artifact_sha256={f:digest(directory / f) for f in
                                ("report.json", "process_recovery.json", "trace.jsonl")}))
    verdict = dict(schema=SCHEMA, result="PASS_LOCAL_LINUX_RELEASE_FIXTURE_PARITY_AND_RESTORE",
                   source_manifest_sha256=digest(bundle / "manifest.json"), binaries=bins, results=results,
                   verifier_sha256=digest(__file__),
                   emulated_x86_64=True, vps_capacity_certified=False, prospective_soak=False,
                   cold_filesystem_cache=False, deployment=False, automatic_promotion=False)
    write(output / "verification.json", verdict)
    print(json.dumps(dict(result=verdict["result"], profiles=len(results), deployment=False)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subs = parser.add_subparsers(dest="action", required=True)
    p = subs.add_parser("prepare")
    p.add_argument("--workspace", type=Path, required=True)
    p.add_argument("--evidence", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    for action in ("run", "verify"):
        p = subs.add_parser(action)
        p.add_argument("--bundle", type=Path, required=True)
        p.add_argument("--output", type=Path, required=True)
        if action == "run": p.add_argument("--registry", type=Path, required=True)
    a = parser.parse_args()
    if a.action == "prepare": prepare(a.workspace, a.evidence, a.output)
    elif a.action == "run": inner(a.bundle, a.output, a.registry)
    else: verify(a.bundle, a.output)


if __name__ == "__main__": main()
