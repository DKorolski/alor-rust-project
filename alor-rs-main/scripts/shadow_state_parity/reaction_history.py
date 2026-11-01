"""Historical prices on an explicit numerical clock, never inferred live receipts."""
from __future__ import annotations

import copy
from datetime import date

import numpy as np
import pandas as pd

from compare import ParityError, file_hash, loads
from reaction_compare import compare
from reaction_fixtures import identity
from reaction_reference import expected_case, frame_for
from ri_reference import MSK
from weekend_reference import POLICIES, cutoff

SCHEMA = "weekend_reaction_history_v1"
EVIDENCE = "historical_prices_declared_close_clock_not_production_receipts"
CLOCK = "received=durable=processed=bar_label+600s; freeze=calendar_open"
START, END = "2026-07-20", "2026-10-28"
OHLCV = ["open", "high", "low", "close", "volume"]


def read_prices(path):
    frame = (pd.read_parquet(path) if path.suffix == ".parquet" else
             pd.read_csv(path, parse_dates=["ts_local"]).set_index("ts_local"))
    return validate_prices(frame)


def validate_prices(frame):
    if (not isinstance(frame.index, pd.DatetimeIndex) or frame.index.tz is not None
        or frame.index.hasnans or not frame.index.is_unique or not frame.index.is_monotonic_increasing
        or set(frame.columns) != set(OHLCV) or frame.empty):
        raise ParityError("raw prices require unique ordered naive MSK labels and exact OHLCV columns")
    frame = frame[OHLCV].astype(float)
    if (not np.isfinite(frame.to_numpy()).all() or (frame.low <= 0).any()
        or (frame.volume < 0).any() or (frame.low > frame[["open", "close"]].min(axis=1)).any()
        or (frame.high < frame[["open", "close"]].max(axis=1)).any()
        or (frame.index.minute % 10 != 0).any() or (frame.index.second != 0).any()
        or (frame.index.microsecond != 0).any() or (frame.index.nanosecond != 0).any()):
        raise ParityError("invalid OHLCV/grid; no silent repairs or forward fills")
    return frame


def configurations(usd_handoff):
    # Only gross-point/state parity is claimed. Zero fees are explicit identity,
    # not a replacement for the frozen (estimated) economic cost policy.
    configs = [dict(engine="imoexf", sleeve=s, costs=dict(point_value_rub=10.,
               tick_value_rub=5., rate_per_component_per_side=0., components=2))
               for s in ("mr", "bo", "combo")]
    spec = loads((usd_handoff/"shadow_contract.json").read_text())
    for hour, horizon, wait in ((7, 3, 4), (7, 4, 4), (7, 3, 2), (9, 3, 2)):
        matches = [p["engine"] for p in spec["profiles"] if
                   (p["engine"]["family"], p["engine"]["context_hour"], p["engine"]["clock_hour"],
                    p["engine"]["mr_horizon"], p["engine"]["bo_wait"], p["engine"]["mr_start_delay"])
                   == ("previous_day", hour, hour, horizon, wait, 0)]
        if len(matches) != 1:
            raise ParityError("frozen Previous-day profile coverage mismatch")
        e = matches[0]
        configs.append(dict(engine="usdrubf", settings={k:e[k] for k in
            ("model_id", "family", "context_hour", "clock_hour", "mr_horizon", "bo_wait", "mr_start_delay")}))
    return configs


def model_label(cfg):
    return "imoexf_"+cfg["sleeve"] if cfg["engine"] == "imoexf" else cfg["settings"]["model_id"]


def epoch(ts):
    return int(pd.Timestamp(ts).tz_localize(MSK).timestamp())


def event(symbol, ts, row, origin):
    return dict(symbol=symbol, close_time_utc=epoch(ts), o=float(row.open), h=float(row.high),
                l=float(row.low), close=float(row.close), v=float(row.volume), origin=origin)


class PriceContext:
    """Full-raw daily inventory independent of the receipt/context implementation."""
    def __init__(self, frame, hour):
        self.frame = validate_prices(frame)
        self.days, self.inventory = {}, []
        for day in pd.date_range(frame.index[0].normalize(), frame.index[-1].normalize()):
            label = day.date().isoformat()
            weekend = day.weekday() >= 5
            start = "09:50" if weekend else f"{9 if label < '2026-07-14' else hour:02}:00"
            end = "18:50" if weekend else "23:40"
            grid = pd.date_range(f"{label} {start}", f"{label} {end}", freq="10min")
            part = frame.loc[frame.index.isin(grid)]
            complete = (len(part) >= 44 and part.index[0] <= day+pd.Timedelta(hours=10)
                        and part.index[-1] == grid[-1]) if weekend else part.index.equals(grid)
            self.days[label] = part
            self.inventory.append(dict(day=label, weekend=weekend, rows=len(part),
                required=44 if weekend else len(grid), eligible=bool(complete),
                missing=[str(t) for t in grid.difference(part.index)],
                excluded_raw_rows=int(((frame.index.normalize() == day) & ~frame.index.isin(grid)).sum())))
        self.valid = [r["day"] for r in self.inventory if r["eligible"]]
        self.weekdays = [r["day"] for r in self.inventory if r["eligible"] and not r["weekend"]]

    def prior(self, day, policy):
        days = self.weekdays if policy == POLICIES[0] else self.valid
        dates = [d for d in days if d < day]
        if len(dates) < 2:
            raise ParityError(f"insufficient full-raw anchors: {day}/{policy}")
        return dates[-1], dates[-2]

    def bounded_history(self, day, symbol):
        # Two complete preceding weekdays suffice for BOTH policies. Include all
        # raw records since the older one, including rejected service/weekend bars.
        first = self.prior(day, POLICIES[0])[1]
        part = self.frame.loc[(self.frame.index >= first) & (self.frame.index < day)]
        if not 1 <= len(part) <= 1000:
            raise ParityError("bounded context exceeds harness capacity; do not truncate")
        return [dict(value=dict(sequence=i, source_id=f"historical-price:{symbol}:{ts.isoformat()}",
                    bar=event(symbol, ts, row, "history"), received_at_utc=epoch(ts)+600,
                    durable_at_utc=epoch(ts)+600), processed_at_utc=epoch(ts)+600)
                for i, (ts, row) in enumerate(part.iterrows(), 1)]

    def assert_full_raw_context(self, frozen, day, policy):
        for key, anchor_day in zip(("previous", "previous2"), self.prior(day, policy)):
            part = self.days[anchor_day]
            expected = dict(session_date=anchor_day, open=float(part.open.iloc[0]), high=float(part.high.max()),
                low=float(part.low.min()), close=float(part.close.iloc[-1]), range=float(part.high.max()-part.low.min()),
                distinct_bars=len(part), first_bar_utc=epoch(part.index[0]), last_bar_utc=epoch(part.index[-1]))
            compare(expected, {k:frozen[key][k] for k in expected}, "full_raw_context."+key)


def make_case(config, policy, day, prices):
    if day not in prices.weekdays:
        raise ParityError(f"incomplete/missing execution session: {day}; do not silently omit")
    cfg = identity(config, policy)
    cfg["receipt_stream"] = "historical-price-close-clock-v1"
    operations = []
    for i, (ts, row) in enumerate(prices.days[day].iterrows()):
        now = epoch(ts)+600
        operations.append(dict(kind="bar", value=event(cfg["symbol"], ts, row, "replay"), observed_at_utc=now))
        if i == 30 or ts.strftime("%H:%M") == "23:30":
            operations.append(dict(kind="restart", now=now))
    case = dict(id=f"{model_label(config)}/{policy}/{day}", identity=cfg, config=copy.deepcopy(config),
                history=prices.bounded_history(day, cfg["symbol"]), day=day, freeze_at_utc=cutoff(cfg, day),
                operations=operations)
    frame_for(case)
    return case


class HistoryPlan:
    def __init__(self, imo_path, usd_handoff, start=START, end=END):
        if not START <= start <= end <= END:
            raise ParityError("date range outside declared common historical window")
        date.fromisoformat(start), date.fromisoformat(end)
        self.dates = [d.date().isoformat() for d in pd.bdate_range(start, end)]
        if not self.dates:
            raise ParityError("empty evaluation range")
        self.configs = configurations(usd_handoff)
        usd_path = usd_handoff/"raw_model_input.csv"
        self.inputs = {str(p.resolve()):file_hash(p) for p in (imo_path, usd_path)}
        self.prices = {"imoexf_reaction07":PriceContext(read_prices(imo_path), 7)}
        usd = read_prices(usd_path)
        for h in (7, 9):
            self.prices[f"usdrubf_previous_day{h:02}"] = PriceContext(usd, h)
        for prices in self.prices.values():
            for day in self.dates:
                if day not in prices.weekdays:
                    raise ParityError(f"missing complete common session {day}")
                for policy in POLICIES:
                    prices.prior(day, policy)

    def cases(self, days):
        return [make_case(cfg, policy, day, self.prices[identity(cfg, policy)["profile"]])
                for day in days for cfg in self.configs for policy in POLICIES]

    def fixture(self, days):
        return dict(schema_version=SCHEMA, evidence_kind=EVIDENCE, cases=self.cases(days))

    def expected(self, fixture, days, modules):
        # Reconstruct coverage and input values from raw prices, not two traces
        # which could both omit the same session/variant/bar.
        compare(self.fixture(days), fixture, "historical_input")
        result = []
        for case in fixture["cases"]:
            e = expected_case(case, modules[case["config"]["engine"]])
            self.prices[case["identity"]["profile"]].assert_full_raw_context(
                e["frozen"], case["day"], case["identity"]["policy"])
            result.append(e)
        return result
