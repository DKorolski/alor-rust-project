//! Admission checks for the opt-in supervised worker. No new model or persisted
//! transaction schema: checks run BEFORE prospective model advancement.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServicePolicy {
    pub startup_timeout_seconds: u64,
    pub poll_timeout_seconds: u64,
    pub poll_interval_seconds: u64,
    pub shutdown_timeout_seconds: u64,
    pub max_queue_age_seconds: u64,
    pub max_model_lateness_seconds: u64,
    pub max_sessions: usize,
}
impl ServicePolicy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=900).contains(&self.startup_timeout_seconds)
                && (1..=120).contains(&self.poll_timeout_seconds)
                && (1..=30).contains(&self.poll_interval_seconds)
                && (1..=30).contains(&self.shutdown_timeout_seconds)
                && (1..=120).contains(&self.max_queue_age_seconds)
                && (1..=300).contains(&self.max_model_lateness_seconds)
                && (1..=20).contains(&self.max_sessions),
            "invalid bounded shadow service policy"
        );
        ensure!(
            self.poll_interval_seconds < self.max_queue_age_seconds
                && self.max_queue_age_seconds <= self.max_model_lateness_seconds,
            "poll/queue/model age budgets are inconsistent"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchSpec {
    pub runner: RunnerSpec,
    pub policy: ServicePolicy,
}
impl LaunchSpec {
    pub fn validate(&self) -> Result<()> {
        self.runner.header()?;
        self.policy.validate()
    }
}

#[derive(Debug, Serialize)]
pub struct ServiceStatus {
    pub runner: RunnerStatus,
    pub ingress: IngressUsage,
    pub generation: String,
    pub source_cursor: Option<String>,
    pub head_sha256: String,
    pub last_complete_session: Option<NaiveDate>,
    pub started_sessions: usize,
    pub armed: bool,
    pub completed: bool,
    pub resource_state: &'static str,
    pub max_unobserved_queue_age_seconds: i64,
    pub max_unobserved_model_lateness_seconds: i64,
    pub order_emission_allowed: bool,
}

pub struct ServiceGuard {
    policy: ServicePolicy,
    ready_by_utc: i64,
    armed: bool,
}
impl ServiceGuard {
    pub fn new(
        runner: &ReactionShadowRunner,
        policy: ServicePolicy,
        ready_by_utc: i64,
    ) -> Result<Self> {
        policy.validate()?;
        let now = runner.collector.journal.now()?;
        let identity = &runner.header.identities[0];
        let status = runner.status();
        ensure!(!status.failed, "failed generation cannot start");
        ensure!(
            status.session_date.is_none() || status.session_complete,
            "service resume requires complete paired EOD; intraday recovery is audit-only"
        );
        let cutoff = match status.session_date {
            Some(day) => identity.next_cutoff(day)?,
            None => identity.cutoff(runner.spec.first_decision_session),
        };
        ensure!(
            now < ready_by_utc && ready_by_utc <= cutoff,
            "readiness deadline must be future and not after the next decision cutoff"
        );
        ensure!(
            runner.state.stores[0].frozen.len() < policy.max_sessions,
            "completed generation cannot resume"
        );
        Ok(Self {
            policy,
            ready_by_utc,
            armed: false,
        })
    }

    fn queue_ages(&self, runner: &ReactionShadowRunner, now: i64) -> Result<(i64, i64)> {
        if runner.state.observed == runner.collector.journal.last_sequence() {
            return Ok((0, 0));
        }
        let identity = &runner.header.identities[0];
        let mut ages = (0, 0);
        for receipt in runner
            .collector
            .prefix(runner.collector.journal.last_sequence())?
            .iter()
            .skip(runner.state.observed as usize)
        {
            let bar = &receipt.bar;
            let dt = local(bar.close_time_utc)?;
            if bar.origin != DataOrigin::Live
                || dt.date() < runner.spec.first_decision_session
                || !identity.trading_date(dt.date())
                || bar.close_time_utc < identity.cutoff(dt.date())
                || dt.time() > chrono::NaiveTime::from_hms_opt(23, 40, 0).unwrap()
                || runner
                    .state
                    .model_inputs
                    .get(&bar.close_time_utc)
                    .is_some_and(|old| old.bar == *bar)
            {
                continue;
            }
            let received = receipt
                .received_at_utc
                .context("missing original receipt time")?;
            ages.0 = ages.0.max(now - received);
            ages.1 = ages.1.max(now - bar.close_time_utc - 600);
        }
        Ok(ages)
    }

    fn check_before_advance(&self, runner: &ReactionShadowRunner, now: i64) -> Result<()> {
        if !self.armed {
            ensure!(
                now < self.ready_by_utc,
                "startup missed explicit readiness deadline"
            );
        }
        let status = self.status(runner)?;
        ensure!(
            status.resource_state != "maintenance_required",
            "storage reached maintenance threshold; retain and audit"
        );
        ensure!(
            status.started_sessions < self.policy.max_sessions || !status.runner.session_complete,
            "generation session budget exhausted"
        );
        ensure!(
            status.max_unobserved_queue_age_seconds <= self.policy.max_queue_age_seconds as i64,
            "model receipt queue age exceeded; no retrospective observations"
        );
        ensure!(
            status.max_unobserved_model_lateness_seconds
                <= self.policy.max_model_lateness_seconds as i64,
            "closed-bar source lateness exceeded; no retrospective observations"
        );
        Ok(())
    }

    fn check_after_advance(&self, runner: &ReactionShadowRunner, now: i64) -> Result<()> {
        let identity = &runner.header.identities[0];
        if let Some(day) = runner.state.day {
            ensure!(
                runner.state.sessions.is_some(),
                "paired anchors not ready at decision cutoff"
            );
            if !runner.status().session_complete {
                let next_label = runner
                    .state
                    .model_inputs
                    .last_key_value()
                    .map_or(identity.cutoff(day), |(ts, _)| ts + 600);
                ensure!(
                    now <= next_label + 600 + self.policy.max_model_lateness_seconds as i64,
                    "expected live M10/EOD bar missing past deadline"
                );
            } else {
                ensure!(
                    now <= identity.next_cutoff(day)?
                        + self.policy.max_model_lateness_seconds as i64,
                    "next decision cutoff missed while catching up source"
                );
            }
        } else if local(now)?.date() >= runner.spec.first_decision_session {
            ensure!(
                now <= identity.cutoff(runner.spec.first_decision_session)
                    + self.policy.max_model_lateness_seconds as i64,
                "first decision cutoff not reached; source is not ready"
            );
        }
        Ok(())
    }

    pub fn status(&self, runner: &ReactionShadowRunner) -> Result<ServiceStatus> {
        let status = runner.status();
        let ingress = runner.collector.journal.usage();
        let (queue_age, model_age) = self.queue_ages(runner, runner.collector.journal.now()?)?;
        let started = runner.state.stores[0].frozen.len();
        let last_complete = if status.session_complete {
            status.session_date
        } else {
            // Each transition verifies both previous numerical sessions complete.
            runner.state.stores[0].frozen.keys().rev().nth(1).copied()
        };
        let usage = [
            (ingress.records as u64, ingress.max_records as u64),
            (ingress.barriers as u64, ingress.max_barriers as u64),
            (ingress.committed_bytes, ingress.max_bytes),
            (
                status.committed_transactions,
                storage::MAX_TRANSACTIONS as u64,
            ),
            (status.transaction_bytes, storage::MAX_TOTAL),
        ];
        let completed = started == self.policy.max_sessions && status.session_complete;
        Ok(ServiceStatus {
            runner: status,
            ingress,
            generation: runner.spec.ingress.generation.clone(),
            source_cursor: runner.collector.journal.last_source_id().map(str::to_owned),
            head_sha256: runner.log.head().into(),
            last_complete_session: last_complete,
            started_sessions: started,
            armed: self.armed,
            completed,
            resource_state: resource_state(&usage),
            max_unobserved_queue_age_seconds: queue_age,
            max_unobserved_model_lateness_seconds: model_age,
            order_emission_allowed: false,
        })
    }

    pub async fn poll_once(
        &mut self,
        runner: &mut ReactionShadowRunner,
        connection: &mut MultiplexedConnection,
    ) -> Result<ServiceStatus> {
        runner.log.healthy()?;
        let result = async {
            self.check_before_advance(runner, runner.collector.journal.now()?)?;
            runner.collector.poll_once(connection).await?;
            // Redis receipts are durable evidence even when too old to drive models.
            self.check_before_advance(runner, runner.collector.journal.now()?)?;
            runner.advance()?;
            let now = runner.collector.journal.now()?;
            self.check_after_advance(runner, now)?;
            if !self.armed {
                ensure!(
                    now < self.ready_by_utc,
                    "catch-up missed readiness deadline"
                );
                self.armed = runner.status().source_caught_up;
            }
            self.status(runner)
        }
        .await;
        result.map_err(|e| runner.log.fail(e))
    }
}

fn resource_state(usage: &[(u64, u64)]) -> &'static str {
    if usage
        .iter()
        .any(|(used, cap)| used.saturating_mul(100) >= cap.saturating_mul(80))
    {
        "maintenance_required"
    } else if usage
        .iter()
        .any(|(used, cap)| used.saturating_mul(100) >= cap.saturating_mul(60))
    {
        "warning"
    } else {
        "normal"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaction_service_resource_thresholds_cover_every_bound() {
        assert_eq!(resource_state(&[(59, 100)]), "normal");
        assert_eq!(resource_state(&[(1, 100), (60, 100)]), "warning");
        assert_eq!(resource_state(&[(79, 100)]), "warning");
        assert_eq!(
            resource_state(&[(1, 100), (80, 100)]),
            "maintenance_required"
        );
        assert_eq!(resource_state(&[(101, 100)]), "maintenance_required");
    }
}
