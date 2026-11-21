//! Controlled kernel-shaped clock/HTTP fixtures, never real host/PIT evidence.
use super::*;
use crate::shadow_oi::{
    collector::{
        self,
        io::{Clock, ClockReading, EvidenceKind, Response, SystemDisk, Transport},
        Policy,
    },
    Plan,
};
use std::time::Duration as Wait;

struct ControlledClock {
    utc: DateTime<Utc>,
    mono: u64,
}
impl Clock for ControlledClock {
    fn kind(&self) -> EvidenceKind {
        EvidenceKind::LinuxKernelClock
    }
    fn read(&mut self) -> Result<ClockReading> {
        self.utc += Duration::milliseconds(1);
        self.mono += 1_000_000;
        Ok(ClockReading {
            utc: self.utc,
            monotonic_ns: self.mono,
            synchronized: true,
            offset_ms: Some(0),
            max_error_ms: Some(0),
        })
    }
    async fn sleep(&mut self, duration: Wait) {
        self.utc += Duration::from_std(duration).unwrap();
        self.mono += duration.as_nanos() as u64;
    }
}
struct FixedResponse {
    server_date: String,
    prior_oi: u64,
}
impl Transport for FixedResponse {
    async fn get(&mut self, plan: &Plan, _: Wait) -> Response {
        let params = plan.request_params();
        Response { status:Some(200), server_date:Some(self.server_date.clone()), failure:None,
            body:serde_json::to_vec(&json!({"history":{"columns":["BOARDID","SECID","TRADEDATE","OPENPOSITION"],
                "data":[["RFUD","IMOEXF",params["from"],100],["RFUD","IMOEXF",params["till"],self.prior_oi]]}})).unwrap() }
    }
}
async fn captured(root: &Path, spec: &Spec) {
    captured_value(root, spec, 90).await;
}
async fn captured_value(root: &Path, spec: &Spec, prior_oi: u64) {
    let cal = spec.calendar().unwrap();
    let plan = Plan::new(&cal, spec.date, &spec.model_sha256).unwrap();
    let utc = DateTime::from_timestamp(cal.bounds_utc(spec.date).unwrap().0 - 600, 0).unwrap();
    let q = collector::collect(
        root,
        &plan,
        &Policy::default(),
        &mut ControlledClock { utc, mono: 0 },
        &mut FixedResponse {
            server_date: utc.to_rfc2822(),
            prior_oi,
        },
        &mut SystemDisk,
    )
    .await
    .unwrap();
    assert_eq!(q.allow_new_bo(), prior_oi < 100);
}

#[tokio::test]
async fn qualified_gate_is_copied_once_and_recovery_never_reads_deleted_external_store() {
    let (root, spec, mut source, mut session) = setup();
    let oi = root.path().join("oi");
    captured(&oi, &spec).await;
    let out = freeze(&mut source, &mut session, &spec, Some(&oi));
    assert!(out.ready && out.oi_allows_new_bo && out.oi_input_valid);
    std::fs::remove_dir_all(&oi).unwrap();
    drop(session);
    session = Session::open_with_clock(&root.path().join("session"), spec.clone(), &source, clock)
        .unwrap();
    assert_eq!(session.committed_output(1).unwrap(), out);
    let mut records = Vec::new();
    for row in fixture()["series"]["rounds"].as_array().unwrap() {
        let b = bar(row, DataOrigin::Live);
        at(b.close_time_utc + 601);
        append(&mut source, &b);
        let out = session.observe(&source).unwrap().unwrap();
        assert!(out.oi_allows_new_bo && out.ready);
        records.extend(out.records);
    }
    let book = |arm| {
        records
            .iter()
            .filter(|r| r.arm == arm)
            .map(|r| (&r.kind, &r.payload))
            .collect::<Vec<_>>()
    };
    assert_eq!(book(Arm::ImoAb), book(Arm::ImoAbOi));
    assert!(records
        .iter()
        .any(|r| r.arm == Arm::ImoAbOi && r.kind == "trade" && r.payload["component"] == "BO"));
}

#[tokio::test]
async fn missing_or_invalid_oi_disables_only_new_bo_and_cannot_enable_later() {
    for invalid in [false, true] {
        let (root, spec, mut source, mut session) = setup();
        let good = root.path().join("prequalified");
        captured(&good, &spec).await;
        let chosen = root.path().join("selected-oi");
        if invalid {
            std::fs::create_dir_all(chosen.join("qualified")).unwrap();
            std::fs::write(
                chosen.join("qualified/2026-11-09.json"),
                b"invalid certificate",
            )
            .unwrap();
        }
        let saved = freeze(&mut source, &mut session, &spec, Some(&chosen));
        assert!(saved.ready && !saved.oi_allows_new_bo && !saved.oi_input_valid);
        // Out-of-band late delivery of previously qualified bytes is not a new
        // session decision. The committed missing/invalid choice stays closed.
        if invalid {
            std::fs::remove_dir_all(&chosen).unwrap();
        }
        std::fs::rename(good, &chosen).unwrap();
        assert!(crate::shadow_oi::collector::snapshot::Snapshot::capture(
            &chosen,
            &Plan::new(&spec.calendar().unwrap(), spec.date, &spec.model_sha256).unwrap()
        )
        .unwrap()
        .unwrap()
        .verify(&Plan::new(&spec.calendar().unwrap(), spec.date, &spec.model_sha256).unwrap())
        .unwrap()
        .allow_new_bo());
        drop(session);
        session =
            Session::open_with_clock(&root.path().join("session"), spec.clone(), &source, clock)
                .unwrap();
        assert_eq!(session.committed_output(1).unwrap(), saved);
        assert!(session
            .freeze(
                &source,
                source.sealed_barriers().unwrap()[0].clone(),
                Some(&chosen)
            )
            .is_err());
        let b = bar(&fixture()["series"]["rounds"][0], DataOrigin::Live);
        at(b.close_time_utc + 601);
        append(&mut source, &b);
        let out = session.observe(&source).unwrap().unwrap();
        assert!(out.ready && !out.oi_allows_new_bo);
    }
}

#[tokio::test]
async fn nondecreasing_oi_is_valid_closed_input_not_missing_data() {
    let (root, spec, mut source, mut session) = setup();
    let oi = root.path().join("oi");
    captured_value(&oi, &spec, 110).await;
    let out = freeze(&mut source, &mut session, &spec, Some(&oi));
    assert!(out.ready && out.oi_input_valid && !out.oi_allows_new_bo);
    assert_eq!(out.oi_status, "accepted_snapshot");
}
