"""Independent executable RI prospective numerical contract.

Not the April interval-pruned research combo, not historical receipt evidence.
No Rust imports, candidate traces, expected trades or broker dependencies.
"""
from __future__ import annotations

import copy
from datetime import datetime, timedelta, timezone

MSK = timezone(timedelta(hours=3))
POLICIES = ("runtime_accepted_only", "ri_confirmed_closing_context_v1")


def local(epoch):
    return datetime.fromtimestamp(epoch, MSK).replace(tzinfo=None)


def opening(day):
    return "09:00" if day < "2026-07-14" else "07:00"


def is_model_bar(bar):
    dt = local(bar["close_time_utc"])
    return (dt.weekday() < 5 and dt.minute % 10 == 0 and dt.second == 0
            and opening(dt.date().isoformat()) <= dt.strftime("%H:%M") <= "23:40")


def context(warmup, policy):
    days = {}
    for bar in warmup:
        dt = local(bar["close_time_utc"])
        if not is_model_bar(bar):
            continue
        if bar["origin"] == "history_gap" and not (
            policy == POLICIES[1] and dt.strftime("%H:%M") == "23:40"
        ):
            continue
        days.setdefault(dt.date().isoformat(), []).append(bar)
    anchors = []
    for day, bars in sorted(days.items()):
        bars = sorted(bars, key=lambda b: b["close_time_utc"])
        first_limit = "09:10" if opening(day) == "09:00" else "07:10"
        required = 80 if opening(day) == "09:00" else 92
        if len(bars) < required or local(bars[0]["close_time_utc"]).strftime("%H:%M") > first_limit or local(bars[-1]["close_time_utc"]).strftime("%H:%M") < "23:30":
            continue
        high, low = max(b["h"] for b in bars), min(b["l"] for b in bars)
        anchors.append(dict(date=day, high=high, low=low, close=bars[-1]["close"],
                            range=high-low, bars=len(bars)))
    return {"previous": anchors[-1] if anchors else None,
            "previous2": anchors[-2] if len(anchors) >= 2 else None}


class Reference:
    """One frozen session, MR first; assumed fills only after BOTH components."""

    def __init__(self, warmup, day, policy):
        self.context = context(warmup, policy)
        self.ready = all(self.context.values())
        self.prev = self.context["previous"] if self.ready else None
        self.prev2 = self.context["previous2"] if self.ready else None
        self.day = day
        self.mr = self.bo = self.pending = None
        self.mr_total = self.bo_total = self.count = 0
        self.first = None
        self.high = self.low = self.long_level = self.short_level = None
        d = datetime.fromisoformat(day)
        self.allowed = bool(self.ready and d.weekday() != 4 and not "05-21" <= day[5:] <= "07-01")
        self.used_long = self.used_short = False
        self.phase = "flat"

    def action(self, owner, role, pos, reason, price=None):
        pnl = None if role == "entry" else (price-pos["entry_price"]) * (1 if pos["side"] == "long" else -1) - (2 if owner == "mr" else 0)
        self.actions.append(dict(component="author41_mr" if owner == "mr" else "author42_bo",
                                 role=role, side=pos["side"], reason=reason,
                                 scheduled_ts=self.dt.isoformat(" "), pnl=pnl))
        self.phase = "live_pending_entry" if role == "entry" else "live_pending_exit"

    def mr_step(self, b):
        if self.mr:
            p = self.mr
            p["bars_held"] += 1
            sign = 1 if p["side"] == "long" else -1
            stop = p["prev_close"] - sign * (1 if sign == 1 else .75) * p["prev_range"]
            take = p["prev_close"] + sign * (.005 if sign == 1 else .020) * p["prev_range"]
            reason, price = None, None
            if (sign == 1 and b["l"] <= stop) or (sign == -1 and b["h"] >= stop):
                reason, price = "stop", stop
            elif (b["close"]-take)*sign > 0:
                reason, price = "take_author_close", b["close"]
            elif self.time >= "20:00":
                reason, price = "time_exit", b["close"]
            elif p["bars_held"] > 20 and ((sign == 1 and b["h"] >= p["entry_price"]) or (sign == -1 and b["l"] <= p["entry_price"])):
                reason, price = "breakeven_limit", p["entry_price"]
            self.branches.append("mr:exit:"+reason if reason else "mr:hold")
            if reason:
                self.action("mr", "exit", p, reason, price)
            return
        if self.mr_total >= 2:
            self.branches.append("mr:entry_quota")
            return
        if self.bo:
            self.branches.append("mr:bo_owns_slot")
            return
        side = None
        if self.prev and self.time <= "10:00" and .005 < self.prev["range"]/self.prev["low"] < .100:
            diff = b["close"]-self.prev["close"]
            if 0 < diff < .20*self.prev["range"]:
                side = "short"
            elif -.11*self.prev["range"] < diff < 0:
                side = "long"
        self.branches.append("mr:entry" if side else "mr:no_entry_signal")
        if side:
            self.mr = dict(side=side, entry_ts=self.dt.isoformat(), entry_price=b["close"],
                           prev_close=self.prev["close"], prev_range=self.prev["range"], bars_held=0)
            self.mr_total += 1
            self.action("mr", "entry", self.mr, "live_entry")

    def hour_check(self):
        return self.dt.minute == 50 and self.dt > self.first + timedelta(minutes=50)

    def observe(self, b):
        if self.first is None:
            self.first = self.dt
        self.high = max(self.high if self.high is not None else b["h"], b["h"])
        self.low = min(self.low if self.low is not None else b["l"], b["l"])
        if self.count == 5 and self.prev:
            self.long_level = max(self.prev["close"] + .42*self.prev["range"], b["close"], self.first_high)
            self.short_level = min(self.prev["close"] - .42*self.prev["range"], b["close"], self.first_low)
            if abs(b["close"]-self.prev["close"]) > 1.5*self.prev["range"]:
                self.allowed = False
        if self.count == 0:
            self.first_high, self.first_low = b["h"], b["l"]
        self.count += 1
        self.branches.append("bo:observed")

    def schedule(self, b):
        reason = None
        if not self.prev:
            reason = "context_unavailable"
        elif self.pending:
            reason = "pending_exists"
        elif self.bo:
            reason = "bo_owns_slot"
        elif self.mr:
            reason = "mr_owns_slot"
        elif not self.allowed:
            reason = "session_disallowed"
        elif self.long_level is None:
            reason = "levels_unavailable"
        elif self.time >= "23:00":
            reason = "terminal_time"
        if reason:
            self.branches.append("bo_entry:"+reason)
            return
        side = None
        reason = "no_entry_signal"
        if self.used_long and b["h"] >= self.high:
            reason, side = "reentry_long", "long"
        elif self.used_short and b["l"] <= self.low:
            reason, side = "reentry_short", "short"
        elif self.hour_check() and self.prev["high"]/self.prev["low"] > 1.01:
            ret = self.prev["close"]/self.prev2["close"]-1
            if ret > -.025 and b["close"] > self.long_level:
                reason, side = "hour_long", "long"
            elif ret < .025 and b["close"] < self.short_level:
                reason, side = "hour_short", "short"
        self.branches.append("bo_entry:"+reason)
        if side:
            self.pending = dict(kind="entry", side=side)

    def bo_step(self, b):
        self.observe(b)
        if self.phase in ("live_pending_entry", "live_pending_exit"):
            self.branches.append("bo:"+self.phase.removeprefix("live_"))
            return
        pending, self.pending = self.pending, None
        if pending and pending["kind"] == "entry":
            if self.time >= "23:00":
                self.branches.append("bo:pending_entry_terminal")
            elif self.mr or self.bo:
                self.branches.append("bo:pending_entry_occupied")
            else:
                self.bo = dict(side=pending["side"], entry_ts=self.dt.isoformat(), entry_price=b["o"], bars_held=0)
                self.bo_total += 1
                if pending["side"] == "long":
                    self.used_long = True
                else:
                    self.used_short = True
                self.branches.append("bo:entry")
                self.action("bo", "entry", self.bo, "live_entry_next_bar")
        if pending and pending["kind"] == "exit":
            if self.bo is None:
                raise ValueError("invalid reference pending exit")
            self.branches.append("bo:exit_pending")
            self.action("bo", "exit", self.bo, pending["reason"], b["o"])
            return
        if self.bo:
            self.bo["bars_held"] += 1
            if self.time >= "23:00":
                self.branches.append("bo:exit_eod")
                self.action("bo", "exit", self.bo, "time_exit_same_bar_close", b["close"])
            else:
                self.branches.append("bo:hold")
                sign = 1 if self.bo["side"] == "long" else -1
                delta = (b["close"]-self.prev["close"])*sign
                suffix = self.bo["side"]+"_next_open"
                if delta < .18*self.prev["range"]:
                    self.pending = dict(kind="exit", reason="stop_emergency_"+suffix)
                elif self.hour_check() and delta < .50*self.prev["range"]:
                    self.pending = dict(kind="exit", reason="stop_hour_"+suffix)
        self.schedule(b)

    def step(self, b):
        self.dt = local(b["close_time_utc"])
        self.time = self.dt.strftime("%H:%M")
        self.branches, self.actions = [], []
        self.mr_step(b)
        self.bo_step(b)
        if self.phase == "live_pending_entry":
            self.phase = "live_in_position"
        elif self.phase == "live_pending_exit":
            self.mr = self.bo = self.pending = None
            self.phase = "flat"
        position = self.mr or self.bo
        return copy.deepcopy(dict(
            context=self.context, phase=self.phase, mr_total=self.mr_total, bo_total=self.bo_total,
            lifecycle_owner="author41_mr" if self.mr else "author42_bo" if self.bo else None,
            lifecycle_side=position["side"] if position else None,
            lifecycle_entry_ts=position["entry_ts"].replace("T", " ") if position else None,
            mr=self.mr, bo=self.bo,
            pending=None if self.pending is None else {k:self.pending.get(k) for k in ("kind", "side", "reason")},
            observation_count=self.count,
            first_bar=self.first.isoformat(), long_level=self.long_level, short_level=self.short_level,
            day_high=self.high, day_low=self.low, trade_allowed=self.allowed,
            long_used=self.used_long, short_used=self.used_short, branches=self.branches, actions=self.actions,
        ))
