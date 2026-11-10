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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ordinary_model_lateness_seconds: Option<u64>,
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
        if let Some(limit) = self.max_ordinary_model_lateness_seconds {
            ensure!(
                (self.max_model_lateness_seconds..=360).contains(&limit),
                "invalid bounded ordinary model lateness"
            );
        }
        Ok(())
    }

    fn ordinary_lateness_seconds(&self) -> u64 {
        self.max_ordinary_model_lateness_seconds
            .unwrap_or(self.max_model_lateness_seconds)
    }

    pub(super) fn lateness_for_label(&self, label: i64) -> Result<u64> {
        Ok(
            if local(label)?.time() == chrono::NaiveTime::from_hms_opt(23, 40, 0).unwrap() {
                self.max_model_lateness_seconds
            } else {
                self.ordinary_lateness_seconds()
            },
        )
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
    pub max_unobserved_ordinary_lateness_seconds: i64,
    pub max_unobserved_terminal_lateness_seconds: i64,
    pub ordinary_model_lateness_limit_seconds: u64,
    pub terminal_model_lateness_limit_seconds: u64,
    pub next_guard_deadline_utc: Option<i64>,
    pub order_emission_allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_late_start: Option<late_start::OutputQuality>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clean_sessions_completed: Option<usize>,
}

pub struct ServiceGuard {
    policy: ServicePolicy,
    ready_by_utc: i64,
    armed: bool,
    collected_prefix: Option<u64>,
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
        let initial_diagnostic = runner.spec.late_start.is_some() && status.session_date.is_none();
        let allowed_deadline = if initial_diagnostic {
            (runner
                .spec
                .first_decision_session
                .and_hms_opt(23, 40, 0)
                .unwrap()
                - Duration::hours(3))
            .and_utc()
            .timestamp()
        } else {
            cutoff
        };
        ensure!(
            now < ready_by_utc && ready_by_utc <= allowed_deadline,
            "readiness deadline must be future and not after the next decision cutoff"
        );
        ensure!(
            runner.state.stores[0]
                .frozen
                .keys()
                .filter(|d| runner
                    .spec
                    .late_start
                    .as_ref()
                    .is_none_or(|l| **d >= l.first_clean_session))
                .count()
                < policy.max_sessions,
            "completed generation cannot resume"
        );
        Ok(Self {
            policy,
            ready_by_utc,
            armed: false,
            collected_prefix: None,
        })
    }

    fn queue_ages(
        &self,
        runner: &ReactionShadowRunner,
        now: i64,
    ) -> Result<(i64, i64, i64, Option<i64>)> {
        if runner.state.observed == runner.collector.journal.last_sequence() {
            return Ok((0, 0, 0, None));
        }
        let identity = &runner.header.identities[0];
        let mut ages = (0, 0, 0, None::<i64>);
        for receipt in runner
            .collector
            .prefix(runner.collector.journal.last_sequence())?
            .iter()
            .skip(runner.state.observed as usize)
        {
            let bar = &receipt.bar;
            if runner
                .spec
                .late_start
                .as_ref()
                .is_some_and(|late| receipt.sequence <= late.recovery.rows)
            {
                continue;
            }
            let dt = local(bar.close_time_utc)?;
            let diagnostic_history = bar.origin == DataOrigin::History
                && runner.spec.late_start.as_ref().is_some_and(|late| {
                    dt.date() == runner.spec.first_decision_session
                        && dt.date() < late.source_first_live_session
                });
            if (bar.origin != DataOrigin::Live && !diagnostic_history)
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
            let model_age = now - bar.close_time_utc - 600;
            let deadline = (received + self.policy.max_queue_age_seconds as i64).min(
                bar.close_time_utc
                    + 600
                    + self.policy.lateness_for_label(bar.close_time_utc)? as i64,
            );
            ages.3 = Some(ages.3.map_or(deadline, |old| old.min(deadline)));
            if dt.time() == chrono::NaiveTime::from_hms_opt(23, 40, 0).unwrap() {
                ages.2 = ages.2.max(model_age);
            } else {
                ages.1 = ages.1.max(model_age);
            }
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
        self.check_model_ages(&status)
    }

    pub(super) fn check_model_ages(&self, status: &ServiceStatus) -> Result<()> {
        ensure!(
            status.max_unobserved_queue_age_seconds <= self.policy.max_queue_age_seconds as i64,
            "model receipt queue age exceeded; no retrospective observations"
        );
        ensure!(
            status.max_unobserved_ordinary_lateness_seconds
                <= self.policy.ordinary_lateness_seconds() as i64
                && status.max_unobserved_terminal_lateness_seconds
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
                    now <= next_label + 600 + self.policy.lateness_for_label(next_label)? as i64,
                    "expected live M10/EOD bar missing past deadline"
                );
            } else {
                ensure!(
                    now <= identity.next_cutoff(day)?
                        + self.policy.max_model_lateness_seconds as i64,
                    "next decision cutoff missed while catching up source"
                );
            }
        } else if local(now)?.date() >= runner.spec.first_decision_session
            && runner.spec.late_start.is_none()
        {
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
        let (queue_age, ordinary_age, terminal_age, queue_deadline) =
            self.queue_ages(runner, runner.collector.journal.now()?)?;
        let identity = &runner.header.identities[0];
        let mut deadline = if !self.armed {
            Some(self.ready_by_utc)
        } else if let Some(day) = runner.state.day {
            Some(if status.session_complete {
                identity.next_cutoff(day)? + self.policy.max_model_lateness_seconds as i64
            } else {
                let next = runner
                    .state
                    .model_inputs
                    .last_key_value()
                    .map_or(identity.cutoff(day), |(ts, _)| ts + 600);
                next + 600 + self.policy.lateness_for_label(next)? as i64
            })
        } else {
            Some(
                identity.cutoff(runner.spec.first_decision_session)
                    + self.policy.max_model_lateness_seconds as i64,
            )
        };
        if let Some(queued) = queue_deadline {
            deadline = Some(deadline.map_or(queued, |old| old.min(queued)));
        }
        let clean = |day: NaiveDate| {
            runner
                .spec
                .late_start
                .as_ref()
                .is_none_or(|l| day >= l.first_clean_session)
        };
        let started = runner.state.stores[0]
            .frozen
            .keys()
            .filter(|d| clean(**d))
            .count();
        let last_complete = (if status.session_complete {
            status.session_date
        } else {
            // Each transition verifies both previous numerical sessions complete.
            runner.state.stores[0].frozen.keys().rev().nth(1).copied()
        })
        .filter(|d| clean(*d));
        let completed_clean = runner.state.stores[0]
            .frozen
            .keys()
            .filter(|d| clean(**d) && (Some(**d) != status.session_date || status.session_complete))
            .count();
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
        let diagnostic_late_start =
            runner
                .spec
                .late_start
                .as_ref()
                .map(|late| late_start::OutputQuality {
                    diagnostic_session: runner.spec.first_decision_session,
                    first_clean_session: late.first_clean_session,
                    recovery_through_sequence: late.recovery.rows,
                    current_session_clean_eligible: status.session_date.is_some_and(clean),
                });
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
            max_unobserved_model_lateness_seconds: ordinary_age.max(terminal_age),
            max_unobserved_ordinary_lateness_seconds: ordinary_age,
            max_unobserved_terminal_lateness_seconds: terminal_age,
            ordinary_model_lateness_limit_seconds: self.policy.ordinary_lateness_seconds(),
            terminal_model_lateness_limit_seconds: self.policy.max_model_lateness_seconds,
            next_guard_deadline_utc: deadline,
            order_emission_allowed: false,
            diagnostic_late_start,
            clean_sessions_completed: runner.spec.late_start.as_ref().map(|_| completed_clean),
        })
    }

    pub async fn poll_once(
        &mut self,
        runner: &mut ReactionShadowRunner,
        connection: &mut MultiplexedConnection,
    ) -> Result<ServiceStatus> {
        self.collect_once(runner, connection).await?;
        self.advance_collected(runner)
    }

    /// Durable ingress only. The supervisor must admit this fixed prefix again
    /// before any model advancement; no Redis await exists in advance_collected.
    pub async fn collect_once(
        &mut self,
        runner: &mut ReactionShadowRunner,
        connection: &mut MultiplexedConnection,
    ) -> Result<ServiceStatus> {
        runner.log.healthy()?;
        let result = async {
            ensure!(
                self.collected_prefix
                    .is_none_or(|p| p == runner.collector.journal.last_sequence()),
                "collected prefix changed before revalidation"
            );
            self.check_before_advance(runner, runner.collector.journal.now()?)?;
            runner.collector.poll_once(connection).await?;
            self.collected_prefix = Some(runner.collector.journal.last_sequence());
            self.status(runner)
        }
        .await;
        result.map_err(|e| runner.log.fail(e))
    }

    pub fn advance_collected(
        &mut self,
        runner: &mut ReactionShadowRunner,
    ) -> Result<ServiceStatus> {
        runner.log.healthy()?;
        let result = (|| {
            let prefix = self
                .collected_prefix
                .context("model advance without collected prefix")?;
            ensure!(
                prefix == runner.collector.journal.last_sequence(),
                "collected prefix changed"
            );
            self.check_before_advance(runner, runner.collector.journal.now()?)?;
            let now = runner.collector.journal.now()?;
            let date = local(now)?.date();
            let identity = &runner.header.identities[0];
            if date >= runner.spec.first_decision_session
                && identity.trading_date(date)
                && now >= identity.cutoff(date)
                && runner.state.day != Some(date)
                && !runner
                    .collector
                    .source
                    .cutoff_poll_ready(&runner.collector.journal, identity.cutoff(date))
            {
                // Collection may straddle opening. A new read is required;
                // never fabricate a post-cutoff poll or fail a healthy prefix.
                self.collected_prefix = None;
                return self.status(runner);
            }
            runner.advance()?;
            let now = runner.collector.journal.now()?;
            self.check_after_advance(runner, now)?;
            if !self.armed {
                ensure!(
                    now < self.ready_by_utc,
                    "catch-up missed readiness deadline"
                );
                let ready = runner.status();
                self.armed = ready.source_caught_up
                    && (runner.spec.late_start.is_none() || ready.pair_ready);
            }
            self.collected_prefix = None;
            self.status(runner)
        })();
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
