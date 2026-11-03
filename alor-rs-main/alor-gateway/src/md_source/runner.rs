use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use chrono::{Timelike, Utc};
use serde_json::json;

use super::{
    VERSION,
    config::{SourceConfig, redis_client},
    digest, local,
    storage::{Record, Store},
};
use crate::{
    auth::TokenProvider,
    models::{BarEvent, DataOrigin},
    router::parse_bars,
    terminal_bar::{Clock, FeedEvidence, Observation, Release, TerminalFinalizer},
    ws_hub::{ConnEvent, WsEvent, WsHub},
};

struct Received {
    bar: BarEvent,
    received: i64,
    generation: u64,
}

struct ReadinessDeadline {
    unready_since: Option<Duration>,
}
impl ReadinessDeadline {
    fn initial() -> Self {
        Self {
            unready_since: Some(Duration::ZERO),
        }
    }
    fn invalidate(&mut self, elapsed: Duration) {
        // Flapping attempts share one outage budget, not a fresh budget each time.
        self.unready_since.get_or_insert(elapsed);
    }
    fn ready(&mut self) {
        self.unready_since = None;
    }
    fn check(&self, elapsed: Duration) -> Result<()> {
        if let Some(since) = self.unready_since {
            ensure!(
                elapsed
                    .checked_sub(since)
                    .is_some_and(|age| age < Duration::from_secs(600)),
                "WS history readiness deadline"
            );
        }
        Ok(())
    }
}
struct Runner {
    cfg: SourceConfig,
    store: Store,
    core: TerminalFinalizer,
    conn: redis::aio::MultiplexedConnection,
    started: Instant,
    generation: u64,
    connected: bool,
    acked: bool,
    history_complete: bool,
    readiness_deadline: ReadinessDeadline,
    subscribed_at: Option<i64>,
    last_rx: i64,
    sequence: u64,
    history: BTreeMap<i64, Received>,
    current: Option<Received>,
}

impl Runner {
    fn clock(&self) -> Clock {
        Clock {
            utc: Utc::now().timestamp(),
            monotonic_ms: self.started.elapsed().as_millis() as u64,
        }
    }
    fn feed(&mut self) -> Result<()> {
        if self.generation > 0 {
            self.core.update_feed(
                FeedEvidence {
                    generation: self.generation,
                    connected: self.connected,
                    subscribed: self.acked,
                    gap_resolved: self.history_complete,
                    last_rx_utc: self.last_rx,
                },
                self.clock(),
            )?;
        }
        Ok(())
    }
    fn health(&self, status: &str, drained: bool) -> Result<()> {
        self.store.health(&json!({"version":VERSION,"status":status,"heartbeat_utc":Utc::now().timestamp(),
            "last_ws_rx_utc":self.last_rx,"generation":self.generation,
            "history_complete":self.history_complete,"connected":self.connected,"subscribed":self.acked,
            "queue_drained":drained,"published_rows":self.store.records.len(),
            "last_published_bar_ts":self.store.records.last_key_value().map(|(ts,_)|*ts),
            "audit_rows":self.store.audit_rows,"review_dates":self.store.review_dates,
            "readiness_wait_seconds":self.readiness_deadline.unready_since.map(|since|self.started.elapsed().saturating_sub(since).as_secs()),
            "order_emission_allowed":false,"config_sha256":digest(&serde_json::to_value(&self.cfg)?)?}))
    }
    fn validate_raw(&self, bar: &BarEvent, received: i64) -> Result<()> {
        ensure!(
            bar.symbol == self.cfg.symbol
                && bar.close_time_utc > 0
                && bar.close_time_utc % 600 == 0
                && bar.close_time_utc <= received
                && received <= self.clock().utc,
            "invalid WS symbol/clock/grid"
        );
        ensure!(
            [bar.o, bar.h, bar.l, bar.c, bar.v]
                .iter()
                .all(|n| n.is_finite())
                && bar.l > 0.
                && bar.l <= bar.o.min(bar.c)
                && bar.h >= bar.o.max(bar.c)
                && bar.v >= 0.,
            "invalid WS OHLCV"
        );
        Ok(())
    }
    async fn release(&mut self, release: Release) -> Result<()> {
        let mut bar = release.bar.clone();
        let day = local(bar.close_time_utc)?.date();
        if day < self.cfg.first_live_session || self.cfg.context_day(day) {
            bar.origin = DataOrigin::History;
        }
        let proof = json!({"kind":"as_observed_ws","release_id":release.id,"release":release.proof,
                         "ws_sequence_at_cutoff":self.sequence});
        let row = Record::new(
            &self.cfg,
            &bar,
            proof,
            Some(release.clone()),
            self.clock().utc,
        )?;
        self.store.append(&self.cfg, row, &mut self.conn).await?;
        self.core.commit(&release, self.clock())?;
        println!(
            "{}",
            json!({"event":"ws_closed_bar_published","symbol":bar.symbol,
            "label":bar.close_time_utc,"origin":bar.origin,"reason":release.proof.reason,
            "rows":self.store.records.len(),"order_emission_allowed":false})
        );
        Ok(())
    }
    async fn live(&mut self, mut input: Received) -> Result<()> {
        input.bar.origin = DataOrigin::Live;
        match self
            .core
            .observe_received(input.bar.clone(), input.received, self.clock())?
        {
            Observation::Ready(release) => self.release(release).await?,
            Observation::Revision { .. } => {
                if let Some(old) = self.store.records.get(&input.bar.close_time_utc) {
                    self.store
                        .audit(&old.bar()?, &input.bar, input.received, input.generation)?;
                }
            }
            Observation::Buffered | Observation::Duplicate => {}
        }
        Ok(())
    }
    async fn raw(&mut self, value: serde_json::Value, generation: u64) -> Result<()> {
        ensure!(
            serde_json::to_vec(&value)?.len() <= 2 * 1024 * 1024,
            "WS frame budget"
        );
        let subscribed = self
            .subscribed_at
            .context("WS data before subscription provenance")?;
        let cutoff = Some(subscribed - 1200);
        for bar in parse_bars(&value, cutoff, DataOrigin::History) {
            self.validate_raw(&bar, self.last_rx)?;
            if !self.cfg.eligible(bar.close_time_utc)? {
                continue;
            }
            if let Some(old) = self.store.records.get(&bar.close_time_utc) {
                self.store
                    .audit(&old.bar()?, &bar, self.last_rx, generation)?;
                continue;
            }
            let closed_at_subscribe = bar.close_time_utc + 600 <= subscribed;
            if closed_at_subscribe
                || local(bar.close_time_utc)?.date() < self.cfg.terminal.first_live_session
            {
                ensure!(
                    local(bar.close_time_utc)?.date() < self.cfg.first_live_session,
                    "unobserved active history cannot become timely Live"
                );
                ensure!(
                    self.store
                        .records
                        .last_key_value()
                        .is_none_or(|(ts, _)| *ts < bar.close_time_utc),
                    "late unknown history would change immutable source prefix"
                );
                self.history.insert(
                    bar.close_time_utc,
                    Received {
                        bar,
                        received: self.last_rx,
                        generation,
                    },
                );
                ensure!(
                    self.history.len() <= super::MAX_ROWS,
                    "history staging budget"
                );
            } else {
                let received = Received {
                    bar,
                    received: self.last_rx,
                    generation,
                };
                if self.prior_complete()? && self.connected && self.acked {
                    self.drain_history().await?;
                    self.live(received).await?;
                } else {
                    ensure!(self.current.as_ref().is_none_or(|old|old.bar.close_time_utc==received.bar.close_time_utc),
                            "current WS observation advanced while history was unready");
                    self.current = Some(received);
                }
            }
        }
        Ok(())
    }
    async fn event(&mut self, event: WsEvent) -> Result<()> {
        self.sequence += 1;
        match event {
            WsEvent::Conn { event, generation } => {
                if generation < self.generation {
                    return Ok(());
                }
                self.generation = generation;
                self.connected = matches!(event, ConnEvent::Connected);
                self.acked = false;
                self.history_complete = false;
                self.readiness_deadline.invalidate(self.started.elapsed());
                self.current = None;
                self.history.clear();
                self.subscribed_at = None;
                self.feed()?;
                self.health("reconnecting", false)?;
            }
            WsEvent::Subscribed {
                wallclock_ts,
                generation,
                ..
            } if generation == self.generation => {
                self.subscribed_at = Some(wallclock_ts);
                self.acked = false;
                self.feed()?;
            }
            WsEvent::SubscriptionAck {
                subscription_type,
                generation,
            } if generation == self.generation => {
                ensure!(
                    subscription_type == "bars",
                    "forbidden account subscription in MD-only role"
                );
                self.acked = true;
                self.feed()?;
            }
            WsEvent::WsRx { ts, generation } if generation == self.generation => {
                ensure!(
                    ts >= self.last_rx && ts <= self.clock().utc,
                    "WS receipt clock changed"
                );
                self.last_rx = ts;
                self.feed()?;
            }
            WsEvent::Raw { value, generation } if generation == self.generation => {
                self.raw(value, generation).await?
            }
            _ => {}
        }
        Ok(())
    }
    fn prior_complete(&self) -> Result<bool> {
        let today = local(self.clock().utc)?.date();
        let mut day = self.cfg.bootstrap_start;
        let mut regular = 0;
        while day < today {
            if !self.cfg.context_day(day) {
                let from = day.and_hms_opt(7, 0, 0).unwrap().and_utc().timestamp() - 10800;
                for slot in 0..101 {
                    let ts = from + slot * 600;
                    if !self.history.contains_key(&ts) && !self.store.records.contains_key(&ts) {
                        return Ok(false);
                    }
                }
                regular += 1;
            }
            day = day.succ_opt().context("calendar overflow")?;
        }
        Ok(regular >= 2)
    }
    async fn drain_history(&mut self) -> Result<()> {
        if !self.connected || !self.acked {
            return Ok(());
        }
        if !self.history_complete && !self.prior_complete()? {
            return Ok(());
        }
        let inputs = std::mem::take(&mut self.history);
        for (_, mut input) in inputs {
            input.bar.origin = DataOrigin::History;
            let proof = json!({"kind":"historical_ws_import","received_at_utc":input.received,
                "generation":input.generation,"imported_at_utc":self.clock().utc});
            let row = Record::new(&self.cfg, &input.bar, proof, None, self.clock().utc)?;
            self.store.append(&self.cfg, row, &mut self.conn).await?;
        }
        self.history_complete = true;
        self.readiness_deadline.ready();
        self.feed()?;
        if let Some(current) = self.current.take() {
            self.live(current).await?;
        }
        Ok(())
    }
}

pub async fn run(config: SourceConfig, root: &Path, mode: &str, fixture: bool) -> Result<()> {
    let started = Instant::now();
    let now = Utc::now().timestamp();
    config.validate(now)?;
    let gateway = config.gateway(fixture)?;
    if mode == "create" {
        ensure!(
            now < config.model_cutoff(),
            "create must precede model activation cutoff"
        );
    }
    let store = Store::open(root, &config, mode, now)?;
    let result = async {
        let url = std::env::var("SHADOW_SOURCE_REDIS_URL")
            .context("dedicated source Redis credential required")?;
        let client = redis_client(&url, fixture)?;
        let mut conn = tokio::time::timeout(
            Duration::from_secs(5),
            client.get_multiplexed_async_connection(),
        )
        .await??;
        store.verify_prefix(&config, &mut conn).await?;
        if mode == "resume" && now >= config.model_cutoff() {
            ensure!(
                local(now)?.hour() < 7,
                "active source cannot resume intraday"
            );
            let mut previous = local(now)?
                .date()
                .pred_opt()
                .context("calendar underflow")?;
            for _ in 0..7 {
                if let Some(label) =
                    config.terminal.symbols[&config.symbol].terminal_label(previous)
                {
                    ensure!(
                        store.records.contains_key(&label),
                        "resume requires prior terminal evidence"
                    );
                    break;
                }
                previous = previous.pred_opt().context("calendar underflow")?;
            }
        }
        Ok::<_, anyhow::Error>(conn)
    }
    .await;
    let conn = match result {
        Ok(conn) => conn,
        Err(error) => {
            store.fail("startup", &format!("{error:#}"), now);
            return Err(error);
        }
    };
    let mut core = TerminalFinalizer::new(config.terminal.clone())?;
    let recent: Vec<_> = store
        .records
        .values()
        .rev()
        .filter_map(|r| r.release.clone())
        .take(2)
        .collect();
    core.restore_committed(
        &recent.into_iter().rev().collect::<Vec<_>>(),
        Clock {
            utc: Utc::now().timestamp(),
            monotonic_ms: started.elapsed().as_millis() as u64,
        },
    )?;
    let token = if fixture {
        TokenProvider::new_with_token(
            &gateway.oauth_url,
            &gateway.refresh_token,
            std::env::var("SHADOW_FIXTURE_ACCESS_TOKEN")
                .context("explicit fixture access token required")?,
        )
    } else {
        TokenProvider::new(&gateway.oauth_url, &gateway.refresh_token)
    };
    let (handle, mut events) = WsHub::start_market_data_only(gateway, token);
    let mut runner = Runner {
        cfg: config,
        store,
        core,
        conn,
        started,
        generation: 0,
        connected: false,
        acked: false,
        history_complete: false,
        readiness_deadline: ReadinessDeadline::initial(),
        subscribed_at: None,
        last_rx: now,
        sequence: 0,
        history: BTreeMap::new(),
        current: None,
    };
    runner.health("starting", false)?;
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let result=async {
        loop {
            tokio::select! {
                _=signal.recv()=>break,
                _=interrupt.recv()=>break,
                event=events.recv()=>runner.event(event.context("WS event source ended")?).await?,
                _=interval.tick()=>{
                    let mut drained=0;
                    while let Ok(event)=events.try_recv() {
                        runner.event(event).await?;drained+=1;
                        ensure!(drained<=2048,"WS event drain budget exceeded");
                    }
                    let clock=runner.clock();
                    runner.cfg.validate(clock.utc)?;
                    if events.is_empty() {
                        runner.drain_history().await?;
                        if runner.history_complete {
                            for release in runner.core.tick(runner.clock(),true)? {runner.release(release).await?;}
                        }
                    }
                    runner.readiness_deadline.check(runner.started.elapsed())?;
                    runner.health(if runner.history_complete&&runner.connected&&runner.acked{"ready"}else{"reconnecting"},events.is_empty())?;
                }
            }
        }
        ensure!(runner.core.pending_releases().is_empty(),"pending publication at stop");
        Ok::<_,anyhow::Error>(())
    }.await;
    handle.shutdown().await;
    let _ = tokio::time::timeout(Duration::from_secs(4), async {
        while events.recv().await.is_some() {}
    })
    .await;
    match result {
        Ok(()) => {
            runner.health("stopped", true)?;
            runner.store.stopped(runner.clock().utc)?;
            Ok(())
        }
        Err(error) => {
            runner
                .store
                .fail("source_fault", &format!("{error:#}"), runner.clock().utc);
            let _ = runner.health("failed", false);
            Err(error)
        }
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    #[test]
    fn mature_source_gets_a_new_bounded_recovery_period_not_process_age() {
        let mut deadline = ReadinessDeadline::initial();
        assert!(deadline.check(Duration::from_secs(599)).is_ok());
        assert!(deadline.check(Duration::from_secs(600)).is_err());
        deadline.ready();
        let mature = Duration::from_secs(10 * 3600);
        deadline.check(mature).unwrap();
        deadline.invalidate(mature);
        deadline.check(mature + Duration::from_secs(1)).unwrap();
        deadline.check(mature + Duration::from_secs(599)).unwrap();
        assert!(deadline.check(mature + Duration::from_secs(600)).is_err());
        deadline.ready();
        deadline.check(mature + Duration::from_secs(3600)).unwrap();
    }

    #[test]
    fn reconnect_flapping_cannot_postpone_readiness_deadline() {
        let mut deadline = ReadinessDeadline::initial();
        deadline.invalidate(Duration::from_secs(599));
        assert!(deadline.check(Duration::from_secs(600)).is_err());
        deadline.ready();
        deadline.invalidate(Duration::from_secs(3600));
        deadline.invalidate(Duration::from_secs(4199));
        assert!(deadline.check(Duration::from_secs(4200)).is_err());
    }
}
