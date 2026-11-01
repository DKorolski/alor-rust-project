use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, VecDeque};

use chrono::{Duration, NaiveDate, NaiveDateTime};

use super::*;
use crate::ri_context::{ContextIdentity, ContextPolicy, ContextRoute, QuarantineReason};
use crate::strategy_host::DataOrigin;

thread_local! {
    static NOW: Cell<i64> = const { Cell::new(0) };
    static TICKS: RefCell<VecDeque<i64>> = const { RefCell::new(VecDeque::new()) };
}

fn clock() -> i64 {
    TICKS
        .with(|ticks| ticks.borrow_mut().pop_front())
        .unwrap_or_else(|| NOW.with(Cell::get))
}
fn set_clock(value: i64) {
    NOW.with(|now| now.set(value));
    TICKS.with(|ticks| ticks.borrow_mut().clear());
}
fn epoch(text: &str) -> i64 {
    (NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap() - Duration::hours(3))
        .and_utc()
        .timestamp()
}
fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}
fn identity() -> IngressIdentity {
    IngressIdentity {
        generation: "test-generation-1".into(),
        receipt_stream: "shadow.ri.receipts.test-generation-1".into(),
        source_stream: "md.bars.test.10m".into(),
        symbol: "RIZ6".into(),
    }
}
fn create() -> (tempfile::TempDir, RiIngressJournal) {
    set_clock(epoch("2026-10-09 06:00"));
    let dir = tempfile::tempdir().unwrap();
    let journal =
        RiIngressJournal::create_with_clock(&dir.path().join("generation"), identity(), clock)
            .unwrap();
    (dir, journal)
}
fn open(path: &Path) -> Result<RiIngressJournal> {
    RiIngressJournal::open_with_clock(path, &identity(), clock)
}
fn bar(label: i64, origin: DataOrigin, close: f64) -> String {
    serde_json::to_string(&BarEvent {
        symbol: "RIZ6".into(),
        close_time_utc: label,
        o: 90000.,
        h: 91000.,
        l: 89000.,
        close,
        v: 10.,
        origin,
    })
    .unwrap()
}
fn context(policy: ContextPolicy) -> RiContextStore {
    RiContextStore::new(ContextIdentity {
        profile_id: "paired_ri_context".into(),
        symbol: identity().symbol,
        numerical_config_sha256: "a".repeat(64),
        model_feed_policy: "micro_regular_v1".into(),
        receipt_stream: identity().receipt_stream,
        policy,
        model_open_hour: 7,
        excluded_dates: BTreeSet::new(),
    })
    .unwrap()
}
fn append_day(j: &mut RiIngressJournal, day: &str, last: &str) {
    let first = epoch(&format!("{day} 07:00"));
    let last = epoch(&format!("{day} {last}"));
    for ts in (first..=last).step_by(600) {
        set_clock(ts + 600);
        j.append(&ts.to_string(), &bar(ts, DataOrigin::Live, 90000.))
            .unwrap()
            .input
            .unwrap();
    }
}

#[test]
fn paired_contexts_replay_same_durable_input_and_restore_exact_frozen_sources() {
    let (dir, mut j) = create();
    append_day(&mut j, "2026-10-09", "23:40");
    append_day(&mut j, "2026-10-12", "23:30");
    set_clock(epoch("2026-10-13 03:00"));
    let last = j
        .append(
            "closing-gap",
            &bar(epoch("2026-10-12 23:40"), DataOrigin::HistoryGap, 90500.),
        )
        .unwrap()
        .input
        .unwrap();
    assert_eq!(last.bar.origin, DataOrigin::HistoryGap);
    set_clock(epoch("2026-10-13 07:10"));
    let barrier = j.seal_cutoff(epoch("2026-10-13 07:00")).unwrap();
    let mut snapshots = vec![];
    for (name, policy, expected_close) in [
        ("control", ContextPolicy::RuntimeAcceptedOnly, 90000.),
        (
            "candidate",
            ContextPolicy::RiConfirmedClosingContextV1,
            90500.,
        ),
    ] {
        let mut s = context(policy);
        j.replay_through(&mut s, barrier.last_sequence, clock())
            .unwrap();
        assert_eq!(s.ingress_sha256().unwrap(), barrier.ingress_sha256);
        let frozen = s
            .freeze_and_save_new(
                date("2026-10-13"),
                clock(),
                &barrier,
                &dir.path().join(format!("{name}.json")),
            )
            .unwrap()
            .clone();
        assert!(frozen.ready());
        assert_eq!(frozen.previous.as_ref().unwrap().close, expected_close);
        assert_eq!(
            s.state.journal.last().unwrap().route,
            if name == "candidate" {
                ContextRoute::ContextOnly
            } else {
                ContextRoute::Quarantined(QuarantineReason::RecoveryDisabled)
            }
        );
        let bytes = s.checkpoint_bytes().unwrap();
        snapshots.push((s.state.identity.clone(), bytes, frozen));
    }
    let root = j.root.clone();
    drop(j);
    set_clock(epoch("2026-10-13 08:00"));
    let mut j = open(&root).unwrap();
    assert_eq!(j.seal_cutoff(barrier.through_utc).unwrap(), barrier);
    for (identity, bytes, frozen) in snapshots {
        let mut s = RiContextStore::restore(&identity, &bytes, clock()).unwrap();
        j.replay_through(&mut s, barrier.last_sequence, clock())
            .unwrap();
        assert_eq!(s.frozen(date("2026-10-13")), Some(&frozen));
        assert!(!s
            .frozen_anchor_inputs(date("2026-10-13"))
            .unwrap()
            .is_empty());
    }
}

#[test]
fn newly_downloaded_history_never_acquires_an_old_receipt() {
    let (_dir, mut j) = create();
    set_clock(epoch("2026-10-13 07:05"));
    let receipt = j
        .append(
            "late-history",
            &bar(epoch("2026-10-12 23:40"), DataOrigin::HistoryGap, 90500.),
        )
        .unwrap()
        .input
        .unwrap();
    assert_eq!(receipt.received_at_utc, Some(clock()));
    let mut s = context(ContextPolicy::RiConfirmedClosingContextV1);
    j.replay_through(&mut s, 1, clock()).unwrap();
    assert_eq!(
        s.state.journal[0].route,
        ContextRoute::Quarantined(QuarantineReason::AtOrAfterCutoff)
    );
}

#[test]
fn payload_fsync_crossing_cutoff_is_late_even_if_receipt_was_before() {
    let (_dir, mut j) = create();
    let cutoff = epoch("2026-10-13 07:00");
    set_clock(cutoff + 1);
    TICKS.with(|ticks| *ticks.borrow_mut() = VecDeque::from([cutoff - 1, cutoff, cutoff + 1]));
    let input = j
        .append(
            "crossing",
            &bar(epoch("2026-10-12 23:40"), DataOrigin::HistoryGap, 90500.),
        )
        .unwrap()
        .input
        .unwrap();
    assert_eq!(input.received_at_utc, Some(cutoff - 1));
    assert_eq!(input.durable_at_utc, Some(cutoff));
    let mut s = context(ContextPolicy::RiConfirmedClosingContextV1);
    j.replay_through(&mut s, 1, clock()).unwrap();
    assert_eq!(
        s.state.journal[0].route,
        ContextRoute::Quarantined(QuarantineReason::AtOrAfterCutoff)
    );
}

#[test]
fn redelivery_after_restart_keeps_original_evidence_and_conflicts_fail() {
    let (_dir, mut j) = create();
    let payload = bar(epoch("2026-10-09 07:00"), DataOrigin::Live, 90000.);
    set_clock(epoch("2026-10-09 07:10"));
    let first = j.append("source-1", &payload).unwrap().input.unwrap();
    let root = j.root.clone();
    drop(j);
    set_clock(epoch("2026-10-12 09:00"));
    let mut j = open(&root).unwrap();
    let repeated = j.append("source-1", &payload).unwrap();
    assert!(repeated.redelivery);
    assert_eq!(repeated.input.unwrap(), first);
    assert_eq!(j.last_sequence(), 1);
    assert_eq!(j.last_source_id(), Some("source-1"));
    assert_eq!(j.usage().records, 1);
    assert!(!j.usage().writer_failed);
    assert!(j
        .append("source-1", &payload.replace("90000", "90100"))
        .is_err());
    assert_eq!(j.last_sequence(), 1);
}

#[test]
fn malformed_payload_is_retained_but_blocks_barrier_and_atomic_context_replay() {
    let (_dir, mut j) = create();
    set_clock(epoch("2026-10-09 07:10"));
    j.append(
        "valid",
        &bar(epoch("2026-10-09 07:00"), DataOrigin::Live, 90000.),
    )
    .unwrap();
    assert!(j
        .append("malformed", "{broken payload")
        .unwrap()
        .input
        .is_err());
    let raw: RawReceipt = read_checked(j.root.join("00000002.raw.json")).unwrap();
    assert_eq!(raw.raw_bar_json, "{broken payload");
    assert!(j.seal_cutoff(clock()).is_err());
    let mut s = context(ContextPolicy::RiConfirmedClosingContextV1);
    assert!(j.replay_through(&mut s, 2, clock()).is_err());
    assert_eq!(s.last_sequence(), 0);
    let root = j.root.clone();
    drop(j);
    let mut j = open(&root).unwrap();
    assert_eq!(j.last_sequence(), 2);
    assert!(j.seal_cutoff(clock()).is_err());
}

#[test]
fn rejects_two_writers_and_wrong_generation_without_reset() {
    let (_dir, j) = create();
    assert!(open(&j.root).is_err());
    assert!(RiIngressJournal::create_with_clock(&j.root, identity(), clock).is_err());
    let root = j.root.clone();
    drop(j);
    let mut wrong = identity();
    wrong.generation = "new-generation".into();
    assert!(RiIngressJournal::open_with_clock(&root, &wrong, clock).is_err());
    assert!(open(&root).is_ok());
}

#[test]
fn uncommitted_or_unknown_tail_fails_without_truncation() {
    for name in [
        "00000001.raw.json",
        ".ingress-unfinished.tmp",
        "00000002.commit.json",
    ] {
        let (_dir, j) = create();
        let root = j.root.clone();
        fs::write(root.join(name), b"partial").unwrap();
        drop(j);
        assert!(open(&root).is_err(), "{name}");
        assert_eq!(fs::read(root.join(name)).unwrap(), b"partial");
    }
}

#[test]
fn failed_publication_poisoned_writer_cannot_issue_barrier_or_continue() {
    let (_dir, mut j) = create();
    fs::write(j.root.join("00000001.commit.json"), b"existing").unwrap();
    assert!(j
        .append(
            "one",
            &bar(epoch("2026-10-09 07:00"), DataOrigin::Live, 90000.)
        )
        .is_err());
    assert_eq!(j.last_sequence(), 0);
    assert!(j.seal_cutoff(clock()).is_err());
    assert!(j.append("two", "{}").is_err());
    assert_eq!(
        fs::read(j.root.join("00000001.commit.json")).unwrap(),
        b"existing"
    );
}

#[test]
fn checksum_and_missing_middle_sequence_fail_restore() {
    for corrupt in [true, false] {
        let (_dir, mut j) = create();
        for i in 0..3 {
            j.append(
                &format!("id-{i}"),
                &bar(epoch("2026-10-09 07:00"), DataOrigin::History, 90000.),
            )
            .unwrap();
        }
        let root = j.root.clone();
        drop(j);
        if corrupt {
            let path = root.join("00000002.raw.json");
            let bytes = fs::read_to_string(&path).unwrap();
            fs::write(path, bytes.replace("id-1", "changed-id")).unwrap();
        } else {
            fs::remove_file(root.join("00000002.raw.json")).unwrap();
            fs::remove_file(root.join("00000002.commit.json")).unwrap();
        }
        assert!(open(&root).is_err());
    }
}

#[test]
fn cutoff_barrier_is_immutable_and_new_receipts_do_not_change_old_prefix() {
    let (_dir, mut j) = create();
    set_clock(epoch("2026-10-12 07:00"));
    let barrier = j.seal_cutoff(clock()).unwrap();
    assert_eq!(barrier.last_sequence, 0);
    set_clock(epoch("2026-10-12 07:05"));
    j.append(
        "late",
        &bar(epoch("2026-10-09 23:40"), DataOrigin::HistoryGap, 90500.),
    )
    .unwrap();
    assert_eq!(j.seal_cutoff(barrier.through_utc).unwrap(), barrier);
    let mut s = context(ContextPolicy::RiConfirmedClosingContextV1);
    j.replay_through(&mut s, barrier.last_sequence, clock())
        .unwrap();
    assert_eq!(s.last_sequence(), 0);
    j.replay_through(&mut s, 1, clock()).unwrap();
    assert_eq!(
        s.state.journal[0].route,
        ContextRoute::Quarantined(QuarantineReason::AtOrAfterCutoff)
    );
    assert!(j.replay_through(&mut s, 0, clock()).is_err());
    let root = j.root.clone();
    drop(j);
    assert_eq!(open(&root).unwrap().barriers[0].barrier, barrier);
}

#[test]
fn refuses_clock_rewind_future_cutoff_and_cutoff_before_collector_birth() {
    let (_dir, mut j) = create();
    let born = clock();
    assert!(j.seal_cutoff(born - 1).is_err());
    assert!(j.seal_cutoff(born + 1).is_err());
    set_clock(born + 10);
    j.seal_cutoff(clock()).unwrap();
    set_clock(born + 9);
    assert!(j.append("one", "{}").is_err());
    assert!(j.seal_cutoff(born + 10).is_err());
    let root = j.root.clone();
    drop(j);
    assert!(open(&root).is_err());
}

#[test]
fn clock_rewind_during_sync_is_not_published_to_context() {
    let (_dir, mut j) = create();
    let t = clock() + 100;
    set_clock(t);
    TICKS.with(|ticks| *ticks.borrow_mut() = VecDeque::from([t, t - 1]));
    assert!(j.append("one", "{}").is_err());
    assert!(j.poisoned);
    assert_eq!(j.last_sequence(), 0);
}

#[test]
fn divergent_context_prefix_and_identity_fail_without_mutation() {
    let (_dir, mut j) = create();
    set_clock(epoch("2026-10-09 07:10"));
    let mut input = j
        .append(
            "one",
            &bar(epoch("2026-10-09 07:00"), DataOrigin::Live, 90000.),
        )
        .unwrap()
        .input
        .unwrap();
    let mut s = context(ContextPolicy::RiConfirmedClosingContextV1);
    input.bar.close = 90500.;
    s.ingest(input, clock()).unwrap();
    let before = s.checkpoint_bytes().unwrap();
    assert!(j.replay_through(&mut s, 1, clock()).is_err());
    assert_eq!(s.checkpoint_bytes().unwrap(), before);
    let mut s = context(ContextPolicy::RiConfirmedClosingContextV1);
    s.state.identity.symbol = "RIU6".into();
    assert!(j.replay_through(&mut s, 1, clock()).is_err());
    assert_eq!(s.last_sequence(), 0);
}

#[test]
fn bounds_fail_before_writing_or_evicting_evidence() {
    let (_dir, mut j) = create();
    assert!(j
        .append("oversized", &"x".repeat(MAX_RAW_BYTES + 1))
        .is_err());
    assert_eq!(j.last_sequence(), 0);
    assert!(j.usage().writer_failed);
    assert!(j.root.join("FAILED.json").is_file());
    assert!(!j.root.join("00000001.raw.json").exists());
    let (_dir, mut j) = create();
    j.total_bytes = MAX_TOTAL_BYTES;
    assert!(j.append("one", "{}").is_err());
    assert!(j.seal_cutoff(clock()).is_err());
    assert_eq!(fs::read_dir(&j.root).unwrap().count(), 3);
}

#[test]
fn rejected_observation_cannot_be_silently_skipped_on_restart() {
    let (_dir, mut j) = create();
    assert!(j.append("", "{}").is_err());
    assert!(j.seal_cutoff(clock()).is_err());
    let root = j.root.clone();
    drop(j);
    assert!(root.join("FAILED.json").exists());
    assert!(open(&root).is_err());
}

#[test]
fn recomputed_checksum_cannot_hide_broken_receipt_chain_or_barrier() {
    for mutate_barrier in [false, true] {
        let (_dir, mut j) = create();
        j.append(
            "one",
            &bar(epoch("2026-10-09 07:00"), DataOrigin::History, 90000.),
        )
        .unwrap();
        j.seal_cutoff(clock()).unwrap();
        let root = j.root.clone();
        drop(j);
        if mutate_barrier {
            let path = root.join("barrier-0001.json");
            let mut payload: StoredBarrier = read_checked(path.clone()).unwrap();
            payload.barrier.ingress_sha256 = "0".repeat(64);
            fs::write(
                path,
                serde_json::to_vec(&Checked {
                    sha256: hash(&payload).unwrap(),
                    payload,
                })
                .unwrap(),
            )
            .unwrap();
        } else {
            let path = root.join("00000001.commit.json");
            let mut payload: Commit = read_checked(path.clone()).unwrap();
            payload.raw_sha256 = "0".repeat(64);
            fs::write(
                path,
                serde_json::to_vec(&Checked {
                    sha256: hash(&payload).unwrap(),
                    payload,
                })
                .unwrap(),
            )
            .unwrap();
        }
        assert!(open(&root).is_err());
    }
}

#[test]
fn symlink_file_is_not_read_or_overwritten() {
    #[cfg(unix)]
    {
        let (dir, j) = create();
        let root = j.root.clone();
        drop(j);
        let outside = dir.path().join("outside");
        fs::write(&outside, b"untouched").unwrap();
        fs::remove_file(root.join("writer.lock")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("writer.lock")).unwrap();
        assert!(open(&root).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"untouched");
    }
}

#[test]
fn context_processing_cannot_precede_commit_or_claim_future_time() {
    let (_dir, mut j) = create();
    set_clock(epoch("2026-10-09 07:10"));
    j.append(
        "one",
        &bar(epoch("2026-10-09 07:00"), DataOrigin::Live, 90000.),
    )
    .unwrap();
    let mut s = context(ContextPolicy::RiConfirmedClosingContextV1);
    assert!(j.replay_through(&mut s, 1, clock() - 1).is_err());
    assert!(j.replay_through(&mut s, 1, clock() + 1).is_err());
    assert_eq!(s.last_sequence(), 0);
    j.replay_through(&mut s, 1, clock()).unwrap();
    assert_eq!(s.last_sequence(), 1);
}

#[test]
fn delayed_processing_uses_original_receipt_not_restart_clock() {
    let (_dir, mut j) = create();
    set_clock(epoch("2026-10-13 03:00"));
    let original = j
        .append(
            "closing",
            &bar(epoch("2026-10-12 23:40"), DataOrigin::HistoryGap, 90500.),
        )
        .unwrap()
        .input
        .unwrap();
    let root = j.root.clone();
    drop(j);
    set_clock(epoch("2026-10-13 08:00"));
    let mut j = open(&root).unwrap();
    let barrier = j.seal_cutoff(epoch("2026-10-13 07:00")).unwrap();
    let mut s = context(ContextPolicy::RiConfirmedClosingContextV1);
    j.replay_through(&mut s, barrier.last_sequence, clock())
        .unwrap();
    assert_eq!(s.state.canonical[&original.bar.close_time_utc], original);
    assert_eq!(s.state.journal[0].route, ContextRoute::ContextOnly);
}
