import asyncio
import csv
import json
import logging
import signal
import sys
from dataclasses import dataclass
from typing import Dict, List, Optional, Tuple

import numpy as np
import pandas as pd


# ========= CONFIG =========

@dataclass
class StrategyConfig:
    symbol: str = "BTCUSDT"
    depth: int = 50  # orderbook depth в топике, напр. orderbook.200.BTCUSDT

    tick_size: float = 0.1
    order_size: float = 1

    fee_rate_maker: float = 0.0
    fee_rate_taker: float = 0.0066 / 100.0
    broker_fee_abs: float = 1.0

    tp_ticks: int = 500

    # вход по кластерам
    entry_cluster_q: float = 0.995
    max_cluster_depth_entry: int = 3

    # выход по кластерам
    exit_cluster_q: float = 0.05
    max_cluster_depth_exit: int = 10
    exit_order_timeout_ms: int = 500
    exit_cluster_max_diff_ticks: int = 495
    exit_cluster_start_ms: int = 500  # EXIT_CLUSTER_START_MS из оффлайна

    # таймаут входной лимитки
    entry_order_timeout_ms: int = 800

    # онлайн-квантили по кластерам
    min_history_for_quantiles: int = 50

    # Параметр защитного выхода при движении против на ADVERSE_TICKS тиков
    adverse_ticks: int = 100  # как в оффлайн V10_OFFLINE

    # Насколько кластер доминирует противоположный (CLUSTER_DIFF)
    cluster_diff: float = 1.0

    # Пороговые значения кластеров (можно задать вручную из оффлайна)
    # Если оставить None, можно раскомментировать quantile-логику ниже
    entry_bid_thr: Optional[float] = 15.4710
    entry_ask_thr: Optional[float] = 14.3710
    exit_bid_thr: Optional[float] = 0.2130
    exit_ask_thr: Optional[float] = 0.1660

    csv_path: str = "trades_live_v11_live.csv"

    # ===== минимальная задержка появления ордера в книге =====
    # Ордер нельзя исполнить, пока ts трейда < order_ts + order_placement_delay_ms
    order_placement_delay_ms: int = 100  # ms, можно крутить (ping ~10ms)


# ========= DATA MODELS =========

@dataclass
class TradeResult:
    entry_time: pd.Timestamp
    exit_time: pd.Timestamp
    direction: str          # "long" / "short"
    side: int               # +1 / -1
    entry_price: float
    exit_price: float
    reason: str             # "cluster_exit" | "adverse_exit" | "eod_force" | "limit_exit"
    gross_ticks: float
    net_ticks: float
    gross_ret: float
    net_ret: float
    waited_ms_for_fill: int
    entry_fee: float
    exit_fee: float
    total_fee: float
    entry_liquidity: str
    exit_liquidity: str     # "maker" / "taker"
    tp_plain: float
    exit_from_cluster: bool
    hold_ms: int


class OrderbookL2:
    """
    Поддерживает локальный L2 стакан по Bybit snapshot/delta.
    Храним все уровни, но стратегии нужны только top N.
    """

    def __init__(self, depth: int):
        self.depth = depth
        self.bids: Dict[float, float] = {}  # price -> size
        self.asks: Dict[float, float] = {}
        self.last_ts: Optional[int] = None

    def _apply_levels(self, side_dict: Dict[float, float], levels: List[List[str]]):
        for price_str, size_str in levels:
            price = float(price_str)
            size = float(size_str)
            if size == 0.0:
                side_dict.pop(price, None)
            else:
                side_dict[price] = size

    def apply_snapshot(self, data: dict):
        self.bids.clear()
        self.asks.clear()
        self._apply_levels(self.bids, data.get("b", []))
        self._apply_levels(self.asks, data.get("a", []))

    def apply_delta(self, data: dict):
        self._apply_levels(self.bids, data.get("b", []))
        self._apply_levels(self.asks, data.get("a", []))

    def top_levels(self, max_levels: int) -> Tuple[List[Tuple[float, float]], List[Tuple[float, float]]]:
        bids_sorted = sorted(self.bids.items(), key=lambda kv: kv[0], reverse=True)[:max_levels]
        asks_sorted = sorted(self.asks.items(), key=lambda kv: kv[0])[:max_levels]
        return bids_sorted, asks_sorted

    def best_bid_ask(self) -> Optional[Tuple[float, float, float, float]]:
        bids, asks = self.top_levels(1)
        if not bids or not asks:
            return None
        (bbp, bbv), (bap, bav) = bids[0], asks[0]
        return bbp, bbv, bap, bav


# ========= STRATEGY ENGINE =========

class ScalpingV10OfflineLiveEngine:
    """
    Live-версия оффлайн backtest_tick_v10_offline:

    - entry/exit лимитки с моделью очереди
    - cluster_exit — пассивная лимитка (maker) по уровню кластера
    - adverse_exit — лимитка maker по best bid/ask с очередью
    - таймауты для входа/выхода
    """

    def __init__(self, cfg: StrategyConfig):
        self.cfg = cfg

        # Состояние
        self.state: str = "flat"  # flat / waiting_entry / in_position
        self.side: int = 0        # +1 long / -1 short

        # Лимитка входа
        self.order_price: Optional[float] = None
        self.order_ts: Optional[int] = None
        self.queue_ahead: float = 0.0
        self.entry_order_ts: Optional[int] = None

        # Позиция
        self.entry_price: Optional[float] = None
        self.entry_ts: Optional[int] = None
        self.entry_time: Optional[pd.Timestamp] = None
        self.tp_plain: Optional[float] = None

        # Лимитка выхода (cluster / adverse)
        self.exit_order_price: Optional[float] = None
        self.exit_order_ts: Optional[int] = None
        self.exit_queue_ahead: float = 0.0
        self.exit_reason_target: Optional[str] = None  # "cluster_exit" | "adverse_exit" | None

        # "лучшая" цена в сторону позиции с момента входа
        self.favorable_price: Optional[float] = None

        # Истории кластеров (берём максимум стенки среди top-N уровней)
        self.bid_cluster_hist: List[float] = []
        self.ask_cluster_hist: List[float] = []

        # Квантили (обновляются онлайн / или берутся из cfg)
        self.bid_entry_thr: Optional[float] = None
        self.ask_entry_thr: Optional[float] = None
        self.bid_exit_thr: Optional[float] = None
        self.ask_exit_thr: Optional[float] = None

        # Последние цены
        self.last_trade_price: Optional[float] = None
        self.last_mid: Optional[float] = None
        self.last_time: Optional[pd.Timestamp] = None

        # Ликвидность входа/выхода по последнему трейду
        self.entry_liquidity: Optional[str] = None
        self.exit_liquidity: Optional[str] = None

        # Был ли entry-ордер агрессивным на момент постановки (кросс спрэда)
        self.entry_order_is_aggressive: bool = False
        # Был ли exit-ордер агрессивным на момент постановки (для будущего использования)
        self.exit_order_is_aggressive: bool = False

        # Статистика доли тейкерных исполнений
        self.n_entry_taker: int = 0
        self.n_exit_taker: int = 0

        # Статистика
        self.n_signals = 0
        self.n_orders_placed = 0
        self.n_filled = 0
        self.n_order_timeouts = 0
        self.n_exit_orders_placed = 0
        self.n_exit_orders_timeout = 0

        self.trades: List[TradeResult] = []

        # CSV
        self._csv_file = open(self.cfg.csv_path, "a", newline="", encoding="utf-8")
        self._csv_writer = csv.writer(self._csv_file)
        if self._csv_file.tell() == 0:
            self._csv_writer.writerow(
                [
                    "entry_time",
                    "exit_time",
                    "direction",
                    "side",
                    "entry_price",
                    "exit_price",
                    "gross_ticks",
                    "net_ticks",
                    "gross_ret",
                    "net_ret",
                    "reason",
                    "entry_liquidity",
                    "exit_liquidity",
                    "tp_plain",
                    "exit_from_cluster",
                    "entry_fee",
                    "exit_fee",
                    "total_fee",
                    "waited_ms_for_fill",
                    "hold_ms",
                ]
            )
            self._csv_file.flush()

        self.PRICE_EPS = 1e-9

    # ----- helpers -----

    def _update_cluster_history_and_thresholds(self, cbv: float, cav: float):
        """
        История объёмов кластеров и обновление порогов.
        Сейчас пороги берутся из cfg.*_thr, но при желании можно включить quantile-режим.
        """
        if cbv > 0:
            self.bid_cluster_hist.append(cbv)
        if cav > 0:
            self.ask_cluster_hist.append(cav)

        if (
            len(self.bid_cluster_hist) >= self.cfg.min_history_for_quantiles
            and len(self.ask_cluster_hist) >= self.cfg.min_history_for_quantiles
        ):
            b = np.asarray(self.bid_cluster_hist, dtype=float)
            a = np.asarray(self.ask_cluster_hist, dtype=float)

            self.bid_entry_thr = (
                self.cfg.entry_bid_thr
                if self.cfg.entry_bid_thr is not None
                else float(np.quantile(b, self.cfg.entry_cluster_q))
            )
            self.ask_entry_thr = (
                self.cfg.entry_ask_thr
                if self.cfg.entry_ask_thr is not None
                else float(np.quantile(a, self.cfg.entry_cluster_q))
            )
            self.bid_exit_thr = (
                self.cfg.exit_bid_thr
                if self.cfg.exit_bid_thr is not None
                else float(np.quantile(b, self.cfg.exit_cluster_q))
            )
            self.ask_exit_thr = (
                self.cfg.exit_ask_thr
                if self.cfg.exit_ask_thr is not None
                else float(np.quantile(a, self.cfg.exit_cluster_q))
            )

    def _thresholds_ready(self) -> bool:
        return (
            self.bid_entry_thr is not None
            and self.ask_entry_thr is not None
            and self.bid_exit_thr is not None
            and self.ask_exit_thr is not None
        )

    # ----- L2 handler -----

    def on_orderbook(
        self,
        ts: int,
        bbp: float,
        bbv: float,
        bap: float,
        bav: float,
        cbp: Optional[float],
        cbv: float,
        cbd: Optional[int],
        cap: Optional[float],
        cav: float,
        cad: Optional[int],
    ):
        """
        Каждое обновление стакана (уже с рассчитанными кластерами).
        """

        cur_time = pd.to_datetime(ts, unit="ms", utc=True)
        self.last_mid = (bbp + bap) / 2.0
        self.last_time = cur_time

        # обновляем истории кластеров/квантили
        self._update_cluster_history_and_thresholds(cbv, cav)

        # обновляем "лучшую" цену в сторону позиции
        if self.state == "in_position" and self.entry_price is not None:
            if self.side == +1:
                # long — лучший bbp максимально высокий
                if self.favorable_price is None:
                    self.favorable_price = max(self.entry_price, bbp)
                else:
                    self.favorable_price = max(self.favorable_price, bbp)
            elif self.side == -1:
                # short — лучший bap минимальный
                if self.favorable_price is None:
                    self.favorable_price = min(self.entry_price, bap)
                else:
                    self.favorable_price = min(self.favorable_price, bap)

        # ---- timeout лимитки входа ----
        if self.state == "waiting_entry" and self.order_ts is not None:
            if ts - self.order_ts >= self.cfg.entry_order_timeout_ms:
                logging.debug("Entry limit timeout -> cancel order")
                self.state = "flat"
                self.side = 0
                self.order_price = None
                self.order_ts = None
                self.queue_ahead = 0.0
                self.entry_order_ts = None
                self.n_order_timeouts += 1

        # ---- timeout лимитки выхода (cluster/adverse) ----
        if self.state == "in_position" and self.exit_order_price is not None and self.exit_order_ts is not None:
            if ts - self.exit_order_ts >= self.cfg.exit_order_timeout_ms:
                logging.debug("Exit limit timeout -> cancel exit order")
                self.exit_order_price = None
                self.exit_order_ts = None
                self.exit_queue_ahead = 0.0
                self.exit_reason_target = None
                self.n_exit_orders_timeout += 1

        # ---- cluster exit (пассивная лимитка maker, только после exit_cluster_start_ms) ----
        if (
            self.state == "in_position"
            and self.entry_ts is not None
            and self.exit_order_price is None
            and self.tp_plain is not None
            and self._thresholds_ready()
            and ts - self.entry_ts >= self.cfg.exit_cluster_start_ms
        ):
            # LONG: смотрим ask-кластер, ставим пассивную sell-лимитку
            if self.side == +1:
                is_big_ask = (
                    cav >= self.ask_exit_thr
                    and cad is not None
                    and cad <= self.cfg.max_cluster_depth_exit
                    and cap is not None
                )
                if is_big_ask:
                    cap_f = float(cap)
                    raw_price = cap_f - self.cfg.tick_size
                    # Для sell-лимитки нельзя быть ниже best ask, иначе это агрессивный ордер.
                    candidate = max(raw_price, bap)
                    if abs(candidate - self.tp_plain) <= self.cfg.exit_cluster_max_diff_ticks * self.cfg.tick_size:
                        logging.debug(
                            "Place cluster-exit limit (long) @ %.1f (raw=%.1f, bap=%.1f)",
                            candidate, raw_price, bap
                        )
                        self.exit_order_price = candidate
                        self.exit_order_ts = ts

                        # Если стоим на best ask — мы за текущим объёмом
                        if abs(candidate - bap) <= self.PRICE_EPS:
                            self.exit_queue_ahead = max(bav, 0.0)
                        else:
                            # глубже в стакане — считаем, что мы первые
                            self.exit_queue_ahead = 0.0

                        self.exit_reason_target = "cluster_exit"
                        # кластерный выход — всегда пассивный
                        self.exit_order_is_aggressive = False
                        self.n_exit_orders_placed += 1

            # SHORT: смотрим bid-кластер, ставим пассивную buy-лимитку
            elif self.side == -1:
                is_big_bid = (
                    cbv >= self.bid_exit_thr
                    and cbd is not None
                    and cbd <= self.cfg.max_cluster_depth_exit
                    and cbp is not None
                )
                if is_big_bid:
                    cbp_f = float(cbp)
                    raw_price = cbp_f + self.cfg.tick_size
                    # Для buy-лимитки нельзя быть выше best bid, иначе это агрессивный ордер.
                    candidate = min(raw_price, bbp)
                    if abs(candidate - self.tp_plain) <= self.cfg.exit_cluster_max_diff_ticks * self.cfg.tick_size:
                        logging.debug(
                            "Place cluster-exit limit (short) @ %.1f (raw=%.1f, bbp=%.1f)",
                            candidate, raw_price, bbp
                        )
                        self.exit_order_price = candidate
                        self.exit_order_ts = ts

                        if abs(candidate - bbp) <= self.PRICE_EPS:
                            self.exit_queue_ahead = max(bbv, 0.0)
                        else:
                            self.exit_queue_ahead = 0.0

                        self.exit_reason_target = "cluster_exit"
                        self.exit_order_is_aggressive = False
                        self.n_exit_orders_placed += 1

        # ---- защитный выход при движении против на adverse_ticks ----
        if (
            self.state == "in_position"
            and self.entry_price is not None
            and self.exit_order_price is None
            and self.favorable_price is not None
        ):
            if self.side == +1:
                adverse_ticks = (self.favorable_price - bbp) / self.cfg.tick_size
                if adverse_ticks >= self.cfg.adverse_ticks:
                    # выходим как maker по лучшему ask, очередь — весь объём на best ask
                    limit_price = bap
                    queue = max(bav, 0.0)
                    logging.debug("Place adverse-exit limit (long) at %.1f, adverse_ticks=%.1f",
                                  limit_price, adverse_ticks)
                    self.exit_order_price = limit_price
                    self.exit_order_ts = ts
                    self.exit_queue_ahead = queue
                    self.exit_reason_target = "adverse_exit"
                    # adverse в этой модели — тоже пассивный
                    self.exit_order_is_aggressive = False
                    self.n_exit_orders_placed += 1
            elif self.side == -1:
                adverse_ticks = (bap - self.favorable_price) / self.cfg.tick_size
                if adverse_ticks >= self.cfg.adverse_ticks:
                    # выходим как maker по лучшему bid
                    limit_price = bbp
                    queue = max(bbv, 0.0)
                    logging.debug("Place adverse-exit limit (short) at %.1f, adverse_ticks=%.1f",
                                  limit_price, adverse_ticks)
                    self.exit_order_price = limit_price
                    self.exit_order_ts = ts
                    self.exit_queue_ahead = queue
                    self.exit_reason_target = "adverse_exit"
                    self.exit_order_is_aggressive = False
                    self.n_exit_orders_placed += 1

        # ---- entry сигналы ----
        if self.state == "flat" and self._thresholds_ready():
            # Bid Entry -> long
            is_bid_entry = (
                cbv >= self.bid_entry_thr
                and cbd is not None
                and cbd <= self.cfg.max_cluster_depth_entry
                # and cbv >= self.cfg.cluster_diff * cav
            )

            # Ask Entry -> short
            is_ask_entry = (
                cav >= self.ask_entry_thr
                and cad is not None
                and cad <= self.cfg.max_cluster_depth_entry
                # and cav >= self.cfg.cluster_diff * cbv
            )

            if is_bid_entry and cbp is not None:
                logging.debug("LONG signal at ts=%d", ts)
                logging.debug(
                    "LONG signal: ts=%d cbp=%.1f cbv=%.3f cav=%.3f bid_thr=%.3f ask_thr=%.3f",
                    ts, cbp, cbv, cav,
                    self.bid_entry_thr, self.ask_entry_thr
                )
                self.state = "waiting_entry"
                self.side = +1
                candidate = cbp + self.cfg.tick_size
                if candidate > bbp + self.PRICE_EPS:
                    candidate = bbp
                self.order_price = candidate
                self.order_ts = ts
                self.entry_order_ts = ts

                # если лимитка совпадает с best bid — учитываем очередь, иначе считаем, что мы первые
                if abs(self.order_price - bbp) <= self.PRICE_EPS:
                    self.queue_ahead = max(bbv, 0.0)
                else:
                    self.queue_ahead = 0.0

                self.entry_liquidity = None
                # агрессивный (кросс спрэда), если цена >= лучшего ask
                self.entry_order_is_aggressive = self.order_price >= bap - self.PRICE_EPS

                self.n_signals += 1
                self.n_orders_placed += 1

            elif is_ask_entry and cap is not None:
                logging.debug("SHORT signal at ts=%d", ts)
                logging.debug(
                    "SHORT signal: ts=%d cap=%.1f cbv=%.3f cav=%.3f bid_thr=%.3f ask_thr=%.3f",
                    ts, cap, cbv, cav,
                    self.bid_entry_thr, self.ask_entry_thr
                )
                self.state = "waiting_entry"
                self.side = -1
                candidate = cap - self.cfg.tick_size
                if candidate < bap - self.PRICE_EPS:
                    candidate = bap
                self.order_price = candidate
                self.order_ts = ts
                self.entry_order_ts = ts

                if abs(self.order_price - bap) <= self.PRICE_EPS:
                    self.queue_ahead = max(bav, 0.0)
                else:
                    self.queue_ahead = 0.0

                self.entry_liquidity = None
                # агрессивный, если цена <= лучшего bid
                self.entry_order_is_aggressive = self.order_price <= bbp + self.PRICE_EPS

                self.n_signals += 1
                self.n_orders_placed += 1

    # ----- трейды -----

    def on_trade(self, ts: int, price: float, size: float):
        """
        Каждый trade.
        """
        cur_time = pd.to_datetime(ts, unit="ms", utc=True)
        self.last_trade_price = price
        self.last_time = cur_time

        cfg = self.cfg

        # ---- фильтр по времени появления ордера в книге (order_placement_delay_ms) ----
        if self.state == "waiting_entry" and self.order_ts is not None:
            visible_ts = self.order_ts + cfg.order_placement_delay_ms
            if ts < visible_ts:
                return

        if self.state == "in_position" and self.exit_order_price is not None and self.exit_order_ts is not None:
            visible_ts = self.exit_order_ts + cfg.order_placement_delay_ms
            if ts < visible_ts:
                return

        # ---- fill входной лимитки (очередь + проход цены) ----
        if self.state == "waiting_entry" and self.order_price is not None and self.side != 0:
            filled_entry = False
            entry_is_taker = False

            if self.side == +1:
                # Лонг: лимитка на покупку по цене order_price
                if price < self.order_price - self.PRICE_EPS:
                    # цена ещё не дошла до нашего уровня – ничего не делаем
                    pass
                else:
                    # price >= order_price:
                    # Цена дошла до нашего уровня или проскочила его.
                    # Мы сидим пассивно в очереди, пока очередь не обнулят.
                    self.queue_ahead -= size
                    if self.queue_ahead <= 0.0:
                        filled_entry = True
                        # taker только если мы ИЗНАЧАЛЬНО ставили агрессивный ордер
                        entry_is_taker = self.entry_order_is_aggressive

            elif self.side == -1:
                # Шорт: лимитка на продажу по order_price
                if price > self.order_price + self.PRICE_EPS:
                    # цена ещё НЕ дошла (мы выше рынка)
                    pass
                else:
                    # price <= order_price:
                    self.queue_ahead -= size
                    if self.queue_ahead <= 0.0:
                        filled_entry = True
                        entry_is_taker = self.entry_order_is_aggressive

            if filled_entry:
                logging.info("Entry filled at %.1f side=%d", self.order_price, self.side)
                self.state = "in_position"
                self.entry_price = self.order_price
                self.entry_ts = ts
                self.entry_time = cur_time

                if self.side == +1:
                    self.tp_plain = self.entry_price + cfg.tp_ticks * cfg.tick_size
                else:
                    self.tp_plain = self.entry_price - cfg.tp_ticks * cfg.tick_size

                # Лучшая цена в сторону позиции
                self.favorable_price = self.entry_price

                # Классификация ликвидности входа
                is_taker = entry_is_taker or self.entry_order_is_aggressive
                self.entry_liquidity = "taker" if is_taker else "maker"
                if self.entry_liquidity == "taker":
                    self.n_entry_taker += 1

                self.n_filled += 1
                self.order_price = None
                self.order_ts = None
                self.queue_ahead = 0.0
                self.entry_order_ts = None
                self.entry_order_is_aggressive = False

                # сбрасываем выходные ордера
                self.exit_order_price = None
                self.exit_order_ts = None
                self.exit_queue_ahead = 0.0
                self.exit_reason_target = None
                self.exit_order_is_aggressive = False

        # ---- fill выходной лимитки (cluster_exit и adverse_exit, одна модель очереди) ----
        if (
            self.state == "in_position"
            and self.entry_price is not None
            and self.exit_order_price is not None
            and self.side != 0
        ):
            filled_exit = False
            exit_is_taker = False

            # Общая модель: есть лимитка по exit_order_price, за ней стоит exit_queue_ahead объёма.
            if self.side == +1:
                # Закрываем long: продаём лимиткой
                if price < self.exit_order_price - self.PRICE_EPS:
                    # цена ещё не дошла до нашего уровня
                    pass
                else:
                    # price >= exit_order_price:
                    # агрессивные покупатели бьют в наш уровень (или перепрыгивают через него),
                    # мы сидим пассивно в очереди.
                    self.exit_queue_ahead -= size
                    if self.exit_queue_ahead <= 0.0:
                        filled_exit = True
                        # taker только если ордер ИЗНАЧАЛЬНО был агрессивным
                        exit_is_taker = self.exit_order_is_aggressive
            else:
                # Закрываем short: покупаем лимиткой
                if price > self.exit_order_price + self.PRICE_EPS:
                    # цена ещё не дошла сверху
                    pass
                else:
                    # price <= exit_order_price
                    self.exit_queue_ahead -= size
                    if self.exit_queue_ahead <= 0.0:
                        filled_exit = True
                        exit_is_taker = self.exit_order_is_aggressive

            if filled_exit:
                # Ликвидность выхода
                self.exit_liquidity = "taker" if exit_is_taker else "maker"
                if self.exit_liquidity == "taker":
                    self.n_exit_taker += 1

                self._close_position(ts, cur_time)

    # ----- PnL & logging -----

    def _compute_pnl_and_log(self, exit_price: float, exit_time: pd.Timestamp, reason: str):
        assert self.entry_price is not None
        assert self.entry_ts is not None
        assert self.entry_time is not None

        side = self.side
        cfg = self.cfg

        pnl_px = side * (exit_price - self.entry_price) * cfg.order_size

        # Ликвидность по ногам
        entry_liq = self.entry_liquidity or "maker"
        exit_liq = self.exit_liquidity or "maker"

        entry_fee_rate = cfg.fee_rate_taker if entry_liq == "taker" else cfg.fee_rate_maker
        exit_fee_rate = cfg.fee_rate_taker if exit_liq == "taker" else cfg.fee_rate_maker

        entry_fee = entry_fee_rate * self.entry_price * cfg.order_size + cfg.broker_fee_abs
        exit_fee = exit_fee_rate * exit_price * cfg.order_size + cfg.broker_fee_abs
        total_fee = entry_fee + exit_fee

        notional_entry = self.entry_price * cfg.order_size
        gross_ret = pnl_px / notional_entry
        net_ret = (pnl_px - total_fee) / notional_entry

        gross_ticks = side * (exit_price - self.entry_price) / cfg.tick_size
        rel_tick = cfg.tick_size / self.entry_price
        net_ticks = net_ret / rel_tick

        waited_ms = (
            int(self.entry_ts - self.entry_order_ts)
            if (self.entry_ts is not None and self.entry_order_ts is not None)
            else 0
        )
        hold_ms = int(exit_time.value // 1_000_000 - self.entry_time.value // 1_000_000)

        exit_from_cluster = reason == "cluster_exit"
        direction = "long" if side == +1 else "short"

        tr = TradeResult(
            entry_time=self.entry_time,
            exit_time=exit_time,
            direction=direction,
            side=side,
            entry_price=float(self.entry_price),
            exit_price=float(exit_price),
            reason=reason,
            gross_ticks=float(gross_ticks),
            net_ticks=float(net_ticks),
            gross_ret=float(gross_ret),
            net_ret=float(net_ret),
            waited_ms_for_fill=int(waited_ms),
            entry_fee=float(entry_fee),
            exit_fee=float(exit_fee),
            total_fee=float(total_fee),
            entry_liquidity=entry_liq,
            exit_liquidity=exit_liq,
            tp_plain=float(self.tp_plain) if self.tp_plain is not None else float("nan"),
            exit_from_cluster=exit_from_cluster,
            hold_ms=int(hold_ms),
        )
        self.trades.append(tr)

        # CSV сразу
        self._csv_writer.writerow(
            [
                tr.entry_time.isoformat(),
                tr.exit_time.isoformat(),
                tr.direction,
                tr.side,
                f"{tr.entry_price:.4f}",
                f"{tr.exit_price:.4f}",
                f"{tr.gross_ticks:.4f}",
                f"{tr.net_ticks:.4f}",
                f"{tr.gross_ret:.8f}",
                f"{tr.net_ret:.8f}",
                tr.reason,
                tr.entry_liquidity,
                tr.exit_liquidity,
                f"{tr.tp_plain:.4f}",
                "1" if tr.exit_from_cluster else "0",
                f"{tr.entry_fee:.4f}",
                f"{tr.exit_fee:.4f}",
                f"{tr.total_fee:.4f}",
                tr.waited_ms_for_fill,
                tr.hold_ms,
            ]
        )
        self._csv_file.flush()

        logging.info(
            "Closed %s trade: entry=%.1f exit=%.1f gross_ticks=%.2f net_ticks=%.2f reason=%s entry_liq=%s exit_liq=%s",
            direction,
            tr.entry_price,
            tr.exit_price,
            tr.gross_ticks,
            tr.net_ticks,
            reason,
            tr.entry_liquidity,
            tr.exit_liquidity,
        )

    def _reset_after_close(self):
        self.state = "flat"
        self.side = 0
        self.order_price = None
        self.order_ts = None
        self.queue_ahead = 0.0
        self.entry_price = None
        self.entry_ts = None
        self.entry_time = None
        self.entry_order_ts = None
        self.tp_plain = None
        self.exit_order_price = None
        self.exit_order_ts = None
        self.exit_queue_ahead = 0.0
        self.exit_reason_target = None
        self.favorable_price = None
        self.entry_liquidity = None
        self.exit_liquidity = None
        self.entry_order_is_aggressive = False
        self.exit_order_is_aggressive = False

    def _close_position(self, ts: int, cur_time: pd.Timestamp):
        if self.entry_price is None or self.entry_ts is None or self.entry_time is None:
            return
        exit_price = self.exit_order_price if self.exit_order_price is not None else self.entry_price
        exit_time = cur_time
        reason = self.exit_reason_target if self.exit_reason_target is not None else "limit_exit"

        self._compute_pnl_and_log(exit_price, exit_time, reason)
        self._reset_after_close()

    def force_flatten(self):
        """
        Форсированное закрытие при завершении работы.
        """
        if self.state == "in_position" and self.entry_price is not None:
            if self.last_trade_price is not None:
                exit_price = self.last_trade_price
            elif self.last_mid is not None:
                exit_price = self.last_mid
            else:
                exit_price = self.entry_price

            exit_time = self.last_time if self.last_time is not None else pd.Timestamp.utcnow()
            self._compute_pnl_and_log(exit_price, exit_time, reason="eod_force")
            self._reset_after_close()

    def print_stats(self):
        n = len(self.trades)
        if n == 0:
            logging.info("No trades yet")
            return
        wins = sum(1 for t in self.trades if t.net_ticks > 0)
        winrate = wins / n
        total_net_ticks = sum(t.net_ticks for t in self.trades)

        logging.info(
            "Trades=%d, winrate=%.2f, total_net_ticks=%.2f",
            n,
            winrate,
            total_net_ticks,
        )

        entry_taker_share = self.n_entry_taker / n
        exit_taker_share = self.n_exit_taker / n

        logging.info(
            "Entry taker share=%.2f (taker=%d), Exit taker share=%.2f (taker=%d)",
            entry_taker_share,
            self.n_entry_taker,
            exit_taker_share,
            self.n_exit_taker,
        )

    def close(self):
        self.force_flatten()
        self.print_stats()
        self._csv_file.close()


# ========= WS CLIENT =========

BYBIT_PUBLIC_LINEAR_WS = "wss://stream.bybit.com/v5/public/linear"


async def bybit_ws_loop(cfg: StrategyConfig, engine: ScalpingV10OfflineLiveEngine):
    # импорт тут, чтобы модуль можно было импортировать без websockets
    import websockets

    url = BYBIT_PUBLIC_LINEAR_WS
    topics = [
        f"orderbook.{cfg.depth}.{cfg.symbol}",
        f"publicTrade.{cfg.symbol}",
    ]

    backoff = [3, 5, 10, 30]

    while True:
        try:
            logging.info("Connecting to %s", url)
            async with websockets.connect(url, ping_interval=20, ping_timeout=20) as ws:
                sub_msg = {"op": "subscribe", "args": topics}
                await ws.send(json.dumps(sub_msg))
                logging.info("Subscribed to %s", topics)

                async for raw in ws:
                    try:
                        msg = json.loads(raw)
                    except json.JSONDecodeError:
                        continue

                    topic = msg.get("topic")
                    if not topic:
                        continue

                    if topic.startswith("orderbook."):
                        handle_orderbook_message(msg, engine, cfg)
                    elif topic.startswith("publicTrade."):
                        handle_trades_message(msg, engine)

        except Exception as e:
            logging.exception("WebSocket error: %s", e)
            delay = backoff[0] if backoff else 60
            if backoff:
                backoff = backoff[1:]
            logging.info("Reconnecting in %s seconds...", delay)
            await asyncio.sleep(delay)


def handle_orderbook_message(msg: dict, engine: ScalpingV10OfflineLiveEngine, cfg: StrategyConfig):
    typ = msg.get("type")  # "snapshot" / "delta"
    ts = int(msg.get("ts", 0))

    data = msg.get("data")
    # Bybit v5: data обычно список с одним объектом
    if isinstance(data, list):
        if not data:
            return
        data = data[0]
    if not isinstance(data, dict):
        return

    # инициализируем стакан один раз
    if not hasattr(engine, "_orderbook"):
        engine._orderbook = OrderbookL2(depth=cfg.depth)

    ob: OrderbookL2 = engine._orderbook  # type: ignore[attr-defined]

    if typ == "snapshot":
        ob.apply_snapshot(data)
    else:
        ob.apply_delta(data)

    max_levels = max(cfg.max_cluster_depth_entry, cfg.max_cluster_depth_exit, 1)
    levels_b, levels_a = ob.top_levels(max_levels)
    if not levels_b or not levels_a:
        return

    (bbp, bbv) = levels_b[0]
    (bap, bav) = levels_a[0]

    # находим максимальную стенку в первых N уровнях (общий N для entry/exit)
    max_b = min(max_levels, len(levels_b))
    cbp, cbv, cbd = None, 0.0, None
    for lvl in range(max_b):
        p, v = levels_b[lvl]
        if v > cbv:
            cbp, cbv, cbd = p, v, lvl

    max_a = min(max_levels, len(levels_a))
    cap, cav, cad = None, 0.0, None
    for lvl in range(max_a):
        p, v = levels_a[lvl]
        if v > cav:
            cap, cav, cad = p, v, lvl

    engine.on_orderbook(
        ts=ts,
        bbp=float(bbp),
        bbv=float(bbv),
        bap=float(bap),
        bav=float(bav),
        cbp=float(cbp) if cbp is not None else None,
        cbv=float(cbv),
        cbd=int(cbd) if cbd is not None else None,
        cap=float(cap) if cap is not None else None,
        cav=float(cav),
        cad=int(cad) if cad is not None else None,
    )


def handle_trades_message(msg: dict, engine: ScalpingV10OfflineLiveEngine):
    ts_snapshot = int(msg.get("ts", 0))
    data = msg.get("data", [])
    if not isinstance(data, list):
        return

    for tr in data:
        try:
            ts = int(tr.get("T", ts_snapshot))
            price = float(tr["p"])
            size = float(tr["v"])
        except (KeyError, TypeError, ValueError):
            continue
        engine.on_trade(ts=ts, price=price, size=size)


# ========= ENTRYPOINT =========

def main():
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(levelname)s] %(message)s",
        handlers=[logging.StreamHandler(sys.stdout)],
    )

    cfg = StrategyConfig()
    engine = ScalpingV10OfflineLiveEngine(cfg)

    loop = asyncio.get_event_loop()
    stop_event = asyncio.Event()

    def _signal_handler():
        logging.info("Signal received, shutting down...")
        stop_event.set()

    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, _signal_handler)
        except NotImplementedError:
            # Windows
            signal.signal(sig, lambda s, f: _signal_handler())

    async def runner():
        ws_task = asyncio.create_task(bybit_ws_loop(cfg, engine))
        await stop_event.wait()
        ws_task.cancel()
        try:
            await ws_task
        except asyncio.CancelledError:
            pass

    try:
        loop.run_until_complete(runner())
    finally:
        engine.close()
        loop.stop()
        loop.close()


if __name__ == "__main__":
    main()
