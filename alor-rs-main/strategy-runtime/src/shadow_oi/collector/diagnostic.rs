//! One fixed-endpoint smoke request. Never writes or mints qualified evidence.
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::super::{dated_values, digest, Plan, Reason, HISTORY_URL, MAX_BYTES};
use super::io::{Clock, ClockReading, EvidenceKind, FetchFailure, Response, Transport};

#[derive(Debug, Serialize)]
pub struct Probe {
    pub version: &'static str,
    pub diagnostic_only: bool,
    pub qualified: bool,
    pub allow_new_bo: bool,
    pub order_emission_allowed: bool,
    pub passed: bool,
    pub plan: Plan,
    pub url: &'static str,
    pub params: std::collections::BTreeMap<String, String>,
    pub evidence_kind: EvidenceKind,
    pub requested_at: ClockReading,
    pub received_at: ClockReading,
    pub clock_healthy: bool,
    pub http_status: Option<u16>,
    pub server_date: Option<String>,
    pub server_date_fresh: bool,
    pub failure: Option<FetchFailure>,
    pub schema_error: Option<Reason>,
    pub prior_oi: Option<f64>,
    pub prior2_oi: Option<f64>,
    pub response_sha256: String,
    pub raw_body: Vec<u8>,
}

fn inspect(
    plan: &Plan,
    evidence_kind: EvidenceKind,
    requested_at: ClockReading,
    received_at: ClockReading,
    response: Response,
) -> Probe {
    let clock_healthy = evidence_kind == EvidenceKind::LinuxKernelClock
        && requested_at.healthy()
        && received_at.follows(&requested_at)
        && received_at.utc - requested_at.utc <= chrono::Duration::seconds(30);
    let server_date_fresh = response
        .server_date
        .as_deref()
        .and_then(|v| DateTime::parse_from_rfc2822(v).ok())
        .is_some_and(|v| {
            (received_at.utc - v.with_timezone(&Utc))
                .num_seconds()
                .unsigned_abs()
                <= 300
        });
    let values = dated_values(plan, &response.body);
    let passed = clock_healthy
        && server_date_fresh
        && response.status == Some(200)
        && response.failure.is_none()
        && response.body.len() <= MAX_BYTES
        && values.is_ok();
    let (prior_oi, prior2_oi) = values
        .as_ref()
        .map_or((None, None), |(a, b)| (Some(*a), Some(*b)));
    Probe {
        version: "imoexf_oi_non_pit_probe_v1",
        diagnostic_only: true,
        qualified: false,
        allow_new_bo: false,
        order_emission_allowed: false,
        passed,
        plan: plan.clone(),
        url: HISTORY_URL,
        params: plan.request_params(),
        evidence_kind,
        requested_at,
        received_at,
        clock_healthy,
        http_status: response.status,
        server_date: response.server_date,
        server_date_fresh,
        failure: response.failure,
        schema_error: values.err(),
        prior_oi,
        prior2_oi,
        response_sha256: digest(&response.body),
        raw_body: response.body,
    }
}

pub async fn probe(
    plan: &Plan,
    clock: &mut impl Clock,
    transport: &mut impl Transport,
) -> Result<Probe> {
    let requested = clock.read()?;
    let response = transport.get(plan, Duration::from_secs(30)).await;
    let received = clock.read()?;
    Ok(inspect(plan, clock.kind(), requested, received, response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn inputs() -> (Plan, ClockReading, ClockReading, Response) {
        let plan = Plan {
            schema_version: "imoexf_pit_oi_evidence_v1".into(),
            profile_id: "IMO_AB_OI".into(),
            model_sha256: "a".repeat(64),
            calendar_sha256: "b".repeat(64),
            decision_date: "2026-11-20".parse().unwrap(),
            previous: "2026-11-19".parse().unwrap(),
            previous2: "2026-11-18".parse().unwrap(),
        };
        let start = ClockReading {
            utc: "2026-11-20T12:00:00Z".parse().unwrap(),
            monotonic_ns: 0,
            synchronized: true,
            offset_ms: Some(1),
            max_error_ms: Some(5),
        };
        let mut end = start.clone();
        end.utc += chrono::Duration::seconds(1);
        end.monotonic_ns = 1_000_000_000;
        let response = Response {
            status: Some(200),
            server_date: Some("Fri, 20 Nov 2026 12:00:01 GMT".into()),
            failure: None,
            body: serde_json::to_vec(&json!({"history": {
                "columns": ["BOARDID", "SECID", "TRADEDATE", "OPENPOSITION"],
                "data": [["RFUD", "IMOEXF", "2026-11-18", 100],
                         ["RFUD", "IMOEXF", "2026-11-19", 90]]
            }}))
            .unwrap(),
        };
        (plan, start, end, response)
    }

    #[test]
    fn outside_window_probe_never_qualifies_or_enables_bo() {
        let (p, a, b, r) = inputs();
        let report = inspect(&p, EvidenceKind::LinuxKernelClock, a, b, r);
        assert!(report.passed && report.diagnostic_only);
        assert!(!report.qualified && !report.allow_new_bo && !report.order_emission_allowed);
        assert_eq!(report.prior_oi, Some(90.));
        assert!(serde_json::from_value::<super::super::Qualification>(
            serde_json::to_value(report).unwrap()
        )
        .is_err());
    }

    #[test]
    fn probe_rejects_bad_or_non_native_clock() {
        for kind in [EvidenceKind::OfflineFixture, EvidenceKind::UnsupportedClock] {
            let (p, a, b, r) = inputs();
            assert!(!inspect(&p, kind, a, b, r).passed);
        }
        let (p, a, mut b, r) = inputs();
        b.max_error_ms = Some(1001);
        assert!(!inspect(&p, EvidenceKind::LinuxKernelClock, a, b, r).passed);
    }

    #[test]
    fn probe_rejects_partial_body_and_stale_server_date() {
        let (p, a, b, mut r) = inputs();
        r.failure = Some(FetchFailure::IncompleteBody);
        assert!(!inspect(&p, EvidenceKind::LinuxKernelClock, a, b, r).passed);
        let (p, a, b, mut r) = inputs();
        r.server_date = None;
        assert!(!inspect(&p, EvidenceKind::LinuxKernelClock, a, b, r).passed);
    }

    #[test]
    fn probe_uses_existing_exact_dated_schema_validation() {
        let (p, a, b, mut r) = inputs();
        r.body = r.body.windows(10).next().unwrap().to_vec();
        let report = inspect(&p, EvidenceKind::LinuxKernelClock, a, b, r);
        assert!(!report.passed);
        assert_eq!(report.schema_error, Some(Reason::InvalidSchema));
    }
}
