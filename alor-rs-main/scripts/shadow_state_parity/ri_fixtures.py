"""Versioned synthetic inputs only; expected state is produced independently."""
from datetime import datetime, timedelta

from ri_reference import MSK

SCHEMA = "ri_prospective_fixture_v1"


def bar(day, time, close, *, high=None, low=None, opening=None, origin="live"):
    ts = int(datetime.fromisoformat(day+"T"+time).replace(tzinfo=MSK).timestamp())
    opening = close if opening is None else opening
    return dict(symbol="RIZ6", close_time_utc=ts, close=float(close), o=float(opening),
                h=float(max(close, opening)+10 if high is None else high),
                l=float(min(close, opening)-10 if low is None else low), v=10., origin=origin)


def warmup(day, gap=False, previous2_close=90000., missing_previous2=False):
    dates, d = [], datetime.fromisoformat(day)
    while len(dates) < 2:
        d -= timedelta(days=1)
        if d.weekday() < 5:
            dates.insert(0, d.date().isoformat())
    result = []
    for day in dates[1:] if missing_previous2 else dates:
        close = previous2_close if day == dates[0] else 90000.
        for m in range(420, 1420, 10):
            result.append(bar(day, f"{m//60:02}:{m%60:02}", close,
                              high=max(91000., close), low=89000., origin="history"))
    if gap:
        result.append(bar(dates[-1], "23:40", 90500, high=91000, low=89000, origin="history_gap"))
    return result


def fixtures():
    cases = []

    def add(name, specs, day="2026-10-13", **kwargs):
        events = [bar(day, t, price, **opts) for t, price, opts in specs]
        cases.append(dict(id=name, day=day, warmup=warmup(day, **kwargs), events=events))

    def point(t, price, **opts):
        return t, price, opts

    prefix = [point(f"07:{m:02}", 90000) for m in range(0, 60, 10)]
    quota = [point("07:00", 89900), point("07:10", 90200), point("07:20", 90100),
             point("07:30", 89900), point("07:40", 89900), point("07:50", 89900),
             point("08:50", 91000), point("09:00", 91000), point("23:00", 91100)]
    add("mr_shared_quota_and_bo_observation", quota)
    add("closing_context_pair", quota, gap=True)
    add("missing_previous2", quota, missing_previous2=True)
    add("short_strict_boundaries_and_stop_priority", [
        point("07:00", 90400), point("07:10", 89780), point("07:20", 90399),
        point("07:30", 89960), point("07:40", 89959), point("07:50", 90001),
        point("08:00", 89900, high=91500), point("08:10", 90100)])
    add("long_strict_take_and_stop_priority", [
        point("07:00", 89800), point("07:10", 90010), point("07:20", 90011),
        point("07:30", 89999), point("07:40", 90100, low=88000), point("07:50", 89900)])
    add("mr_inclusive_window_no_forced_10_exit", prefix + [
        point("10:00", 89900), point("10:10", 89890), point("20:00", 89890)])
    add("mr_after_window_no_entry", prefix + [point("10:10", 89900), point("20:00", 90100)])
    for side, entry, held, trigger in [("long", 89900, 89850, {"high": 89900}),
                                       ("short", 90100, 90150, {"low": 90100})]:
        hold = [point("07:00", entry)]
        for index in range(1, 22):
            m = 420 + index*10
            hold.append(point(f"{m//60:02}:{m%60:02}", held, **(trigger if index == 21 else {})))
        add("mr_breakeven_21_"+side, hold)
    add("bo_terminal_pending_entry", prefix + [point("22:50", 91000), point("23:00", 91000), point("23:10", 91100)])
    three = prefix + [point("08:50", 91000), point("09:00", 91000, opening=91010),
                      point("09:10", 90000), point("09:20", 90000, opening=90020),
                      point("09:30", 91200), point("09:40", 91200), point("09:50", 90400),
                      point("10:00", 90400), point("10:10", 91300), point("10:20", 91300),
                      point("23:00", 91400), point("23:10", 91500)]
    add("bo_three_entries_long", three)
    mirrored = [(t, 180000-p, {("low" if k == "high" else "high" if k == "low" else k):180000-v
                              for k, v in opts.items()}) for t, p, opts in three]
    add("bo_three_entries_short", mirrored)
    add("mr_priority_over_pending_bo", prefix + [
        point("08:50", 91000), point("09:00", 90100), point("09:10", 90100),
        point("09:20", 89900), point("09:30", 89900), point("09:40", 90200),
        point("09:50", 91100), point("10:00", 91100), point("23:00", 91200)])
    add("friday_mr_allowed_bo_blocked", quota, day="2026-10-16")
    add("bo_first_hour_extreme", prefix[:-1] + [point("07:50", 94000), point("08:50", 95000)])
    add("bo_previous_return_gate", prefix + [point("08:50", 91000), point("09:50", 89000),
                                            point("10:00", 89000), point("23:00", 89000)], previous2_close=93000)
    return dict(schema_version=SCHEMA, evidence_kind="synthetic_model_clock", cases=cases)
