//! Explicit, bounded context import. Source IDs and decoded bar content are
//! pinned in the launch spec; real receipt clocks are never replaced by history.
use super::*;
use crate::ri_context::ingress::redis_source::{bar_payload, stream_id};
use redis::streams::StreamRangeReply;

pub const MAX_BOOTSTRAP_ROWS: u64 = 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HistoricalBootstrap {
    pub first_source_id: String,
    pub last_source_id: String,
    pub rows: u64,
    pub source_sha256: String,
    pub first_session: NaiveDate,
    pub last_session: NaiveDate,
}

impl HistoricalBootstrap {
    pub(super) fn validate(&self, activation: NaiveDate) -> Result<()> {
        ensure!(
            self.first_source_id.len() <= 64 && self.last_source_id.len() <= 64,
            "oversized bootstrap boundary"
        );
        let first = stream_id(&self.first_source_id)?;
        let last = stream_id(&self.last_source_id)?;
        ensure!(
            first != (0, 0) && first <= last,
            "invalid bootstrap boundaries"
        );
        ensure!(
            (2..=MAX_BOOTSTRAP_ROWS).contains(&self.rows),
            "bootstrap row bound"
        );
        ensure!(
            self.source_sha256.len() == 64
                && self
                    .source_sha256
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "invalid bootstrap digest"
        );
        ensure!(
            self.first_session <= self.last_session
                && self.last_session < activation
                && activation
                    .signed_duration_since(self.first_session)
                    .num_days()
                    <= 14,
            "invalid bootstrap historical dates"
        );
        Ok(())
    }

    fn digest(inputs: &[Receipt]) -> Result<String> {
        hash(
            &inputs
                .iter()
                .map(|r| (&r.source_id, &r.bar))
                .collect::<Vec<_>>(),
        )
    }

    pub(super) fn verify(&self, inputs: &[Receipt], activation: NaiveDate) -> Result<()> {
        self.validate(activation)?;
        ensure!(inputs.len() as u64 == self.rows, "bootstrap count mismatch");
        ensure!(
            inputs
                .first()
                .is_some_and(|r| r.source_id == self.first_source_id)
                && inputs
                    .last()
                    .is_some_and(|r| r.source_id == self.last_source_id),
            "bootstrap boundary mismatch"
        );
        let mut previous = (0, 0);
        for input in inputs {
            let id = stream_id(&input.source_id)?;
            ensure!(id > previous, "bootstrap source IDs not increasing");
            previous = id;
            let date = local(input.bar.close_time_utc)?.date();
            ensure!(
                date >= self.first_session && date <= self.last_session,
                "bootstrap bar outside pinned dates"
            );
        }
        let dates = inputs
            .iter()
            .map(|r| local(r.bar.close_time_utc).map(|d| d.date()))
            .collect::<Result<BTreeSet<_>>>()?;
        ensure!(
            dates.first() == Some(&self.first_session) && dates.last() == Some(&self.last_session),
            "bootstrap date coverage mismatch"
        );
        ensure!(
            Self::digest(inputs)? == self.source_sha256,
            "bootstrap source digest mismatch"
        );
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct BootstrapPreview {
    pub bootstrap: HistoricalBootstrap,
    pub inspected_at_utc: i64,
    pub runner_spec_sha256: String,
    pub anchors: [Vec<Anchor>; 2],
    pub skipped: [Vec<Skipped>; 2],
    pub order_emission_allowed: bool,
}

/// Read-only preflight: select an explicit inclusive range, check full context
/// coverage in both arms, then pin this manifest in a reviewed launch spec.
/// No generation is created and these inspection timestamps are NOT imported.
pub async fn inspect(
    connection: &mut MultiplexedConnection,
    spec: &RunnerSpec,
    first_id: &str,
    last_id: &str,
) -> Result<BootstrapPreview> {
    let header = spec.header()?;
    ensure!(
        stream_id(first_id)? != (0, 0) && stream_id(first_id)? <= stream_id(last_id)?,
        "invalid bootstrap range"
    );
    let response: StreamRangeReply = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        redis::cmd("XRANGE")
            .arg(&spec.ingress.source_stream)
            .arg(first_id)
            .arg(last_id)
            .arg("COUNT")
            .arg(MAX_BOOTSTRAP_ROWS + 1)
            .query_async(connection),
    )
    .await
    .context("bootstrap inspection timed out")??;
    ensure!(
        (2..=MAX_BOOTSTRAP_ROWS as usize).contains(&response.ids.len()),
        "bootstrap row bound"
    );
    let now = Utc::now().timestamp();
    let inputs: Vec<Receipt> = response
        .ids
        .iter()
        .enumerate()
        .map(|(index, row)| {
            Ok(Receipt {
                sequence: index as u64 + 1,
                source_id: row.id.clone(),
                bar: serde_json::from_str(&bar_payload(row)?)?,
                received_at_utc: Some(now),
                durable_at_utc: Some(now),
            })
        })
        .collect::<Result<_>>()?;
    let dates = inputs
        .iter()
        .map(|r| local(r.bar.close_time_utc).map(|d| d.date()))
        .collect::<Result<BTreeSet<_>>>()?;
    let plan = HistoricalBootstrap {
        first_source_id: first_id.into(),
        last_source_id: last_id.into(),
        rows: inputs.len() as u64,
        source_sha256: HistoricalBootstrap::digest(&inputs)?,
        first_session: *dates.first().unwrap(),
        last_session: *dates.last().unwrap(),
    };
    plan.verify(&inputs, spec.first_decision_session)?;
    ensure!(
        spec.bootstrap
            .as_ref()
            .is_none_or(|expected| expected == &plan),
        "bootstrap inspection differs from pinned spec"
    );
    let mut state = PairState::new(&header)?;
    let mut anchors = [vec![], vec![]];
    let mut skipped = [vec![], vec![]];
    for (index, store) in state.stores.iter_mut().enumerate() {
        store.bootstrap(&inputs, spec.first_decision_session, now)?;
        (anchors[index], skipped[index]) = store.anchors_before(spec.first_decision_session)?;
    }
    Ok(BootstrapPreview {
        bootstrap: plan,
        inspected_at_utc: now,
        runner_spec_sha256: hash(spec)?,
        anchors,
        skipped,
        order_emission_allowed: false,
    })
}
