"""Independent continuous context/decision oracle, using immutable Python models."""
from __future__ import annotations

import copy
import hashlib
import json
from datetime import date

import pandas as pd

from compare import ParityError, loads
from reaction_compare import compare, keys, projection
from reaction_fixtures import identity
from reaction_history import epoch, event as bar_event, model_label
from reaction_reference import run_reference, frozen_projection, event, trade
from weekend_reference import Context, POLICIES, cutoff

SCHEMA = "reaction_continuous_input_v1"
EVIDENCE = "historical_continuous_fixture_clock_not_production_receipts"
CLOCK = "receipt=durable=processed=label+600; freeze=07:00; one receipt per transaction pair"


def fixture(config, prices, start, end):
    days = [d.date().isoformat() for d in pd.bdate_range(start, end)]
    if not 1 <= len(days) <= 30 or days[0] != start or days[-1] != end:
        raise ParityError("require 1..30 complete regular sessions, weekday endpoints")
    if config["engine"] == "usdrubf" and (config["settings"]["context_hour"], config["settings"]["bo_wait"]) != (7, 4):
        raise ParityError("only approved prospective USD07 W4 pairs")
    cfg = identity(config, POLICIES[0])
    for day in days:
        if day not in prices.weekdays:
            raise ParityError("missing complete execution session: "+day)
        for policy in POLICIES:
            prices.prior(day, policy)
    first = prices.prior(start, POLICIES[0])[1]
    frame = prices.frame.loc[(prices.frame.index >= first) & (prices.frame.index < pd.Timestamp(end)+pd.Timedelta(days=1))]
    name = f"continuous-v1-{model_label(config)}-{start}-{end}"
    spec = dict(ingress=dict(generation=name, symbol=cfg["symbol"], source_stream="fixture:canonical:M10",
                receipt_stream=name), config=copy.deepcopy(config), first_decision_session=start, excluded_dates=[])
    ops = [dict(kind="receipt", bar=bar_event(cfg["symbol"], ts, row, "live"), now=epoch(ts)+600)
           for ts, row in frame.iterrows()]
    ops += [dict(kind="freeze", day=d, now=cutoff(cfg, d)) for d in days]
    ops += [dict(kind="session_end", day=d, now=epoch(d+" 23:40")+600) for d in days]
    # Mid-session, pending-EOD boundary, then whole-generation recovery.
    ops += [dict(kind="restart", now=epoch(start+" "+time)) for time in ("11:10", "23:40")]
    ops += [dict(kind="restart", now=max(op["now"] for op in ops)+1)]
    priority = dict(freeze=0, receipt=1, session_end=2, restart=3)
    ops.sort(key=lambda o: (o["now"], priority[o["kind"]]))
    if len(ops) > 6500:
        raise ParityError("continuous input limit exceeded")
    return dict(schema_version=SCHEMA, evidence_kind=EVIDENCE, spec=spec,
                created_at_utc=ops[0]["now"]-1, operations=ops)


def identities(f):
    result = [identity(f["spec"]["config"], p) for p in POLICIES]
    for i in result:
        i["receipt_stream"] = f["spec"]["ingress"]["receipt_stream"]
        i["excluded_dates"] = f["spec"]["excluded_dates"]
    return result


def receipt(f, op, sequence):
    return dict(sequence=sequence, source_id=f"{sequence}-0", bar=op["bar"],
                received_at_utc=op["now"], durable_at_utc=op["now"])


def expected(f, prices, modules):
    cfg, start = f["spec"]["config"], f["spec"]["first_decision_session"]
    contexts = [Context(i) for i in identities(f)]
    frozen, rows, models = None, None, None
    sequence, tx_count = 0, 0
    for index, op in enumerate(f["operations"]):
        common = dict(input_index=index, kind=op["kind"])
        if op["kind"] == "freeze":
            for ctx in contexts:
                ctx.freeze(op["day"], op["now"])
            frozen = [copy.deepcopy(c.frozen[-1]) for c in contexts]
            rows = []
            for ctx, anchor in zip(contexts, frozen):
                if not anchor["ready"]:
                    raise ParityError("selected generation has unready anchors")
                prices.assert_full_raw_context(anchor, op["day"], ctx.cfg["policy"])
                case = dict(config=cfg, identity=ctx.cfg, day=op["day"], operations=[
                    dict(kind="bar", value=bar_event(ctx.cfg["symbol"], ts, row, "replay"))
                    for ts, row in prices.days[op["day"]].iterrows()])
                rows.append(run_reference(case, anchor, modules))
            models = None
            tx_count += 1
            yield dict(**common, frozen=frozen, sequence=sequence, tx_count=tx_count)
        elif op["kind"] == "receipt":
            sequence += 1
            r = receipt(f, op, sequence)
            routes = [ctx.ingest(r, op["now"]) for ctx in contexts]
            dt = pd.Timestamp(op["bar"]["close_time_utc"], unit="s")+pd.Timedelta(hours=3)
            day = dt.date().isoformat()
            state, deltas = None, None
            if day < start:
                disposition = "before_activation"
            elif dt.weekday() >= 5 or not 7*60 <= dt.hour*60+dt.minute <= 23*60+40:
                disposition = "non_trading_bar"
            else:
                if frozen is None or frozen[0]["session_date"] != day or routes != [dict(kind="context_only")]*2:
                    raise ParityError("invalid prospective context route/day")
                slot = (dt.hour*60+dt.minute-7*60)//10
                state = [rows[a][slot] for a in range(2)]
                deltas = []
                for a in range(2):
                    delta = {}
                    for k in ("events", "trades"):
                        old = models[a][k] if models else []
                        compare(old, state[a][k][:len(old)], "reference_append_only")
                        delta[k] = [[j, v] for j, v in enumerate(state[a][k]) if j >= len(old)]
                    deltas.append(delta)
                models = state
                disposition = "model_observed"
            tx_count += 2
            yield dict(**common, receipt=r, routes=routes, disposition=disposition,
                       models=state, deltas=deltas, frozen=frozen, sequence=sequence, tx_count=tx_count)
        elif op["kind"] == "session_end":
            if models is None or any(m["position"] or m["pending"] for m in models):
                raise ParityError("reference incomplete EOD")
            yield dict(**common, day=op["day"], models=models, frozen=frozen, sequence=sequence, tx_count=tx_count)
        elif op["kind"] == "restart":
            yield dict(**common, sequence=sequence, tx_count=tx_count)
        else:
            raise ParityError("unknown input operation")


def digest_id(generation, identity_value, day, kind, index):
    raw = json.dumps([generation, identity_value, day, kind, index], separators=(",", ":"), allow_nan=False)
    return hashlib.sha256(raw.encode()).hexdigest()


def hex_hash(value):
    if not isinstance(value, str) or len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
        raise ParityError("invalid digest")


def check_delta(actual, want, cfg_identity, generation, day, seen):
    keys(actual, "events trades")
    for kind, project in (("events", event), ("trades", trade)):
        if len(actual[kind]) != len(want[kind]):
            raise ParityError("delta coverage mismatch")
        for record, (index, ref) in zip(actual[kind], want[kind]):
            keys(record, "id payload")
            wanted_id = digest_id(generation, cfg_identity, day, kind, index)
            compare(wanted_id, record["id"], "stable_record_id")
            if record["id"] in seen:
                raise ParityError("duplicate economic/event record")
            seen.add(record["id"])
            compare(ref, project(record["payload"]), "delta."+kind)


def check_states(actual, want, frozen, cfg, ids):
    if not isinstance(actual, list) or len(actual) != 2:
        raise ParityError("missing paired states")
    for arm, state in enumerate(actual):
        keys(state, "version evidence identity frozen model bars_processed order_emission_allowed")
        compare("frozen_reaction_prefix_v1", state["version"])
        compare("numerical_prefix_not_operational_receipts", state["evidence"])
        compare(False, state["order_emission_allowed"])
        compare(ids[arm], state["identity"], "identity")
        compare(frozen[arm], frozen_projection(state["frozen"]), "frozen")
        compare(want[arm]["observation"]["count"], state["bars_processed"])
        compare(want[arm], projection(state["model"], cfg), "model")


def check_fixture_status_shape(status):
    fields = "committed_transactions durable_receipts ingested_sequence observed_sequence transaction_bytes source_caught_up failed session_date pair_ready session_complete order_emission_allowed"
    # Keep old evidence readable, but never pass a bootstrapped trace off as this
    # original-receipt-clock fixture. Bootstrap has separate admission tests.
    if "bootstrap_status" in status or "bootstrap_rows" in status:
        fields += " bootstrap_status bootstrap_rows"
        keys(status, fields)
        compare("disabled", status["bootstrap_status"])
        compare(0, status["bootstrap_rows"])
    else:
        keys(status, fields)


def verify_trace(f, expected_rows, path):
    if f["spec"].get("bootstrap") is not None:
        raise ParityError("historical bootstrap is not this original-receipt fixture")
    ids, cfg, generation = identities(f), f["spec"]["config"], f["spec"]["ingress"]["generation"]
    stats = dict(state_rows=0, rounds=0, events=0, completed_sessions=0, restarts=0, transactions=0)
    seen, outbox_prefix, economics = set(), None, []
    def take(stream, kind, index):
        raw = stream.readline()
        if not raw:
            raise ParityError("truncated candidate output")
        row = loads(raw)
        compare(kind, row.get("kind")); compare(index, row.get("input_index"))
        return row
    def tx(stream, index, sequence, observed):
        nonlocal outbox_prefix
        row = take(stream, "transaction", index)
        keys(row, "kind input_index output")
        out = row["output"]
        keys(out, "generation transaction outbox_id ingested_sequence observed_sequence frozen steps order_emission_allowed")
        stats["transactions"] += 1
        compare(generation, out["generation"]); compare(stats["transactions"], out["transaction"])
        compare(sequence, out["ingested_sequence"]); compare(observed, out["observed_sequence"])
        compare(False, out["order_emission_allowed"])
        prefix, ordinal = out["outbox_id"].split(":")
        hex_hash(prefix)
        if outbox_prefix is None: outbox_prefix = prefix
        compare(outbox_prefix, prefix); compare(str(stats["transactions"]), ordinal)
        return out
    with path.open() as stream:
        for e in expected_rows:
            k, i, seq = e["kind"], e["input_index"], e["sequence"]
            if k == "receipt":
                first = tx(stream, i, seq, seq-1)
                compare(None, first["frozen"]); compare([], first["steps"])
                second = tx(stream, i, seq, seq)
                compare(None, second["frozen"])
                if len(second["steps"]) != 1: raise ParityError("missing/extra receipt step")
                step = second["steps"][0]
                keys(step, "receipt disposition states deltas")
                compare(e["receipt"], step["receipt"], "original_receipt")
                compare(e["disposition"], step["disposition"])
                if e["models"] is None:
                    compare(None, step["states"]); compare(None, step["deltas"])
                else:
                    check_states(step["states"], e["models"], e["frozen"], cfg, ids)
                    if len(step["deltas"]) != 2: raise ParityError("missing paired deltas")
                    stats["state_rows"] += 2
                    for a in range(2):
                        check_delta(step["deltas"][a], e["deltas"][a], ids[a], generation, e["frozen"][a]["session_date"], seen)
                        stats["rounds"] += len(e["deltas"][a]["trades"])
                        stats["events"] += len(e["deltas"][a]["events"])
                        economics += [dict(model=model_label(cfg),policy=ids[a]["policy"],day=e["frozen"][a]["session_date"],**t) for _, t in e["deltas"][a]["trades"]]
            elif k == "freeze":
                out = tx(stream, i, seq, seq); compare([], out["steps"])
                compare(e["frozen"], [frozen_projection(v) for v in out["frozen"]], "paired_freeze")
            elif k == "session_end":
                row = take(stream, k, i)
                keys(row, "kind input_index day status states directory_bytes directory_files elapsed_sec rss_sample_kib")
                compare(e["day"], row["day"])
                check_states(row["states"], e["models"], e["frozen"], cfg, ids)
                status = row["status"]
                check_fixture_status_shape(status)
                for key in ("durable_receipts", "ingested_sequence", "observed_sequence"): compare(seq, status[key])
                compare(stats["transactions"], status["committed_transactions"])
                compare(e["day"], status["session_date"])
                for key in ("pair_ready", "session_complete"): compare(True, status[key])
                for key in ("failed", "order_emission_allowed", "source_caught_up"): compare(False, status[key])
                stats["completed_sessions"] += 1
            elif k == "restart":
                row = take(stream, k, i)
                keys(row, "kind input_index model_sha256 head_sha256 committed_transactions directory_unchanged duration_sec rss_sample_kib")
                hex_hash(row["model_sha256"]); hex_hash(row["head_sha256"])
                compare(stats["transactions"], row["committed_transactions"])
                compare(True, row["directory_unchanged"])
                stats["restarts"] += 1
            compare(e["tx_count"], stats["transactions"])
        if stream.read().strip(): raise ParityError("unexpected candidate tail")
    return stats, economics
