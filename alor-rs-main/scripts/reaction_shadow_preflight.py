#!/usr/bin/env python3
"""Read-only paired preparation/admission checks. Never starts or writes a generation."""
import argparse
import datetime as dt
import hashlib
import ipaddress
import json
import math
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
import tomllib
from urllib.parse import unquote, urlsplit

import reaction_shadow_service as service

MSK = dt.timezone(dt.timedelta(hours=3))
GATES = {"complete_session", "terminal", "overnight_recovery", "revisions", "source_prefix", "bootstrap"}
AS_OBSERVED = "as_observed_audit_v1"
FORBIDDEN_ENV = {"ALOR_REFRESH_TOKEN", "SHADOW_ALOR_REFRESH_TOKEN", "ALOR_ACCESS_TOKEN",
                 "SHADOW_SOURCE_REDIS_URL", "REDIS_URL"}


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def regular(path):
    p = Path(path)
    require(p.is_absolute() and p.resolve(strict=True) == p and p.is_file(), "canonical_regular_file_required")
    return p


def small(path):
    return service.read_small(regular(path))


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def sha(value):
    return type(value) is str and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def fields(value, expected):
    require(type(value) is dict and set(value) == set(expected), "unexpected_preflight_schema")


class Reader:
    """Small bounded RESP reader: AUTH, PING and exact-stream XRANGE only."""
    def __init__(self, url, binding, stream):
        u = urlsplit(url)
        require(u.scheme == "redis" and u.hostname == binding["host"]
                and u.port == binding["port"] and u.path == "/0" and not u.query and not u.fragment
                and unquote(u.username or "") == binding["username"] and bool(u.password), "reader_binding_mismatch")
        self.stream = stream
        self.sock = socket.create_connection((u.hostname, u.port), timeout=3)
        self.file = self.sock.makefile("rb")
        try:
            require(self.command("AUTH", unquote(u.username), unquote(u.password)) == b"OK", "reader_auth_failed")
        except BaseException:
            self.close()
            raise

    def close(self):
        self.file.close()
        self.sock.close()

    def reply(self, depth=0):
        require(depth <= 8, "redis_reply_depth")
        line = self.file.readline(65537)
        self.remaining -= len(line)
        require(self.remaining >= 0, "redis_reply_budget")
        require(line.endswith(b"\r\n") and len(line) <= 65536, "redis_reply_line")
        kind, value = line[:1], line[1:-2]
        if kind == b"+": return value
        if kind == b"-": raise ValueError("reader_command_denied_or_failed")
        if kind == b":": return int(value)
        if kind == b"$":
            n = int(value)
            if n == -1: return None
            require(0 <= n <= 65536, "redis_reply_size")
            self.remaining -= n + 2
            require(self.remaining >= 0, "redis_reply_budget")
            raw = self.file.read(n + 2)
            require(len(raw) == n + 2 and raw.endswith(b"\r\n"), "redis_reply_truncated")
            return raw[:-2]
        require(kind == b"*", "redis_reply_type")
        n = int(value)
        require(0 <= n <= 16, "redis_reply_array")
        return [self.reply(depth + 1) for _ in range(n)]

    def command(self, *args):
        require((args[:1] == ("AUTH",) and len(args) == 3)
                or args == ("PING",)
                or (len(args) == 6 and args[:2] == ("XRANGE", self.stream)
                    and args[4:] == ("COUNT", "1")), "non_readonly_probe_command")
        parts = [s.encode() for s in args]
        self.remaining = 128 * 1024
        self.sock.sendall(b"*" + str(len(parts)).encode() + b"\r\n" + b"".join(
            b"$" + str(len(p)).encode() + b"\r\n" + p + b"\r\n" for p in parts))
        return self.reply()

    def envelope(self, first="-", last="+"):
        rows = self.command("XRANGE", self.stream, first, last, "COUNT", "1")
        require(len(rows) == 1 and len(rows[0]) == 2 and len(rows[0][1]) == 2
                and rows[0][1][0] == b"payload", "reader_source_row_missing_or_invalid")
        return rows[0][0].decode(), json.loads(rows[0][1][1])


def previous_session(first, excluded):
    day = first - dt.timedelta(days=1)
    for _ in range(14):
        if day.weekday() < 5 and day.isoformat() not in excluded:
            return day
        day -= dt.timedelta(days=1)
    raise ValueError("previous_session_not_found")


def source_records(source):
    with regular(source / "committed.jsonl").open("rb") as handle:
        raw = handle.read(16 * 1024**2 + 1)
    require(len(raw) <= 16 * 1024**2 and raw.endswith(b"\n"), "source_prefix_incomplete_or_oversized")
    records = [json.loads(line)["record"] for line in raw.splitlines()]
    require(1 <= len(records) <= 4500, "source_prefix_row_bound")
    labels = [r["envelope"]["payload"]["close_time_utc"] for r in records]
    require(all(type(x) is int for x in labels) and labels == sorted(set(labels)), "source_prefix_label_order")
    require(all(r["id"] == str(label * 1000) + "-0" for r, label in zip(records, labels)), "source_prefix_id_binding")
    return records


def revision_snapshot(source, symbol, now):
    """Inspect immutable originals and audited restatements; never apply revisions."""
    records = source_records(source)
    canonical = {r["envelope"]["payload"]["close_time_utc"]: r["envelope"]["payload"] for r in records}
    path = source / "revisions.jsonl"
    if path.exists() or path.is_symlink():
        with regular(path).open("rb") as handle:
            raw = handle.read(1024**2 + 1)
        require(len(raw) <= 1024**2 and (not raw or raw.endswith(b"\n")), "revision_journal_incomplete_or_oversized")
    else:
        raw = b""
    items = [json.loads(line) for line in raw.splitlines()]
    require(len(items) <= 1024, "revision_row_bound")
    dates, ids, price_rows = set(), set(), 0
    bar_fields = {"symbol", "close_time_utc", "o", "h", "l", "c", "v", "origin"}
    for item in items:
        fields(item, ("event", "sha256"))
        require(sha(item["sha256"]) and item["sha256"] not in ids, "revision_event_identity")
        ids.add(item["sha256"])
        event = item["event"]
        fields(event, ("identity", "label", "kind", "received_at_utc", "generation", "review_required"))
        fields(event["identity"], ("original", "observed"))
        old, new = event["identity"]["original"], event["identity"]["observed"]
        fields(old, bar_fields); fields(new, bar_fields)
        label = event["label"]
        require(type(label) is int and label in canonical and label % 600 == 0, "revision_canonical_label")
        require(old == {k: canonical[label][k] for k in bar_fields}
                and new["symbol"] == old["symbol"] == symbol and new["close_time_utc"] == label,
                "revision_original_binding")
        require(all(type(new[k]) in (int, float) and math.isfinite(new[k]) for k in ("o", "h", "l", "c", "v"))
                and 0 < new["l"] <= min(new["o"], new["c"])
                and new["h"] >= max(new["o"], new["c"]) and new["v"] >= 0
                and new["origin"] in ("live", "history"), "invalid_revision_ohlcv")
        price = any(old[k] != new[k] for k in ("o", "h", "l", "c"))
        require((price or old["v"] != new["v"]) and event["review_required"] is price
                and event["kind"] == ("price_revision" if price else "volume_only_revision")
                and type(event["generation"]) is int and event["generation"] > 0
                and type(event["received_at_utc"]) is int and label + 600 <= event["received_at_utc"] <= now,
                "invalid_revision_metadata")
        if price:
            price_rows += 1
            dates.add(dt.datetime.fromtimestamp(label, MSK).date().isoformat())
    health = json.loads(small(source / "HEALTH.json"))
    require(type(health.get("audit_rows")) is int and health["audit_rows"] == len(items)
            and health.get("review_dates") == sorted(dates),
            "revision_health_journal_mismatch")
    require(health.get("queue_drained") is True and not (source / "pending.json").exists()
            and not (source / "pending.json").is_symlink(), "source_publication_pending")
    # File SHA binds exact Rust-owned bytes; this is not a Python reimplementation
    # of the Rust event hash or canonical proof verifier.
    return dict(policy=AS_OBSERVED, journal_sha256=digest(raw), rows=len(items),
                price_rows=price_rows, volume_only_rows=len(items) - price_rows, review_dates=sorted(dates))


def terminal_record(source, cfg, day, now, symbol="USDRUBF"):
    end = cfg["terminal"]["symbols"][symbol]
    end = end["overrides"].get(day.isoformat(), end["weekday_end"])
    require(end == "23:50:00", "review_nonstandard_terminal_separately")
    close = int(dt.datetime.combine(day, dt.time(23, 50), MSK).timestamp())
    label = close - 600
    records = source_records(source)
    rows = [r for r in records if dt.datetime.fromtimestamp(
        r["envelope"]["payload"]["close_time_utc"], MSK).date() == day]
    expected = list(range(int(dt.datetime.combine(day, dt.time(7), MSK).timestamp()), label + 1, 600))
    require([r["envelope"]["payload"]["close_time_utc"] for r in rows] == expected,
            "previous_session_not_complete")
    record = rows[-1]
    release = record["release"]
    require(type(release) is dict, "historical_terminal_is_not_forward_proof")
    p, bar, envelope = release["proof"], release["bar"], record["envelope"]
    payload = envelope["payload"]
    budget = cfg["terminal"]["max_lateness_seconds"]
    require(p["generation"] > 0 and p["closed_at_utc"] == close
            and label <= p["first_received_at_utc"] <= p["last_received_at_utc"] <= p["observation_cutoff_utc"]
            and close <= p["observation_cutoff_utc"] <= envelope["ts_utc"] <= min(now, close + budget)
            and bar["origin"] == "live" and bar["close_time_utc"] == label
            and payload["source_proof"]["release_id"] == release["id"]
            and payload["source_proof"]["release"] == p
            and all(bar[k] == payload[k] for k in ("symbol", "close_time_utc", "o", "h", "l", "c", "v")),
            "invalid_terminal_structural_proof")
    if p["reason"] == "session_boundary_ws":
        require(p["next_label_utc"] is None and p["observation_cutoff_utc"] >= close + cfg["terminal"]["grace_seconds"],
                "invalid_terminal_boundary_proof")
    else:
        require(p["reason"] == "next_timestamp" and type(p["next_label_utc"]) is int
                and close <= p["next_label_utc"] <= p["observation_cutoff_utc"], "invalid_terminal_release_reason")
    return record


def late_context_snapshot(source, cfg, now):
    """Bind quarantined context to canonical witnesses, never import or replay it."""
    records = source_records(source)
    by_label = {r['envelope']['payload']['close_time_utc']: r for r in records}
    heads = {item['record']['id']: item.get('sha256') for item in (
        json.loads(line) for line in (source/'committed.jsonl').read_bytes().splitlines())}
    path = source/'late-context.jsonl'
    raw = b''
    if path.exists() or path.is_symlink():
        with regular(path).open('rb') as handle:
            raw = handle.read(1024**2 + 1)
    require(len(raw) <= 1024**2 and (not raw or raw.endswith(b'\n')), 'late_context_journal_incomplete')
    items = [json.loads(line) for line in raw.splitlines()]
    require(len(items) <= 1024, 'late_context_row_bound')
    dates, identities, hashes = set(), set(), set()
    for item in items:
        fields(item, ('event','sha256'))
        require(sha(item['sha256']) and item['sha256'] not in hashes, 'late_context_event_identity')
        hashes.add(item['sha256'])
        e = item['event']
        fields(e, ('kind','disposition','observed','received_at_utc','durable_at_utc','generation',
                   'canonical_tail_label','canonical_tail_id','canonical_head_sha256'))
        b = e['observed']
        fields(b, ('symbol','close_time_utc','o','h','l','c','v','origin'))
        label = b['close_time_utc']
        require(type(label) is int and label > 0 and label % 600 == 0, 'late_context_label')
        day = dt.datetime.fromtimestamp(label, MSK)
        context = day.weekday() >= 5 or day.date().isoformat() in cfg['excluded_dates']
        calendar = cfg['terminal']['symbols'][cfg['symbol']]
        end = calendar['overrides'].get(day.date().isoformat(), calendar['weekend_end'] if day.weekday() >= 5 else calendar['weekday_end'])
        require(context and end is not None and dt.time(9,50) <= day.time() < dt.time.fromisoformat(end)
                and day.date() >= dt.date.fromisoformat(cfg['bootstrap_start'])
                and b['symbol'] == cfg['symbol'] and b['origin'] == 'history' and label not in by_label,
                'late_context_calendar_or_canonical_conflict')
        require(all(type(b[k]) in (int,float) and math.isfinite(b[k]) for k in ('o','h','l','c','v'))
                and 0 < b['l'] <= min(b['o'],b['c']) and b['h'] >= max(b['o'],b['c']) and b['v'] >= 0,
                'invalid_late_context_ohlcv')
        tail = by_label.get(e['canonical_tail_label'])
        require(type(e['canonical_tail_label']) is int and tail is not None and label < e['canonical_tail_label']
                and tail['id'] == e['canonical_tail_id'] and sha(e['canonical_head_sha256'])
                and heads[tail['id']] == e['canonical_head_sha256'], 'late_context_tail_binding')
        require(e['kind'] == 'late_unknown_context_bar' and e['disposition'] == 'audit_only_no_model_replay'
                and type(e['generation']) is int and e['generation'] > 0
                and type(e['received_at_utc']) is int and type(e['durable_at_utc']) is int
                and label + 600 <= e['received_at_utc'] <= e['durable_at_utc'] <= now,
                'invalid_late_context_receipt')
        identity = json.dumps(b,sort_keys=True,separators=(',',':'),allow_nan=False)
        require(identity not in identities, 'duplicate_late_context_observation')
        identities.add(identity); dates.add(day.date().isoformat())
    h = json.loads(small(source/'HEALTH.json'))
    require(h.get('late_context_rows') == len(items) and type(h.get('late_context_rows')) is int
            and h.get('late_context_review_dates') == sorted(dates), 'late_context_health_journal_mismatch')
    return dict(policy='audit_only_v1',journal_sha256=digest(raw),rows=len(items),review_dates=sorted(dates))


def preflight(plan_path, plan_sha, stage, environ=None, now=None):
    require(stage in ("preparation", "admission") and sha(plan_sha), "explicit_preflight_stage_and_digest_required")
    require(os.geteuid() != 0, "dedicated_nonroot_reader_required")
    env = dict(os.environ if environ is None else environ)
    require(not FORBIDDEN_ENV.intersection(env), "broker_or_generic_redis_environment_forbidden")
    raw = small(plan_path)
    require(digest(raw) == plan_sha, "preflight_plan_changed")
    plan = json.loads(raw)
    fields(plan, ("version", "files", "paths", "redis"))
    require(plan["version"] in ("reaction_shadow_preflight_v1", "reaction_shadow_preflight_v2", "reaction_shadow_preflight_v3"), "unsupported_preflight_plan")
    v3 = plan['version'] == 'reaction_shadow_preflight_v3'
    v2 = plan["version"] in ('reaction_shadow_preflight_v2','reaction_shadow_preflight_v3')
    fields(plan["files"], ("worker", "supervisor", "prepared_spec", "source_manifest"))
    files = {}
    for name, item in plan["files"].items():
        fields(item, ("path", "sha256"))
        files[name] = regular(item["path"])
        require(sha(item["sha256"]) and service.file_hash(files[name]) == item["sha256"], "pinned_file_changed")
        require(not os.access(files[name], os.W_OK), "pinned_file_writable_by_worker")
    require(files["supervisor"] == Path(service.__file__).resolve(), "supervisor_import_mismatch")
    require(os.access(files["worker"], os.X_OK), "worker_not_executable")
    fields(plan["paths"], ("source", "owner", "generation", "approved_spec", "approval"))
    paths = {k: Path(v) for k, v in plan["paths"].items()}
    for name in ("source", "owner"):
        p = paths[name]
        require(p.is_absolute() and p.resolve(strict=True) == p and p.is_dir(), "canonical_directory_required")
    require(files["source_manifest"] == paths["source"] / "manifest.json", "source_manifest_path_mismatch")
    require(paths["source"].stat().st_uid != os.geteuid(), "source_and_worker_uid_must_differ")
    require(os.access(paths["owner"], os.W_OK) and not os.access(paths["source"], os.W_OK), "worker_source_directory_permissions")
    generation = paths["generation"]
    require(generation.is_absolute() and generation.parent == paths["owner"] and generation.name not in ("", ".", ".."), "generation_parent_mismatch")
    for p in (generation, generation.with_name(generation.name + ".supervisor")):
        require(not p.exists() and not p.is_symlink(), "generation_or_attempt_already_exists")
    config_dir = regular(plan_path).parent
    for name in ("approved_spec", "approval"):
        require(paths[name].parent == config_dir and not paths[name].is_symlink(), "admission_path_mismatch")
    prepared_raw = small(files["prepared_spec"])
    prepared = tomllib.loads(prepared_raw.decode())
    policy = service.policy_from_spec(prepared_raw)
    require("bootstrap" not in prepared["runner"], "preparation_must_not_pin_bootstrap")
    runner = prepared["runner"]
    symbol = runner["ingress"]["symbol"]
    require((symbol, runner["config"]["engine"]) in (("USDRUBF", "usdrubf"), ("IMOEXF", "imoexf"))
            and (v2 or symbol == "USDRUBF"), "symbol_profile_required")
    guard = service.ClosedSourceGuard(paths["source"], prepared_raw)
    manifest = json.loads(small(files["source_manifest"]))
    require(manifest["version"] == ("shadow_gateway_ws_closed_m10_v4" if v3 else "shadow_gateway_ws_closed_m10_v3"), "source_version_required")
    cfg = manifest["config"]
    fields(plan["redis"], ("host", "port", "username"))
    binding = plan["redis"]
    address = ipaddress.ip_address(binding["host"])
    require(address.version == 4 and address.is_private and type(binding["port"]) is int
            and 1 <= binding["port"] <= 65535 and binding["username"].startswith("shadow_ws_reader_"), "invalid_reader_binding")
    current = int(time.time()) if now is None else now
    first = dt.date.fromisoformat(runner["first_decision_session"])
    cutoff = int(dt.datetime.combine(first, dt.time(7), MSK).timestamp())
    require(current < cutoff, "model_date_expired_no_inplace_retarget")
    approval = None
    approved_raw = None
    revisions = revision_snapshot(paths["source"], symbol, current) if v2 else None
    late_context = late_context_snapshot(paths['source'], cfg, current) if v3 else None
    if stage == "admission":
        require(paths["approval"].is_file() and paths["approved_spec"].is_file(), "explicit_admission_missing")
        require(not os.access(paths["approval"], os.W_OK) and not os.access(paths["approved_spec"], os.W_OK), "admission_files_writable_by_worker")
        approval_raw = small(paths["approval"])
        approval = json.loads(approval_raw)
        approval_fields = {"version", "plan_sha256", "approved_spec_sha256", "reviewed_at_utc", "ready_by", "gates", "evidence"}
        if v2:
            approval_fields.add("revision_review")
        if v3:
            approval_fields.add('late_context_review')
        fields(approval, approval_fields)
        require(approval["version"] == ("reaction_shadow_admission_v3" if v3 else "reaction_shadow_admission_v2" if v2 else "reaction_shadow_admission_v1")
                and approval["plan_sha256"] == plan_sha, "admission_identity_mismatch")
        require(type(approval["reviewed_at_utc"]) is int and 0 <= current - approval["reviewed_at_utc"] <= 3600
                and dt.datetime.fromtimestamp(current, MSK).date() == first, "fresh_model_morning_review_required")
        fields(approval["gates"], GATES)
        require(all(value is True for value in approval["gates"].values()), "operator_gates_not_approved")
        ready = dt.datetime.fromisoformat(approval["ready_by"])
        require(ready.tzinfo is not None and ready.microsecond == 0
                and approval["ready_by"] == env.get("SHADOW_READY_BY")
                and current + policy["startup_timeout_seconds"] <= ready.timestamp() <= cutoff, "readiness_window_invalid")
        evidence = approval["evidence"]
        require(type(evidence) is list and len(evidence) == (3 if v2 else 2), "reviewed_evidence_required")
        kinds = set()
        for item in evidence:
            fields(item, ("kind", "path", "sha256"))
            p = regular(item["path"])
            require(p.parent == config_dir / "evidence" and not os.access(p, os.W_OK)
                    and sha(item["sha256"]) and digest(small(p)) == item["sha256"], "reviewed_evidence_changed")
            kinds.add(item["kind"])
        require(kinds == ({"natural_session", "overnight_recovery", "source_revisions"} if v2
                          else {"natural_session", "overnight_recovery"}), "reviewed_evidence_kinds")
        if v2:
            require(approval["revision_review"] == revisions, "revision_review_missing_or_changed")
        if v3:
            require(approval['late_context_review'] == late_context, 'late_context_review_missing_or_changed')
        approved_raw = small(paths["approved_spec"])
        require(digest(approved_raw) == approval["approved_spec_sha256"], "approved_spec_changed")
        approved = tomllib.loads(approved_raw.decode())
        bootstrap = approved["runner"].pop("bootstrap", None)
        require(approved == prepared and type(bootstrap) is dict, "only_bootstrap_may_be_added")
        fields(bootstrap, ("first_source_id", "last_source_id", "rows", "source_sha256", "first_session", "last_session"))
        previous = previous_session(first, runner["excluded_dates"])
        terminal = terminal_record(paths["source"], cfg, previous, current, symbol)
        if v2:
            tail = source_records(paths["source"])[-1]
            tail_day = dt.datetime.fromtimestamp(tail["envelope"]["payload"]["close_time_utc"], MSK).date()
            require(previous <= tail_day < first and bootstrap["last_session"] == tail_day.isoformat()
                    and bootstrap["last_source_id"] == tail["id"], "bootstrap_must_include_context_tail")
            previous_open = int(dt.datetime.combine(previous, dt.time(7), MSK).timestamp())
            require(dt.date.fromisoformat(bootstrap["first_session"]) <= previous
                    and int(bootstrap["first_source_id"].split('-')[0]) <= previous_open * 1000,
                    "bootstrap_must_include_previous_session")
        else:
            require(bootstrap["last_session"] == previous.isoformat(), "bootstrap_must_include_previous_session")
            health = json.loads(small(paths["source"] / "HEALTH.json"))
            require(health.get("review_dates") == [], "source_price_revision_requires_separate_review")
            require(bootstrap["last_source_id"] == terminal["id"], "bootstrap_must_include_terminal")
    reader = Reader(env.get("REACTION_SHADOW_REDIS_URL", ""), binding, cfg["stream"])
    try:
        require(reader.command("PING") == b"PONG", "reader_ping_failed")
        _, envelope = reader.envelope()
        require(envelope["source"] == manifest["version"] and envelope["payload"]["symbol"] == symbol, "reader_source_identity_mismatch")
        if stage == "admission":
            sid, envelope = reader.envelope(terminal["id"], terminal["id"])
            require(sid == terminal["id"] and envelope == terminal["envelope"], "terminal_redis_prefix_mismatch")
    finally:
        reader.close()
    if stage == "admission":
        result = subprocess.run([str(files["worker"]), "inspect-bootstrap", str(paths["approved_spec"]),
                                 bootstrap["first_source_id"], bootstrap["last_source_id"]],
                                env={"REACTION_SHADOW_REDIS_URL": env["REACTION_SHADOW_REDIS_URL"], "TZ": "UTC"},
                                capture_output=True, timeout=20)
        require(result.returncode == 0 and len(result.stdout) <= 65536, "bootstrap_inspection_failed")
        preview = json.loads(result.stdout)
        require(preview["event"] == "bootstrap_inspected" and preview["order_emission_allowed"] is False
                and preview["preview"]["order_emission_allowed"] is False
                and preview["preview"]["bootstrap"] == bootstrap
                and len(preview["preview"]["anchors"]) == 2
                and all(len(arm) >= 2 for arm in preview["preview"]["anchors"]), "bootstrap_inspection_mismatch")
        finished = int(time.time()) if now is None else current
        require(finished + policy["startup_timeout_seconds"] <= ready.timestamp(), "preflight_missed_startup_window")
    guard.check(require_ready=True)
    if v2:
        require(revision_snapshot(paths["source"], symbol, int(time.time()) if now is None else current) == revisions,
                "revision_journal_changed_during_preflight")
        if v3:
            require(late_context_snapshot(paths['source'],cfg,int(time.time()) if now is None else current) == late_context,
                    'late_context_changed_during_preflight')
        if stage == "admission":
            require(source_records(paths["source"])[-1]["id"] == bootstrap["last_source_id"],
                    "source_tail_changed_during_inspection")
    elif stage == "admission":
        require(json.loads(small(paths["source"] / "HEALTH.json")).get("review_dates") == [],
                "source_price_revision_during_inspection")
    require(not generation.exists() and not generation.with_name(generation.name + ".supervisor").exists(), "generation_created_during_preflight")
    result = dict(event="reaction_shadow_preflight", stage=stage, preparation_ready=True,
                admission_allowed=stage == "admission", worker_started=False, generation_created=False,
                reader_uid=os.geteuid(), source_readonly=True,
                bootstrap_pinned=stage == "admission", order_emission_allowed=False,
                model_first_session=first.isoformat(), plan_sha256=plan_sha,
                ordinary_model_limit=policy.get(service.ORDINARY_LIMIT, policy["max_model_lateness_seconds"]),
                terminal_model_limit=policy["max_model_lateness_seconds"], queue_limit=policy["max_queue_age_seconds"],
                pending_gates=sorted(GATES) if stage == "preparation" else [])
    if v2:
        result.update(symbol=symbol, revision_review=revisions, context_tail_policy="include_observed_pre_cutoff_tail")
    if v3:
        result['late_context_review'] = late_context
        if stage == 'admission':
            checked = int(time.time()) if now is None else current
            require(checked >= current and checked + policy['startup_timeout_seconds'] <= ready.timestamp(),
                    'grant_missed_startup_window')
            result['admission_grant'] = dict(
                version='reaction_shadow_admission_grant_v1', checked_at_utc=checked,
                policy='as_observed_reviewed_snapshot_grant_v1',
                plan_sha256=plan_sha, approved_spec_sha256=approval['approved_spec_sha256'],
                approval_sha256=digest(approval_raw), reviewed_at_utc=approval['reviewed_at_utc'],
                source_manifest_sha256=plan['files']['source_manifest']['sha256'],
                worker_sha256=plan['files']['worker']['sha256'],
                supervisor_sha256=plan['files']['supervisor']['sha256'],
                bootstrap=bootstrap, revision_review=revisions, late_context_review=late_context,
                ready_by=approval['ready_by'], model_first_session=first.isoformat(),
                reader_uid=os.geteuid(), order_emission_allowed=False)
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--plan", type=Path, required=True)
    p.add_argument("--plan-sha256", required=True)
    p.add_argument("--stage", choices=("preparation", "admission"), required=True)
    args = p.parse_args()
    try:
        print(json.dumps(preflight(args.plan, args.plan_sha256, args.stage), sort_keys=True))
        return 0
    except Exception as error:
        print(json.dumps(dict(event="reaction_shadow_preflight_failed", stage=args.stage,
                              error_class=type(error).__name__, error_sha256=digest(str(error).encode()),
                              admission_allowed=False, worker_started=False, order_emission_allowed=False)))
        return 1


if __name__ == "__main__":
    sys.exit(main())
