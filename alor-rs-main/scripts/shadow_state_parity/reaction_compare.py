"""Declared state projection; fee economics/Kahan correction are not parity claims."""
import copy
import math

from compare import ParityError
from reaction_reference import event, trade, position, pending, frozen_projection


def keys(value, fields):
    if not isinstance(value, dict) or set(value) != set(fields.split()):
        raise ParityError(f"unexpected raw candidate fields: expected {fields}")


def compare(left, right, path="root"):
    if isinstance(left, dict):
        if not isinstance(right, dict) or set(left) != set(right):
            raise ParityError(f"{path}: fields differ")
        for k in left:
            compare(left[k], right[k], path+"."+k)
    elif isinstance(left, list):
        if not isinstance(right, list) or len(left) != len(right):
            raise ParityError(f"{path}: coverage differs")
        for i, (a, b) in enumerate(zip(left, right)):
            compare(a, b, f"{path}[{i}]")
    elif isinstance(left, float):
        if (type(right) not in (float, int) or not math.isfinite(left) or not math.isfinite(right)
            or abs(left-right) > 1e-8):
            raise ParityError(f"{path}: numeric mismatch {left!r} != {right!r}")
    elif type(left) is not type(right) or left != right:
        raise ParityError(f"{path}: exact mismatch {left!r} != {right!r}")


def anchor(raw):
    return dict(day=raw["first"][:10], first=raw["first"], last=raw["last"],
                high=raw["high"], low=raw["low"], close=raw["close"], count=raw["count"])


def projection(raw, cfg):
    keys(raw, "engine state")
    if raw["engine"] != cfg["engine"]:
        raise ParityError("wrong candidate engine")
    e = raw["state"]
    imo = cfg["engine"] == "imoexf"
    common = "previous previous2 position pending used_long used_short branch trades events"
    keys(e, common+(" sleeve observation mr_entries costs" if imo else " cfg metrics mr_scheduled"))
    if imo:
        compare(cfg["costs"], e["costs"], "cost_policy")
        if e["sleeve"] != cfg["sleeve"]:
            raise ParityError("wrong sleeve")
    else:
        compare(cfg["settings"], e["cfg"], "engine_config")
    a, a2 = e["previous"], e["previous2"]
    for raw_anchor in (a, a2):
        keys(raw_anchor, "first last high low close count"+("" if imo else " open numerator volume"))
    pp = qq = None
    p, q = e["position"], e["pending"]
    if p:
        keys(p, "entry price bars_held" if imo else "entry price stop take")
        ent = p["entry"]
        keys(ent, "owner side signal at exit_reason" if imo else "owner side signal action scale anchor stop1 stop2")
        if imo and ent["exit_reason"] is not None:
            raise ParityError("invalid entry action")
        pp = position(ent["owner"], ent["side"], ent["signal"], ent["at" if imo else "action"], p["price"],
            p.get("bars_held"), ent.get("scale"), ent.get("anchor"), ent.get("stop1"), ent.get("stop2"), p.get("stop"), p.get("take"))
    if q:
        if imo:
            keys(q, "owner side signal at exit_reason")
            qq = pending("exit" if q["exit_reason"] else "entry", q["owner"], q["side"], q["signal"], q["at"],
                         q["exit_reason"] or q["owner"].lower()+"_entry")
        elif set(q) == {"Entry"}:
            q = q["Entry"]
            keys(q, "owner side signal action scale anchor stop1 stop2")
            qq = pending("entry", q["owner"], q["side"], q["signal"], q["action"], q["owner"].lower()+"_entry",
                         q["scale"], q["anchor"], q["stop1"], q["stop2"])
        elif set(q) == {"Exit"} and p:
            q = q["Exit"]
            keys(q, "signal action reason ambiguous")
            qq = pending("exit", p["entry"]["owner"], p["entry"]["side"], q["signal"], q["action"],
                         q["reason"], ambiguous=q["ambiguous"])
        else:
            raise ParityError("unknown pending state")
    o = e["observation" if imo else "metrics"]
    keys(o, "first last high low close count"+("" if imo else " open numerator volume"))
    if not imo:
        for k in ("numerator", "volume"):
            keys(o[k], "sum correction")
            if not math.isfinite(o[k]["correction"]):
                raise ParityError("nonfinite Kahan correction")
    obs = dict(count=o["count"], first=o["first"], last=o["last"], high=o["high"], low=o["low"], close=o["close"],
               open=o.get("open"), numerator=None if imo else o["numerator"]["sum"],
               volume=None if imo else o["volume"]["sum"])
    for ev in e["events"]:
        keys(ev, "role ts component side signal_ts price"+(" sleeve" if imo else " model_id reason"))
        if imo and ev["sleeve"] != cfg["sleeve"]:
            raise ParityError("event identity mismatch")
        if not imo and ev["model_id"] != cfg["settings"]["model_id"]:
            raise ParityError("event model mismatch")
    for t in e["trades"]:
        fields = "model_id component side entry_ts entry_signal_ts entry_price exit_ts exit_signal_ts exit_price exit_reason gross_points gross_rub fees_rub net_rub cost_x2_net_rub"
        keys(t, fields+(" contract sleeve entry_event exit_event bars_held one_tick_net_rub" if imo
                        else " scale anchor stop1 stop2 stop_price take_price ambiguous_mr_bar tick_stress_net_rub"))
        if imo:
            model_id = "bo053_reaction07_control" if cfg["sleeve"] == "bo" else "k016_t020_s058_"+cfg["sleeve"]
            if (t["contract"] != "reaction_next_open" or t["sleeve"] != cfg["sleeve"]
                or t["entry_event"] != t["entry_ts"] or t["exit_event"] != t["exit_ts"] or t["model_id"] != model_id):
                raise ParityError("trade identity/timing mismatch")
        elif t["model_id"] != cfg["settings"]["model_id"]:
            raise ParityError("trade model mismatch")
    return dict(branch=e["branch"], mr_counter=e["mr_entries" if imo else "mr_scheduled"], mr_limit=2 if imo else None,
        bo_long_used=e["used_long"], bo_short_used=e["used_short"], observation=obs,
        features=dict(anchor=a["close"], scale=a["high"]-a["low"], normalizer=a["low"] if imo else o["close"],
                      previous_return=(a["close"]-a2["close"])/a2["close"] if imo else a["close"]/a2["close"]-1),
        previous=anchor(a), previous2=anchor(a2), position=pp, pending=qq,
        events=[event(v) for v in e["events"]], trades=[trade(v) for v in e["trades"]])


def project_case(actual):
    keys(actual, "id identity config routes frozen rows complete order_emission_allowed")
    result = copy.deepcopy(actual)
    result["frozen"] = frozen_projection(actual["frozen"])
    for row in result["rows"]:
        keys(row, "index kind route context_sequence context_accepted bars_processed model")
        row["model"] = projection(row["model"], result["config"])
    return result
