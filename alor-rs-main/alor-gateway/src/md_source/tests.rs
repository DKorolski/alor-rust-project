use super::*;
use crate::{
    models::{BarEvent, DataOrigin},
    terminal_bar::{Clock, FeedEvidence, TerminalFinalizer},
};
use serde_json::json;

fn ts(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
}
fn cfg() -> config::SourceConfig {
    serde_json::from_value(json!({"symbol":"IMOEXF","stream":"shadow.source.closed10m.IMOEXF.test",
        "bootstrap_start":"2026-10-29","first_live_session":"2026-11-03","excluded_dates":[],
        "gateway_config":"/fixture/gateway.toml","gateway_config_sha256":"a".repeat(64),
        "terminal":{"enabled":true,"first_live_session":"2026-11-02","grace_seconds":10,
            "max_lateness_seconds":90,"max_heartbeat_age_seconds":45,"max_clock_step_seconds":5,
            "symbols":{"IMOEXF":{"weekday_end":"23:50:00","weekend_end":"19:00:00","overrides":{}}}}})).unwrap()
}
fn bar() -> BarEvent {
    BarEvent {
        symbol: "IMOEXF".into(),
        close_time_utc: ts("2026-11-02T23:40:00+03:00"),
        o: 100.,
        h: 106.,
        l: 98.,
        c: 103.,
        v: 589.,
        origin: DataOrigin::Live,
    }
}

#[test]
fn source_config_rejects_nonisolated_identity_dates_and_public_redis() {
    let now = ts("2026-11-02T21:00:00+03:00");
    let config = cfg();
    config.validate(now).unwrap();
    for value in [
        "cmd.orders.IMOEXF",
        "md.bars.IMOEXF.10m",
        "shadow.source.closed10m.USDRUBF.test",
    ] {
        let mut invalid = config.clone();
        invalid.stream = value.into();
        assert!(invalid.validate(now).is_err());
    }
    let mut invalid = config.clone();
    invalid.terminal.enabled = false;
    assert!(invalid.validate(now).is_err());
    invalid = config.clone();
    invalid.first_live_session = chrono::NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
    assert!(invalid.validate(now).is_err());
    assert!(config::redis_client("redis://source:fixture@8.8.8.8/0", false).is_err());
    assert!(config::redis_client("redis://127.0.0.1:1/0", true).is_err());
    assert!(config::redis_client("redis://source:fixture@127.0.0.1:1/0", true).is_ok());
}

#[test]
fn terminal_release_survives_canonical_journal_roundtrip_and_audit() {
    let config = cfg();
    let mut core = TerminalFinalizer::new(config.terminal.clone()).unwrap();
    let receive = ts("2026-11-02T23:49:59+03:00");
    let cutoff = ts("2026-11-02T23:50:10+03:00");
    let clock = Clock {
        utc: cutoff,
        monotonic_ms: 0,
    };
    core.update_feed(
        FeedEvidence {
            generation: 1,
            connected: true,
            subscribed: true,
            gap_resolved: true,
            last_rx_utc: cutoff,
        },
        clock,
    )
    .unwrap();
    core.observe_received(bar(), receive, clock).unwrap();
    let release = core.tick(clock, true).unwrap().pop().unwrap();
    let mut canonical = release.bar.clone();
    canonical.origin = DataOrigin::History;
    let row = storage::Record::new(
        &config,
        &canonical,
        json!({"kind":"as_observed_ws",
        "release_id":release.id,"release":release.proof}),
        Some(release),
        cutoff,
    )
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("source");
    let store = storage::Store::open(&root, &config, "create", cutoff).unwrap();
    storage::publish_new(
        &root.join("committed.jsonl"),
        &json!({"previous":"genesis",
        "record":row,"sha256":digest(&("genesis",&row)).unwrap()}),
    )
    .unwrap();
    store.stopped(cutoff).unwrap();
    drop(store);
    let mut restored = storage::Store::open(&root, &config, "resume", cutoff).unwrap();
    assert_eq!(restored.records.len(), 1);
    let mut revised = canonical.clone();
    revised.v += 1.;
    restored.audit(&canonical, &revised, cutoff, 2).unwrap();
    restored.audit(&canonical, &revised, cutoff, 2).unwrap();
    assert_eq!(restored.audit_rows, 1);
    assert!(restored.review_dates.is_empty());
    revised.c += 1.;
    restored.audit(&canonical, &revised, cutoff, 2).unwrap();
    assert_eq!(restored.review_dates.len(), 1);
    let mut unrelated = canonical.clone();
    unrelated.v += 10.;
    assert!(restored.audit(&unrelated, &revised, cutoff, 2).is_err());
    restored.stopped(cutoff).unwrap();
    drop(restored);
    let restored = storage::Store::open(&root, &config, "resume", cutoff).unwrap();
    assert_eq!(restored.audit_rows, 2);
    assert_eq!(restored.review_dates.len(), 1);
}

#[test]
fn source_resume_rejects_incomplete_attempt_and_pending_publication() {
    let config = cfg();
    let now = ts("2026-11-02T21:00:00+03:00");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("source");
    let store = storage::Store::open(&root, &config, "create", now).unwrap();
    assert!(storage::Store::open(&root, &config, "resume", now).is_err());
    drop(store);
    assert!(storage::Store::open(&root, &config, "resume", now).is_err());
    let root = temp.path().canonicalize().unwrap().join("pending");
    let store = storage::Store::open(&root, &config, "create", now).unwrap();
    store.stopped(now).unwrap();
    drop(store);
    storage::publish_new(&root.join("pending.json"), &json!({"unknown_ack":true})).unwrap();
    assert!(storage::Store::open(&root, &config, "resume", now).is_err());
}

#[test]
fn sparse_context_forward_is_not_certified_by_history_bootstrap() {
    let mut config = cfg();
    let calendar = config.terminal.symbols.remove("IMOEXF").unwrap();
    config.symbol = "USDRUBF".into();
    config.stream = "shadow.source.closed10m.USDRUBF.test".into();
    config
        .terminal
        .symbols
        .insert(config.symbol.clone(), calendar);
    let first = ts("2026-11-07T09:51:00+03:00");
    let later = ts("2026-11-07T10:20:00+03:00");
    config.validate(first).unwrap();
    let mut quote = bar();
    quote.symbol = config.symbol.clone();
    quote.close_time_utc = ts("2026-11-07T09:50:00+03:00");
    assert!(config.context_day(local(first).unwrap().date()));
    assert!(config.eligible(quote.close_time_utc).unwrap());
    let mut core = TerminalFinalizer::new(config.terminal.clone()).unwrap();
    let clock = Clock {
        utc: first,
        monotonic_ms: 0,
    };
    core.update_feed(
        FeedEvidence {
            generation: 1,
            connected: true,
            subscribed: true,
            gap_resolved: true,
            last_rx_utc: first,
        },
        clock,
    )
    .unwrap();
    core.observe_received(quote.clone(), first, clock).unwrap();
    let clock = Clock {
        utc: later,
        monotonic_ms: ((later - first) * 1000) as u64,
    };
    core.update_feed(
        FeedEvidence {
            generation: 1,
            connected: true,
            subscribed: true,
            gap_resolved: true,
            last_rx_utc: later,
        },
        clock,
    )
    .unwrap();
    quote.close_time_utc = later;
    // Diagnostic blocker: sparse context still uses the timely-Live path.
    let error = core.observe_received(quote, later, clock).unwrap_err();
    assert!(error.to_string().contains("outside publication deadline"));
    assert_eq!(core.last_committed_label("USDRUBF"), None);
}
