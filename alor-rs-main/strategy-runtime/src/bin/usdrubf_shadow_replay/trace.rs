use std::collections::BTreeMap;
use std::io::Write;

use anyhow::{ensure, Result};
use chrono::{Duration, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::model::{time, Bar, Component, Config, Engine, Family, Pending};

pub const SCHEMA: &str = "usdrubf_reaction_state_v1";
pub const REFERENCE: &str = "3c0546815af66802986bc30e224d4aaae78443464dc4eba0bb5191e62dcd4d20";
pub const CONTRACT: &str = "91d182b04f813505f2ae6259039bfd8b22c8197084a6698eac399d9c79ce5a5d";

pub fn stamp(ts: NaiveDateTime) -> String {
    ts.format("%Y-%m-%dT%H:%M:%S").to_string()
}
pub fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let ordered: BTreeMap<_, _> = fields.iter().collect();
            Value::Object(
                ordered
                    .into_iter()
                    .map(|(key, value)| (key.clone(), sorted(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.iter().map(sorted).collect()),
        _ => value.clone(),
    }
}

pub fn hash_json(value: &Value) -> String {
    sha(&serde_json::to_vec(&sorted(value)).expect("JSON serialization"))
}

pub fn identity(cfg: &Config, config_hash: &str) -> Value {
    json!({"model_id":cfg.model_id,"symbol":"USDRUBF","timeframe":"10m","timezone":"Europe/Moscow",
        "evidence_kind":"numerical_history", "feed_policy":format!("regular_weekdays_transition09_to{:02}_2349",cfg.context_hour),
        "context_policy":format!("{}_weekday_only",cfg.family.name()), "execution_policy":"closed_bar_next_open_market_v1",
        "config_sha256":config_hash,"reference_sha256":REFERENCE})
}

fn decision(
    owner: Component,
    side: &str,
    role: &str,
    signal: NaiveDateTime,
    action: NaiveDateTime,
) -> String {
    hash_json(
        &json!({"owner":owner,"side":side,"role":role,"signal_ts_local":stamp(signal),"action_ts_local":stamp(action)}),
    )
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub input_id: String,
    pub input_sha256: String,
    pub bar_ts_local: String,
}

pub fn verify_selection(manifest: &Value, raw: &[Bar], cfg: &Config) -> Result<Vec<Input>> {
    ensure!(
        manifest["bundle_version"] == "shadow_state_bundle_v1"
            && manifest["trace_schema"] == SCHEMA,
        "unsupported manifest"
    );
    ensure!(
        manifest["frozen_contract_sha256"] == CONTRACT,
        "contract mismatch"
    );
    let inputs: Vec<Input> = serde_json::from_value(manifest["inputs"].clone())?;
    let excluded: Vec<Input> = serde_json::from_value(manifest["excluded_inputs"].clone())?;
    ensure!(
        hash_json(&serde_json::to_value(&inputs)?) == manifest["inputs_sha256"],
        "input manifest hash mismatch"
    );
    ensure!(
        hash_json(&serde_json::to_value(&excluded)?) == manifest["excluded_inputs_sha256"],
        "excluded manifest hash mismatch"
    );
    ensure!(
        manifest["raw_input_count"] == raw.len(),
        "raw input count mismatch"
    );
    for (expected, eligible) in [(&inputs, true), (&excluded, false)] {
        let selected: Vec<_> = raw.iter().filter(|b| cfg.eligible(b) == eligible).collect();
        ensure!(
            selected.len() == expected.len(),
            "raw feed selection count mismatch"
        );
        for (bar, input) in selected.iter().zip(expected) {
            let label = stamp(bar.ts_local);
            ensure!(
                input.bar_ts_local == label && input.input_id == format!("USDRUBF|10m|{label}"),
                "raw feed selection identity mismatch"
            );
            ensure!(
                input.input_sha256.len() == 64
                    && input.input_sha256.bytes().all(|c| c.is_ascii_hexdigit()),
                "invalid input hash"
            );
        }
    }
    // Input hashes are bound by the separately verified raw FILE hash, not a cross-language float serializer.
    let mut all = inputs.clone();
    all.extend(excluded);
    all.sort_by(|a, b| a.bar_ts_local.cmp(&b.bar_ts_local));
    ensure!(
        hash_json(&serde_json::to_value(all)?) == manifest["raw_inputs_sha256"],
        "raw coverage hash mismatch"
    );
    Ok(inputs)
}

pub struct Trace<W: Write> {
    writer: W,
    identity: Value,
    inputs: BTreeMap<String, Input>,
    daily_inputs: BTreeMap<String, Vec<Input>>,
    provenance_cache: BTreeMap<Vec<String>, String>,
    pub rows: usize,
}

impl<W: Write> Trace<W> {
    pub fn new(writer: W, identity: Value, inputs: Vec<Input>) -> Self {
        let mut daily_inputs: BTreeMap<String, Vec<Input>> = BTreeMap::new();
        for input in &inputs {
            daily_inputs
                .entry(input.bar_ts_local[..10].to_string())
                .or_default()
                .push(input.clone());
        }
        Self {
            writer,
            identity,
            inputs: inputs
                .into_iter()
                .map(|i| (i.bar_ts_local.clone(), i))
                .collect(),
            daily_inputs,
            provenance_cache: BTreeMap::new(),
            rows: 0,
        }
    }

    pub fn emit(&mut self, phase: &str, e: &Engine, bar: &Bar) -> Result<()> {
        let m = e.metrics.as_ref().unwrap();
        let (mr, bo, scale, gate) = e.features();
        let previous = e.previous.as_ref().map(|p| {
            json!({"session_date":p.first.date().to_string(),
            "high":p.high,"low":p.low,"close":p.close,"range":p.range(),"bar_count":p.count})
        });
        let previous2 = e
            .previous2
            .as_ref()
            .map(|p| json!({"session_date":p.first.date().to_string(),"close":p.close}));
        let provenance_dates: Vec<_> = [e.previous2.as_ref(), e.previous.as_ref()]
            .into_iter()
            .flatten()
            .map(|p| p.first.date().to_string())
            .collect();
        let provenance_hash = self
            .provenance_cache
            .entry(provenance_dates.clone())
            .or_insert_with(|| {
                let provenance: Vec<_> = provenance_dates
                    .iter()
                    .map(|d| &self.daily_inputs[d])
                    .collect();
                hash_json(&serde_json::to_value(provenance).expect("input metadata serialization"))
            })
            .clone();
        let position = e.position.as_ref().map(|p| json!({
            "owner":p.entry.owner,"side":p.entry.side,"entry_ts_local":stamp(p.entry.action),
            "entry_signal_ts_local":stamp(p.entry.signal),"entry_price":p.price,
            "entry_decision_id":decision(p.entry.owner,p.entry.side.name(),"entry",p.entry.signal,p.entry.action),
            "entry_proxy_known_ts_local":stamp(p.entry.action+Duration::minutes(10)),
            "captured_scale":p.entry.scale,"captured_anchor":p.entry.anchor,"stop1":p.entry.stop1,"stop2":p.entry.stop2,
            "stop_price":p.stop,"take_price":p.take}));
        let pending = e.pending.as_ref().map(|p| match p {
            Pending::Entry(p) => json!({"role":"entry","owner":p.owner,"side":p.side,"signal_ts_local":stamp(p.signal),
                "action_ts_local":stamp(p.action),"reason":if p.owner == Component::MR {"mr_entry"} else {"bo_entry"},
                "decision_id":decision(p.owner,p.side.name(),"entry",p.signal,p.action),"proxy_known":false,
                "ambiguous_mr_bar":false,"captured_scale":p.scale,"captured_anchor":p.anchor,"stop1":p.stop1,"stop2":p.stop2}),
            Pending::Exit(p) => {
                let position = e.position.as_ref().expect("pending exit owns position");
                json!({"role":"exit","owner":position.entry.owner,"side":position.entry.side,
                    "signal_ts_local":stamp(p.signal),"action_ts_local":stamp(p.action),"reason":p.reason,
                    "decision_id":decision(position.entry.owner,position.entry.side.name(),"exit",p.signal,p.action),
                    "proxy_known":false,"ambiguous_mr_bar":p.ambiguous,"captured_scale":null,"captured_anchor":null,"stop1":null,"stop2":null})
            }
        });
        let suppression: Vec<_> =
            if matches!(e.branch, "not_evaluated" | "mr_selected" | "bo_selected") {
                vec![]
            } else {
                vec![e.branch]
            };
        let (mr_setup, long_setup, short_setup) = e.setups();
        let input = &self.inputs[&stamp(bar.ts_local)];
        let row = json!({"schema_version":SCHEMA,"identity":self.identity,"event_seq":self.rows,
            "input_id":input.input_id,"input_sha256":input.input_sha256,"phase":phase,
            "timing":{"bar_ts_local":stamp(bar.ts_local),"decision_ready_ts_local":stamp(bar.ts_local+Duration::minutes(10)),
                "context_cutoff_ts_local":stamp(bar.ts_local.date().and_time(time(e.clock(),0))),
                "received_ts_utc":null,"processed_ts_utc":null},
            "context":{"family":e.cfg.family.name(),"previous":previous,"previous2":previous2,
                "mr_anchor":mr,"bo_anchor":bo,"scale":scale,"gate_return":gate,"previous_return":e.prior_return(),
                "two_prior_sessions_available":e.previous2.is_some(),
                "warmup_return_fallback":e.cfg.family==Family::PreviousDay && e.prior_return().is_none(),
                "provenance_sha256":provenance_hash},
            "observation":{"count":m.count,"first_ts_local":stamp(m.first),"last_ts_local":stamp(m.last),
                "day_open":m.open,"running_range":m.range(),"vwap_numerator":m.numerator.sum,
                "cumulative_volume":m.volume.sum,"vwap":m.vwap()},
            "windows":{"session_open":time(e.clock(),0).to_string(),
                "mr_first_signal":time(e.clock()+e.cfg.mr_start_delay,0).to_string(),
                "mr_last_signal":time(e.clock()+e.cfg.mr_horizon-1,40).to_string(),"mr_force_signal":e.mr_force().to_string(),
                "bo_first_signal":time(e.clock()+e.cfg.bo_wait,0).to_string(),"bo_terminal_action":"23:30:00"},
            "counters":{"session_date":m.first.date().to_string(),"mr_scheduled_total":e.mr_scheduled,"mr_limit":null,
                "bo_total":usize::from(e.used_long)+usize::from(e.used_short),"bo_long_used":e.used_long,"bo_short_used":e.used_short},
            "position":position,"pending":pending,
            "signals":{"branch":e.branch,"mr_price_setup":mr_setup,"bo_long_price_setup":long_setup,
                "bo_short_price_setup":short_setup,"suppression_reasons":suppression}});
        serde_json::to_writer(&mut self.writer, &row)?;
        self.writer.write_all(b"\n")?;
        self.rows += 1;
        Ok(())
    }

    pub fn finish(&mut self) -> Result<()> {
        self.writer.flush()?;
        Ok(())
    }
}
