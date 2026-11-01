//! Read-only Redis polling for the isolated shadow receipt journal. Deliberately
//! no XACK, group creation, XADD, source trim or broker-command transport.
use alor_protocol::{Envelope, MessageType, SCHEMA_VERSION};
use redis::{aio::MultiplexedConnection, streams::StreamRangeReply};

use super::*;

const BATCH: usize = 128;

/// One serialized polling boundary shared by the matched context arms. The
/// journal owns the durable resume cursor; no separate volatile cursor is trusted.
pub struct RiRedisSource {
    identity: IngressIdentity,
    last_success: Option<(i64, u64)>,
    initial_first_id: Option<String>,
}

impl RiRedisSource {
    pub fn caught_up(&self, journal: &RiIngressJournal) -> bool {
        journal.identity() == &self.identity
            && self
                .last_success
                .is_some_and(|(_, seq)| seq == journal.last_sequence())
    }

    pub fn new(identity: IngressIdentity) -> Result<Self> {
        identity.validate()?;
        Ok(Self {
            identity,
            last_success: None,
            initial_first_id: None,
        })
    }

    // Only the opt-in reaction bootstrap uses an inclusive initial boundary.
    // Once a receipt exists, the durable journal cursor always takes precedence.
    pub(crate) fn start_at(&mut self, first_id: &str) -> Result<()> {
        ensure!(stream_id(first_id)? != (0, 0), "invalid bootstrap boundary");
        ensure!(
            self.initial_first_id.is_none(),
            "source start already configured"
        );
        self.initial_first_id = Some(first_id.into());
        Ok(())
    }

    /// Drain one bounded batch. A failure invalidates readiness until a successful
    /// cursor-checked retry. A missing/trimmed cursor cannot silently jump forward.
    pub async fn poll(
        &mut self,
        connection: &mut MultiplexedConnection,
        journal: &mut RiIngressJournal,
    ) -> Result<Vec<AppendReceipt>> {
        self.last_success = None;
        ensure!(
            journal.identity() == &self.identity,
            "Redis source/journal identity mismatch"
        );
        journal.now()?;
        let initial = journal.last_source_id().is_none() && self.initial_first_id.is_some();
        let cursor = journal
            .last_source_id()
            .or(self.initial_first_id.as_deref())
            .unwrap_or("0-0")
            .to_string();
        let parsed_cursor = stream_id(&cursor)?;
        // Atomic read transaction prevents trim between the resume-cursor check
        // and the fetch. It contains ONLY XRANGE commands and does not mutate Redis.
        let mut pipe = redis::pipe();
        pipe.atomic()
            .cmd("XRANGE")
            .arg(&self.identity.source_stream)
            .arg(&cursor)
            .arg(&cursor)
            .arg("COUNT")
            .arg(1)
            .cmd("XRANGE")
            .arg(&self.identity.source_stream)
            .arg(if initial {
                cursor.clone()
            } else {
                format!("({cursor}")
            })
            .arg("+")
            .arg("COUNT")
            .arg(BATCH);
        let (previous, batch): (StreamRangeReply, StreamRangeReply) =
            pipe.query_async(connection).await?;
        if initial {
            ensure!(
                previous.ids.len() == 1
                    && previous.ids[0].id == cursor
                    && batch.ids.first().is_some_and(|row| row.id == cursor),
                "bootstrap first source row missing/trimmed"
            );
        } else if parsed_cursor != (0, 0) {
            ensure!(
                previous.ids.len() == 1 && previous.ids[0].id == cursor,
                "Redis resume cursor missing/trimmed; source coverage is unknown"
            );
            let previous_payload = bar_payload(&previous.ids[0])?;
            ensure!(
                journal
                    .rows
                    .last()
                    .context("missing durable cursor")?
                    .raw
                    .raw_bar_json
                    == previous_payload,
                "Redis resume payload changed; possible stream recreation"
            );
        }
        let caught_up = batch.ids.len() < BATCH;
        let mut receipts = Vec::with_capacity(batch.ids.len());
        let mut previous_id = if initial { (0, 0) } else { parsed_cursor };
        for row in batch.ids {
            let parsed_id = stream_id(&row.id)?;
            ensure!(
                parsed_id > previous_id,
                "Redis source IDs are not increasing"
            );
            previous_id = parsed_id;
            let payload = bar_payload(&row)?;
            let receipt = journal.append(&row.id, &payload)?;
            ensure!(
                receipt.input.is_ok(),
                "malformed Redis bar retained; shadow collector blocked"
            );
            receipts.push(receipt);
        }
        if caught_up {
            self.last_success = Some((journal.now()?, journal.last_sequence()));
        }
        Ok(receipts)
    }

    /// This remains a LOCAL-receipt cutoff, not proof of exchange completeness.
    /// Require a successful poll after cutoff, and no out-of-band journal writes.
    pub fn seal_cutoff(
        &self,
        journal: &mut RiIngressJournal,
        cutoff: i64,
    ) -> Result<ReceiptBarrier> {
        ensure!(
            journal.identity() == &self.identity,
            "Redis source/journal identity mismatch"
        );
        let (polled, sequence) = self.last_success.context("Redis collector is not ready")?;
        ensure!(
            polled >= cutoff && sequence == journal.last_sequence(),
            "cutoff requires a fresh source poll"
        );
        journal.seal_cutoff(cutoff)
    }
}

pub(crate) fn bar_payload(row: &redis::streams::StreamId) -> Result<String> {
    let raw = match row
        .map
        .get("payload")
        .map(redis::from_redis_value::<String>)
    {
        Some(Ok(raw)) => raw,
        _ => format!("invalid Redis bar entry: {row:?}"),
    };
    // Preserve malformed envelopes in audit rather than silently skipping them.
    // Valid payload JSON is normalized; envelope publication time is not receipt.
    Ok(
        match serde_json::from_str::<Envelope<serde_json::Value>>(&raw) {
            Ok(e) if e.msg_type == MessageType::Bar && e.schema_version == SCHEMA_VERSION => {
                serde_json::to_string(&e.payload)?
            }
            _ => format!("invalid Redis envelope: {raw}"),
        },
    )
}

#[cfg(test)]
pub(crate) mod tests;

pub(crate) fn stream_id(raw: &str) -> Result<(u64, u64)> {
    let (millis, sequence) = raw.split_once('-').context("invalid Redis cursor")?;
    let result = (millis.parse::<u64>()?, sequence.parse::<u64>()?);
    ensure!(
        format!("{}-{}", result.0, result.1) == raw,
        "non-canonical Redis cursor"
    );
    Ok(result)
}
