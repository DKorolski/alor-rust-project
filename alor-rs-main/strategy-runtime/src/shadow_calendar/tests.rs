use super::*;

fn date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
}
fn session(day: &str, kind: SessionKind) -> Session {
    let mut s = Session {
        date: date(day),
        exchange_trade_date: None,
        kind,
        context_open_minute: None,
        close_minute: None,
        main_open_minute: None,
        main_close_minute: None,
        breaks: vec![],
        note: "synthetic_contract_fixture_not_full_reference_replay".into(),
    };
    if matches!(kind, SessionKind::Regular | SessionKind::Dsvd) {
        s.context_open_minute = Some(if kind == SessionKind::Dsvd { 590 } else { 420 });
        s.close_minute = Some(if kind == SessionKind::Dsvd {
            1140
        } else {
            1430
        });
        s.main_open_minute = Some(600);
        s.main_close_minute = Some(1140);
    }
    s
}
fn definition(sessions: Vec<Session>) -> Definition {
    Definition {
        schema_version: SCHEMA.into(),
        calendar_version: "frozen-research-calendar-test-v1".into(),
        reference_sha256: "a".repeat(64),
        symbol: "IMOEXF".into(),
        first_date: date("2026-11-01"),
        last_date: date("2026-11-30"),
        sessions,
    }
}
fn calendar(sessions: Vec<Session>) -> Calendar {
    Calendar::new(definition(sessions)).unwrap()
}
fn complete(c: &Calendar, days: &[&str]) -> BTreeMap<NaiveDate, Coverage> {
    days.iter()
        .map(|day| {
            let day = date(day);
            (day, c.coverage(day, &c.expected_labels_utc(day)))
        })
        .collect()
}

fn pin(d: &Definition, bytes: &[u8]) -> ArtifactPin {
    ArtifactPin {
        artifact_sha256: format!("{:x}", Sha256::digest(bytes)),
        reference_sha256: d.reference_sha256.clone(),
        calendar_version: d.calendar_version.clone(),
        symbol: d.symbol.clone(),
        first_date: d.first_date,
        last_date: d.last_date,
    }
}

#[test]
fn registry_load_is_bounded_and_checks_reviewed_identity_and_exact_bytes() {
    let d = definition(vec![session("2026-11-16", SessionKind::Regular)]);
    let bytes = serde_json::to_vec(&d).unwrap();
    let expected = pin(&d, &bytes);
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("calendar.json");
    std::fs::write(&file, &bytes).unwrap();
    let c = Calendar::load(&file, &expected).unwrap();
    assert_eq!(c.session(date("2026-11-16")).kind, SessionKind::Regular);
    assert_eq!(c.session(date("2026-12-01")).kind, SessionKind::Unknown);
    for field in ["symbol", "version", "reference", "horizon", "artifact"] {
        let mut wrong = expected.clone();
        match field {
            "symbol" => wrong.symbol = "USDRUBF".into(),
            "version" => wrong.calendar_version.push('2'),
            "reference" => wrong.reference_sha256 = "b".repeat(64),
            "horizon" => wrong.last_date = date("2026-12-01"),
            _ => wrong.artifact_sha256 = "b".repeat(64),
        }
        assert!(Calendar::from_json(&bytes, &wrong).is_err(), "{field}");
    }
    let mut changed = bytes.clone();
    changed.push(b' ');
    assert!(Calendar::from_json(&changed, &expected).is_err());
    let oversized = vec![b' '; MAX_REGISTRY_BYTES + 1];
    assert!(Calendar::from_json(&oversized, &expected).is_err());
    let malformed = br#"{"schema_version":"duplicate","schema_version":"duplicate"}"#;
    let duplicate_pin = pin(&d, malformed);
    assert!(Calendar::from_json(malformed, &duplicate_pin).is_err());
}

#[test]
fn oi_source_dates_ignore_dsvd_but_never_jump_over_unknown_regular_frontier() {
    let c = calendar(vec![
        session("2026-11-12", SessionKind::Regular),
        session("2026-11-13", SessionKind::Regular),
        session("2026-11-14", SessionKind::Dsvd),
        session("2026-11-15", SessionKind::Dsvd),
        session("2026-11-16", SessionKind::Regular),
    ]);
    assert_eq!(
        c.prior_regular_dates(date("2026-11-16")).unwrap(),
        (date("2026-11-13"), date("2026-11-12"))
    );
    assert!(c.prior_regular_dates(date("2026-11-14")).is_err());
    assert!(c.prior_regular_dates(date("2026-12-01")).is_err());
    let missing = calendar(vec![
        session("2026-11-12", SessionKind::Regular),
        session("2026-11-14", SessionKind::Dsvd),
        session("2026-11-15", SessionKind::Dsvd),
        session("2026-11-16", SessionKind::Regular),
    ]);
    assert!(missing.prior_regular_dates(date("2026-11-16")).is_err());
}

#[test]
fn pinned_python_calendar_grids_coverage_and_a_b_selection_match() {
    #[derive(Deserialize)]
    struct Fixture {
        schema_version: String,
        reference_sha256: String,
        session_functions_sha256: String,
        cases: Vec<Case>,
    }
    #[derive(Deserialize)]
    struct Case {
        id: String,
        definition: Definition,
        pin: ArtifactPin,
        target: NaiveDate,
        days: Vec<Day>,
        selections: Vec<ExpectedSelection>,
    }
    #[derive(Deserialize)]
    struct Day {
        date: NaiveDate,
        expected_labels: Vec<i64>,
        observed_labels: Vec<i64>,
        accepted: bool,
    }
    #[derive(Deserialize)]
    struct ExpectedSelection {
        policy: ContextPolicy,
        previous: Option<NaiveDate>,
        previous2: Option<NaiveDate>,
        ready: bool,
    }
    let bytes = include_bytes!("../../tests/fixtures/calendar_reference_v1.json");
    let fixture: Fixture = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture.schema_version, "moex_calendar_reference_fixture_v1");
    assert_eq!(
        fixture.reference_sha256,
        "76c50e7d7a43a256472b867ea3e0bb092a61c43732c37d93fd9757d51933177d"
    );
    assert_eq!(
        fixture.session_functions_sha256,
        "5bb6b17a2863b1053750160252f44b21c14eafd33df1787a3cffb8bbf74ec46e"
    );
    assert_eq!(fixture.cases.len(), 38);
    for case in fixture.cases {
        let bytes = serde_json::to_vec(&case.definition).unwrap();
        let c = Calendar::from_json(&bytes, &case.pin).unwrap();
        let mut coverage = BTreeMap::new();
        for day in case.days {
            assert_eq!(
                c.expected_labels_utc(day.date),
                day.expected_labels,
                "{} / {} grid",
                case.id,
                day.date
            );
            let observed = c.coverage(day.date, &day.observed_labels);
            assert_eq!(
                observed.accepted, day.accepted,
                "{} / {} coverage",
                case.id, day.date
            );
            coverage.insert(day.date, observed);
        }
        for expected in case.selections {
            let actual = c
                .select_components(case.target, expected.policy, expected.policy, &coverage)
                .mr;
            assert_eq!(actual.previous, expected.previous, "{} previous", case.id);
            assert_eq!(
                actual.previous2, expected.previous2,
                "{} previous2",
                case.id
            );
            assert_eq!(actual.ready, expected.ready, "{} readiness", case.id);
        }
    }
}

#[test]
fn unknown_date_never_uses_weekday_fallback_or_disables_risk_management() {
    let c = calendar(vec![]);
    let route = c.route(
        date("2026-11-16"),
        420,
        ContextPolicy::RegularOnly,
        ContextPolicy::RegularAndDsvd,
    );
    assert_eq!(route.kind, SessionKind::Unknown);
    assert!(
        !route.new_entry_allowed
            && !route.mr_context_update_allowed
            && !route.bo_context_update_allowed
    );
    assert!(route.position_management_allowed);
}

#[test]
fn holiday_dsvd_on_wednesday_is_context_only_for_b() {
    let c = calendar(vec![session("2026-11-04", SessionKind::Dsvd)]);
    let route = c.route(
        date("2026-11-04"),
        590,
        ContextPolicy::RegularOnly,
        ContextPolicy::RegularAndDsvd,
    );
    assert!(!route.mr_context_update_allowed && route.bo_context_update_allowed);
    assert!(!route.new_entry_allowed && route.position_management_allowed);
}

#[test]
fn regular_working_saturday_updates_a_but_does_not_enable_entries() {
    let mut s = session("2025-11-01", SessionKind::Regular);
    s.context_open_minute = Some(540);
    let mut d = definition(vec![s]);
    d.first_date = date("2025-11-01");
    d.last_date = date("2025-11-03");
    let c = Calendar::new(d).unwrap();
    let route = c.route(
        date("2025-11-01"),
        540,
        ContextPolicy::RegularOnly,
        ContextPolicy::RegularAndDsvd,
    );
    assert!(route.mr_context_update_allowed && route.bo_context_update_allowed);
    assert!(!route.new_entry_allowed);
}

#[test]
fn regular_service_bar_is_not_model_input() {
    let c = calendar(vec![session("2026-11-16", SessionKind::Regular)]);
    let day = date("2026-11-16");
    let route = c.route(
        day,
        410,
        ContextPolicy::RegularOnly,
        ContextPolicy::RegularAndDsvd,
    );
    assert!(!route.mr_context_update_allowed && !route.new_entry_allowed);
    let mut labels = c.expected_labels_utc(day);
    labels.push(Calendar::epoch(day, 410));
    let coverage = c.coverage(day, &labels);
    assert!(coverage.accepted);
    assert_eq!(coverage.observed_bins, 101);
    assert_eq!(coverage.ignored_labels, 1);
}

#[test]
fn july_transition_is_explicit_registry_data() {
    let mut prior = session("2026-07-13", SessionKind::Regular);
    prior.context_open_minute = Some(540);
    let mut d = definition(vec![prior, session("2026-07-14", SessionKind::Regular)]);
    d.first_date = date("2026-07-13");
    d.last_date = date("2026-07-14");
    let c = Calendar::new(d).unwrap();
    assert_eq!(c.expected_labels_utc(date("2026-07-13")).len(), 89);
    assert_eq!(c.expected_labels_utc(date("2026-07-14")).len(), 101);
    assert!(
        !c.route(
            date("2026-07-13"),
            530,
            ContextPolicy::RegularOnly,
            ContextPolicy::RegularOnly
        )
        .new_entry_allowed
    );
}

#[test]
fn main_session_start_is_independent_of_full_context_opening() {
    let mut s = session("2026-11-16", SessionKind::Regular);
    s.main_open_minute = Some(540);
    let c = calendar(vec![s]);
    assert_eq!(c.session(date("2026-11-16")).main_open_minute, Some(540));
    assert_eq!(c.expected_labels_utc(date("2026-11-16")).len(), 101);
}

#[test]
fn dsvd_0950_is_context_and_missing_it_can_still_meet_frozen_guard() {
    let c = calendar(vec![session("2026-11-14", SessionKind::Dsvd)]);
    let day = date("2026-11-14");
    let labels = c.expected_labels_utc(day);
    assert_eq!(labels.len(), 55);
    let coverage = c.coverage(day, &labels[1..]);
    assert!(coverage.accepted);
    assert_eq!(coverage.required_bins, 44);
    assert_eq!(coverage.observed_bins, 54);
    assert_eq!(coverage.missing_labels_utc, vec![Calendar::epoch(day, 590)]);
}

#[test]
fn extended_holiday_uses_68_of_84_with_terminal_span() {
    let mut s = session("2026-11-04", SessionKind::Dsvd);
    s.close_minute = Some(1430);
    let c = calendar(vec![s]);
    let day = date("2026-11-04");
    let labels = c.expected_labels_utc(day);
    assert_eq!(labels.len(), 84);
    let keep: Vec<_> = labels
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == 0 || *i >= 17)
        .map(|(_, v)| *v)
        .collect();
    let coverage = c.coverage(day, &keep);
    assert!(coverage.accepted);
    assert_eq!(coverage.required_bins, 68);
    assert!(!c.coverage(day, &labels[..83]).accepted);
}

#[test]
fn dsvd_count_without_opening_span_is_not_ready() {
    let c = calendar(vec![session("2026-11-14", SessionKind::Dsvd)]);
    let day = date("2026-11-14");
    let labels = c.expected_labels_utc(day);
    assert!(!c.coverage(day, &labels[11..]).accepted);
}

#[test]
fn missing_regular_bar_cannot_be_silently_accepted() {
    let c = calendar(vec![session("2026-11-16", SessionKind::Regular)]);
    let day = date("2026-11-16");
    let mut labels = c.expected_labels_utc(day);
    labels.remove(40);
    let coverage = c.coverage(day, &labels);
    assert!(!coverage.accepted);
    assert_eq!(coverage.reason, CoverageReason::IncompleteRegular);
    assert_eq!(coverage.missing_labels_utc.len(), 1);
}

#[test]
fn duplicate_canonical_labels_do_not_count_as_coverage() {
    let c = calendar(vec![session("2026-11-16", SessionKind::Regular)]);
    let day = date("2026-11-16");
    let mut labels = c.expected_labels_utc(day);
    labels.push(labels[0]);
    assert_eq!(
        c.coverage(day, &labels).reason,
        CoverageReason::DuplicateLabels
    );
}

#[test]
fn ordinary_grid_preserves_partly_tradable_legacy_break_bins() {
    let mut s = session("2026-11-16", SessionKind::Regular);
    s.breaks = vec![
        Break {
            start_minute: 840,
            end_minute: 845,
        },
        Break {
            start_minute: 1125,
            end_minute: 1140,
        },
    ];
    let c = calendar(vec![s]);
    let day = date("2026-11-16");
    let labels = c.expected_labels_utc(day);
    assert!(labels.contains(&Calendar::epoch(day, 840)));
    assert!(labels.contains(&Calendar::epoch(day, 1120)));
    assert!(!labels.contains(&Calendar::epoch(day, 1130)));
}

#[test]
fn closed_day_leaves_previous_context_intact() {
    let c = calendar(vec![
        session("2026-11-01", SessionKind::Closed),
        session("2026-11-02", SessionKind::Regular),
        session("2026-11-03", SessionKind::Regular),
        session("2026-11-04", SessionKind::Closed),
        session("2026-11-05", SessionKind::Regular),
    ]);
    let coverage = complete(&c, &["2026-11-02", "2026-11-03"]);
    let selected = c.select_components(
        date("2026-11-05"),
        ContextPolicy::RegularOnly,
        ContextPolicy::RegularAndDsvd,
        &coverage,
    );
    assert!(selected.combo_ready);
    assert_eq!(selected.mr.previous, Some(date("2026-11-03")));
    assert_eq!(selected.bo.previous2, Some(date("2026-11-02")));
}

#[test]
fn missing_expected_dsvd_blocks_bo_without_replacing_it_with_older_regular() {
    let c = calendar(vec![
        session("2026-11-02", SessionKind::Regular),
        session("2026-11-03", SessionKind::Regular),
        session("2026-11-04", SessionKind::Dsvd),
        session("2026-11-05", SessionKind::Regular),
    ]);
    let coverage = complete(&c, &["2026-11-02", "2026-11-03"]);
    let selected = c.select_components(
        date("2026-11-05"),
        ContextPolicy::RegularOnly,
        ContextPolicy::RegularAndDsvd,
        &coverage,
    );
    assert!(selected.mr.ready);
    assert!(!selected.bo.ready && !selected.combo_ready);
    assert_eq!(selected.bo.previous, Some(date("2026-11-04")));
    assert_eq!(selected.bo.unresolved_dates, vec![date("2026-11-04")]);
}

#[test]
fn separate_mr_bo_previous2_dates_are_retained() {
    let c = calendar(vec![
        session("2026-11-02", SessionKind::Regular),
        session("2026-11-03", SessionKind::Regular),
        session("2026-11-04", SessionKind::Dsvd),
        session("2026-11-05", SessionKind::Regular),
    ]);
    let coverage = complete(&c, &["2026-11-02", "2026-11-03", "2026-11-04"]);
    let selected = c.select_components(
        date("2026-11-05"),
        ContextPolicy::RegularOnly,
        ContextPolicy::RegularAndDsvd,
        &coverage,
    );
    assert!(selected.combo_ready);
    assert_eq!(selected.target_date, date("2026-11-05"));
    assert_eq!(selected.symbol, "IMOEXF");
    assert_eq!(selected.mr.previous2, Some(date("2026-11-02")));
    assert_eq!(selected.bo.previous2, Some(date("2026-11-03")));
}

#[test]
fn unclassified_expected_date_blocks_both_contexts() {
    let c = calendar(vec![
        session("2026-11-02", SessionKind::Regular),
        session("2026-11-03", SessionKind::Regular),
        session("2026-11-05", SessionKind::Regular),
    ]);
    let coverage = complete(&c, &["2026-11-02", "2026-11-03"]);
    let selected = c.select_components(
        date("2026-11-05"),
        ContextPolicy::RegularOnly,
        ContextPolicy::RegularAndDsvd,
        &coverage,
    );
    assert!(!selected.mr.ready && !selected.bo.ready);
    assert_eq!(selected.mr.previous, Some(date("2026-11-04")));
}

#[test]
fn older_unknown_does_not_poison_two_later_complete_selected_sessions() {
    let c = calendar(vec![
        session("2026-11-05", SessionKind::Regular),
        session("2026-11-06", SessionKind::Regular),
        session("2026-11-07", SessionKind::Closed),
        session("2026-11-08", SessionKind::Closed),
        session("2026-11-09", SessionKind::Regular),
    ]);
    let coverage = complete(&c, &["2026-11-05", "2026-11-06"]);
    let selected = c.select_components(
        date("2026-11-09"),
        ContextPolicy::RegularOnly,
        ContextPolicy::RegularAndDsvd,
        &coverage,
    );
    assert!(selected.combo_ready);
    assert_eq!(selected.bo.previous, Some(date("2026-11-06")));
    assert_eq!(selected.bo.previous2, Some(date("2026-11-05")));
}

#[test]
fn closed_product_session_cannot_invent_context_or_reset_prior_dates() {
    let c = calendar(vec![session("2026-11-14", SessionKind::Closed)]);
    let route = c.route(
        date("2026-11-14"),
        590,
        ContextPolicy::RegularOnly,
        ContextPolicy::RegularAndDsvd,
    );
    assert!(
        !route.new_entry_allowed
            && !route.mr_context_update_allowed
            && !route.bo_context_update_allowed
    );
    assert_eq!(
        c.coverage(date("2026-11-14"), &[]).reason,
        CoverageReason::Closed
    );
}

#[test]
fn coverage_from_another_calendar_is_not_a_readiness_certificate() {
    let c = calendar(vec![
        session("2026-11-02", SessionKind::Regular),
        session("2026-11-03", SessionKind::Regular),
        session("2026-11-04", SessionKind::Closed),
        session("2026-11-05", SessionKind::Regular),
    ]);
    let mut coverage = complete(&c, &["2026-11-02", "2026-11-03"]);
    coverage
        .get_mut(&date("2026-11-03"))
        .unwrap()
        .calendar_sha256 = "b".repeat(64);
    assert!(
        !c.select_components(
            date("2026-11-05"),
            ContextPolicy::RegularOnly,
            ContextPolicy::RegularOnly,
            &coverage
        )
        .combo_ready
    );
}

#[test]
fn calendar_dates_are_not_merged_into_exchange_trade_date() {
    let mut s = session("2026-11-14", SessionKind::Dsvd);
    s.exchange_trade_date = Some(date("2026-11-16"));
    let c = calendar(vec![s]);
    assert_eq!(c.session(date("2026-11-14")).date, date("2026-11-14"));
    assert_eq!(c.session(date("2026-11-16")).kind, SessionKind::Unknown);
}

#[test]
fn utc_boundaries_are_explicit_moscow_offsets() {
    let c = calendar(vec![session("2026-11-16", SessionKind::Regular)]);
    let (open, close) = c.bounds_utc(date("2026-11-16")).unwrap();
    assert_eq!(
        open,
        date("2026-11-16")
            .and_hms_opt(4, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp()
    );
    assert_eq!(
        close,
        date("2026-11-16")
            .and_hms_opt(20, 50, 0)
            .unwrap()
            .and_utc()
            .timestamp()
    );
}

#[test]
fn invalid_registry_is_rejected_before_use() {
    let s = session("2026-11-16", SessionKind::Regular);
    assert!(Calendar::new(definition(vec![s.clone(), s.clone()])).is_err());
    let mut invalid = s;
    invalid.close_minute = Some(1450);
    assert!(Calendar::new(definition(vec![invalid])).is_err());
    let mut d = definition(vec![]);
    d.reference_sha256.clear();
    assert!(Calendar::new(d).is_err());
    let mut overflow = session("2026-11-16", SessionKind::Regular);
    overflow.date = NaiveDate::MAX;
    let mut d = definition(vec![overflow]);
    d.first_date = NaiveDate::MAX;
    d.last_date = NaiveDate::MAX;
    assert!(Calendar::new(d).is_err());
}

#[test]
fn definition_roundtrip_preserves_identity_and_rejects_unknown_fields() {
    let d = definition(vec![session("2026-11-16", SessionKind::Regular)]);
    let raw = serde_json::to_vec(&d).unwrap();
    let restored: Definition = serde_json::from_slice(&raw).unwrap();
    assert_eq!(
        Calendar::new(d).unwrap().sha256(),
        Calendar::new(restored).unwrap().sha256()
    );
    let mut json: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    json["enable_live_orders"] = true.into();
    assert!(serde_json::from_value::<Definition>(json).is_err());
}
