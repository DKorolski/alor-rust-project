//! Explicit as-observed reconstruction, never a pre-opening receipt attestation.
use super::*;
use crate::ri_context::ingress::redis_source::{bar_payload, stream_id};
use redis::streams::StreamRangeReply;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateStart {
    pub version: String,
    pub source_first_live_session: NaiveDate,
    pub first_clean_session: NaiveDate,
    pub recovery: RecoveryPrefix,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPrefix {
    pub last_source_id: String,
    pub rows: u64,
    pub source_sha256: String,
    pub last_label_utc: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OutputQuality {
    pub diagnostic_session: NaiveDate,
    pub first_clean_session: NaiveDate,
    pub recovery_through_sequence: u64,
    pub current_session_clean_eligible: bool,
}

#[derive(Serialize)]
pub struct Preview {
    pub bootstrap: bootstrap::HistoricalBootstrap,
    pub recovery: RecoveryPrefix,
    pub anchors: [Vec<Anchor>; 2],
    pub first_clean_session: NaiveDate,
    pub inspected_at_utc: i64,
    pub order_emission_allowed: bool,
}

impl LateStart {
    pub(super) fn validate(&self, spec: &RunnerSpec, identity: &Identity) -> Result<()> {
        let bootstrap = spec
            .bootstrap
            .as_ref()
            .context("late start requires context bootstrap")?;
        ensure!(
            self.version == "diagnostic_late_start_v1",
            "unsupported diagnostic late start"
        );
        ensure!(
            self.first_clean_session
                == local(identity.next_cutoff(spec.first_decision_session)?)?.date(),
            "first clean session must follow diagnostic day"
        );
        ensure!(
            (self.source_first_live_session - spec.first_decision_session)
                .num_days()
                .abs()
                <= 14,
            "diagnostic source date binding outside bounded window"
        );
        ensure!(
            self.source_first_live_session <= self.first_clean_session,
            "source must be Live by first clean session"
        );
        ensure!(
            self.recovery.rows > bootstrap.rows
                && self.recovery.rows <= bootstrap::MAX_BOOTSTRAP_ROWS,
            "bounded recovery must include same-day bars"
        );
        ensure!(
            stream_id(&self.recovery.last_source_id)? > stream_id(&bootstrap.last_source_id)?
                && self.recovery.source_sha256.len() == 64
                && self
                    .recovery
                    .source_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid diagnostic recovery identity"
        );
        let label = local(self.recovery.last_label_utc)?;
        ensure!(
            label.date() == spec.first_decision_session
                && label.minute().is_multiple_of(10)
                && label.second() == 0
                && label.time() >= chrono::NaiveTime::from_hms_opt(7, 0, 0).unwrap()
                && label.time() <= chrono::NaiveTime::from_hms_opt(23, 20, 0).unwrap(),
            "diagnostic recovery outside bounded model window"
        );
        Ok(())
    }
}

pub(super) fn verify_forward_proof(collector: &Collector, input: &Receipt) -> Result<()> {
    let raw: Value = serde_json::from_str(collector.journal.raw_payload(input.sequence)?)?;
    let proof = &raw["source_proof"];
    let release = &proof["release"];
    let received = input
        .received_at_utc
        .context("diagnostic forward receipt missing")?;
    let close = input.bar.close_time_utc + 600;
    let cutoff = release["observation_cutoff_utc"]
        .as_i64()
        .context("diagnostic WS proof missing")?;
    let first = release["first_received_at_utc"]
        .as_i64()
        .context("diagnostic WS first receipt missing")?;
    let last = release["last_received_at_utc"]
        .as_i64()
        .context("diagnostic WS last receipt missing")?;
    ensure!(
        proof["kind"] == "as_observed_ws"
            && proof["ws_sequence_at_cutoff"]
                .as_u64()
                .is_some_and(|v| v > 0)
            && proof["release_id"]
                .as_str()
                .is_some_and(|v| !v.is_empty() && v.len() <= 256)
            && release["generation"].as_u64().is_some_and(|v| v > 0)
            && release["closed_at_utc"] == close
            && input.bar.close_time_utc <= first
            && first <= last
            && last <= cutoff
            && close <= cutoff
            && cutoff <= received,
        "invalid diagnostic forward WS proof"
    );
    if release["reason"] == "next_timestamp" {
        ensure!(
            release["next_label_utc"]
                .as_i64()
                .is_some_and(|t| close <= t && t <= cutoff),
            "invalid diagnostic next timestamp proof"
        );
    } else {
        ensure!(
            release["reason"] == "session_boundary_ws"
                && release["next_label_utc"].is_null()
                && local(input.bar.close_time_utc)?.time()
                    == chrono::NaiveTime::from_hms_opt(23, 40, 0).unwrap()
                && cutoff >= close + 10,
            "invalid diagnostic terminal proof"
        );
    }
    Ok(())
}

impl RecoveryPrefix {
    pub(super) fn verify(&self, inputs: &[Receipt], spec: &RunnerSpec, now: i64) -> Result<()> {
        let context = spec
            .bootstrap
            .as_ref()
            .context("late start missing bootstrap")?;
        ensure!(
            context.rows < inputs.len() as u64,
            "diagnostic context prefix exceeds recovery"
        );
        ensure!(
            inputs.len() as u64 == self.rows
                && inputs
                    .last()
                    .is_some_and(|r| r.source_id == self.last_source_id),
            "diagnostic recovery boundary mismatch"
        );
        ensure!(
            bootstrap::HistoricalBootstrap::digest(inputs)? == self.source_sha256,
            "diagnostic recovery digest mismatch"
        );
        context.verify(
            &inputs[..context.rows as usize],
            spec.first_decision_session,
        )?;
        let mut expected = spec.header()?.identities[0].cutoff(spec.first_decision_session);
        let mut previous = stream_id(&context.last_source_id)?;
        for input in &inputs[context.rows as usize..] {
            let id = stream_id(&input.source_id)?;
            ensure!(id > previous, "diagnostic recovery IDs not increasing");
            previous = id;
            ensure!(
                local(input.bar.close_time_utc)?.date() == spec.first_decision_session,
                "diagnostic recovery contains another date"
            );
            if input.bar.close_time_utc
                < spec.header()?.identities[0].cutoff(spec.first_decision_session)
            {
                continue;
            }
            ensure!(
                input.bar.close_time_utc == expected
                    && input.bar.symbol == spec.ingress.symbol
                    && input.bar.origin != DataOrigin::Replay,
                "diagnostic recovery requires complete canonical M10 grid"
            );
            ensure!(
                input
                    .received_at_utc
                    .is_some_and(|t| t >= input.bar.close_time_utc + 600)
                    && input.received_at_utc <= input.durable_at_utc
                    && input.durable_at_utc.is_some_and(|t| t <= now),
                "diagnostic recovery receipt clocks invalid"
            );
            expected += 600;
        }
        ensure!(
            expected == self.last_label_utc + 600
                && expected > spec.header()?.identities[0].cutoff(spec.first_decision_session),
            "diagnostic recovery has no closed model bars"
        );
        Ok(())
    }
}

/// Actual read-only Redis inspection; inspection clocks never replace WAL clocks.
pub async fn inspect(
    connection: &mut MultiplexedConnection,
    spec: &RunnerSpec,
    first: &str,
    last: &str,
) -> Result<Preview> {
    let header = spec.header()?;
    ensure!(
        stream_id(first)? != (0, 0) && stream_id(first)? <= stream_id(last)?,
        "invalid diagnostic range"
    );
    let response: StreamRangeReply = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        redis::cmd("XRANGE")
            .arg(&spec.ingress.source_stream)
            .arg(first)
            .arg(last)
            .arg("COUNT")
            .arg(bootstrap::MAX_BOOTSTRAP_ROWS + 1)
            .query_async(connection),
    )
    .await
    .context("diagnostic inspection timed out")??;
    ensure!(
        (3..=bootstrap::MAX_BOOTSTRAP_ROWS as usize).contains(&response.ids.len()),
        "diagnostic inspection row bound"
    );
    let now = Utc::now().timestamp();
    let inputs = response
        .ids
        .iter()
        .enumerate()
        .map(|(i, row)| {
            Ok(Receipt {
                sequence: i as u64 + 1,
                source_id: row.id.clone(),
                bar: serde_json::from_str(&bar_payload(row)?)?,
                received_at_utc: Some(now),
                durable_at_utc: Some(now),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let n = inputs
        .iter()
        .take_while(|r| {
            local(r.bar.close_time_utc).is_ok_and(|t| t.date() < spec.first_decision_session)
        })
        .count();
    ensure!(
        n > 0
            && n < inputs.len()
            && inputs[0].source_id == first
            && inputs.last().unwrap().source_id == last,
        "diagnostic context/recovery prefix missing"
    );
    let context = bootstrap::HistoricalBootstrap {
        first_source_id: first.into(),
        last_source_id: inputs[n - 1].source_id.clone(),
        rows: n as u64,
        source_sha256: bootstrap::HistoricalBootstrap::digest(&inputs[..n])?,
        first_session: local(inputs[0].bar.close_time_utc)?.date(),
        last_session: local(inputs[n - 1].bar.close_time_utc)?.date(),
    };
    context.verify(&inputs[..n], spec.first_decision_session)?;
    let recovery = RecoveryPrefix {
        last_source_id: last.into(),
        rows: inputs.len() as u64,
        source_sha256: bootstrap::HistoricalBootstrap::digest(&inputs)?,
        last_label_utc: inputs.last().unwrap().bar.close_time_utc,
    };
    let mut pinned = spec.clone();
    pinned.bootstrap = Some(context.clone());
    recovery.verify(&inputs, &pinned, now)?;
    if let Some(expected) = &spec.bootstrap {
        ensure!(expected == &context, "diagnostic context changed");
    }
    if let Some(expected) = &spec.late_start {
        ensure!(expected.recovery == recovery, "diagnostic recovery changed");
    }
    let mut anchors = [vec![], vec![]];
    for (arm, value) in anchors.iter_mut().enumerate() {
        let mut store = Store::new(header.identities[arm].clone())?;
        store.bootstrap_diagnostic(&inputs[..n], spec.first_decision_session, now)?;
        *value = store
            .collect_anchors(spec.first_decision_session, now + 1)?
            .0;
    }
    Ok(Preview {
        bootstrap: context,
        recovery,
        anchors,
        first_clean_session: local(header.identities[0].next_cutoff(spec.first_decision_session)?)?
            .date(),
        inspected_at_utc: now,
        order_emission_allowed: false,
    })
}
