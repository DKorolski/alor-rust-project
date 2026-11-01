use std::collections::BTreeMap;
use std::io::Write;

use anyhow::{ensure, Result};
use chrono::{Duration, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::model::{time, transition, Action, Bar, Engine};

pub const SCHEMA: &str = "imoexf_reaction_state_v1";
pub const REFERENCE: &str = "a469365d54844879ed63108bc99d45be18ace95d261627b57a94ceb1a4c18135";
pub const CONTRACT: &str = "2f9afa7977c8170ce9b8ed5ce09fb3a303c43800aaefee8949947edb6974ac85";
pub const COSTS: &str = "04154c0b8e613aa090db304273b4c575b1b69e48983d0d011397de5e3ebf362d";

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
                    .map(|(k, v)| (k.clone(), sorted(v)))
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

pub fn identity(sleeve: &str) -> Value {
    json!({"model_id":format!("imoexf_author41_short_reaction07_v1/{sleeve}"), "symbol":"IMOEXF",
        "feed_policy":"frozen_regular_weekday_transition_09_to_07",
        "context_policy":"previous_weekday_reference_history", "execution_policy":"reaction_next_open",
        "config_sha256":CONTRACT,"reference_sha256":REFERENCE,"timeframe":"10m",
        "timezone":"Europe/Moscow","evidence_kind":"numerical_history"})
}

fn decision(a: &Action) -> String {
    hash_json(&json!({"owner":a.owner,"side":a.side,"role":a.role(),
        "signal_ts_local":stamp(a.signal),"action_ts_local":stamp(a.at)}))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub input_id: String,
    pub input_sha256: String,
    pub bar_ts_local: String,
}

pub fn verify_inputs(
    manifest: &Value,
    bars: &[Bar],
    raw_hash: &str,
    sleeve: &str,
    reference_manifest_hash: &str,
) -> Result<Vec<Input>> {
    ensure!(
        manifest["bundle_version"] == "shadow_state_bundle_v1"
            && manifest["trace_schema"] == SCHEMA,
        "unsupported reference state manifest"
    );
    ensure!(
        manifest["identity"] == identity(sleeve),
        "identity mismatch"
    );
    ensure!(
        manifest["model_input_file_sha256"] == raw_hash,
        "model input file hash mismatch"
    );
    ensure!(
        manifest["reference_manifest_sha256"] == reference_manifest_hash,
        "reference manifest mismatch"
    );
    let inputs: Vec<Input> = serde_json::from_value(manifest["inputs"].clone())?;
    ensure!(
        hash_json(&serde_json::to_value(&inputs)?) == manifest["inputs_sha256"],
        "input manifest hash mismatch"
    );
    ensure!(inputs.len() == bars.len(), "input coverage mismatch");
    for (bar, input) in bars.iter().zip(&inputs) {
        let label = stamp(bar.ts_local);
        ensure!(
            input.bar_ts_local == label && input.input_id == format!("IMOEXF|10m|{label}"),
            "input identity mismatch"
        );
        ensure!(
            input.input_sha256.len() == 64
                && input.input_sha256.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid input hash"
        );
    }
    // Metadata only: float payloads are bound by the verified source file hash.
    // No reference states, features, decisions or prices are used by the model.
    Ok(inputs)
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
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
    pub fn new(writer: W, sleeve: &str, inputs: Vec<Input>) -> Self {
        let mut daily_inputs: BTreeMap<String, Vec<Input>> = BTreeMap::new();
        for input in &inputs {
            daily_inputs
                .entry(input.bar_ts_local[..10].to_string())
                .or_default()
                .push(input.clone());
        }
        Self {
            writer,
            identity: identity(sleeve),
            inputs: inputs
                .into_iter()
                .map(|i| (i.bar_ts_local.clone(), i))
                .collect(),
            daily_inputs,
            provenance_cache: BTreeMap::new(),
            rows: 0,
        }
    }

    pub fn emit(&mut self, phase: &str, e: &Engine, b: &Bar) -> Result<()> {
        let obs = e.observation.as_ref().unwrap();
        let (pc, pr, pl, ret) = e.context();
        let previous = e.previous.as_ref().map(|p| {
            json!({"session_date":p.first.date().to_string(),
            "high":p.high,"low":p.low,"close":p.close,"range":p.range(),"bar_count":p.count})
        });
        let previous2 = e
            .previous2
            .as_ref()
            .map(|p| json!({"session_date":p.first.date().to_string(),"close":p.close}));
        let dates: Vec<_> = [e.previous.as_ref(), e.previous2.as_ref()]
            .into_iter()
            .flatten()
            .map(|p| p.first.date().to_string())
            .collect();
        let provenance = self
            .provenance_cache
            .entry(dates.clone())
            .or_insert_with(|| {
                let inputs: Vec<_> = dates
                    .iter()
                    .flat_map(|d| self.daily_inputs[d].iter())
                    .collect();
                hash_json(&serde_json::to_value(inputs).expect("input metadata"))
            })
            .clone();
        let position = e.position.as_ref().map(|p| {
            json!({"owner":p.entry.owner,"side":p.entry.side,
            "entry_ts_local":stamp(p.entry.at),"entry_signal_ts_local":stamp(p.entry.signal),
            "entry_price":p.price,"bars_held":p.bars_held,"entry_decision_id":decision(&p.entry),
            "entry_proxy_known_ts_local":stamp(p.entry.at+Duration::minutes(10))})
        });
        let pending = e.pending.as_ref().map(|p| {
            json!({"role":p.role(),"owner":p.owner,"side":p.side,
            "signal_ts_local":stamp(p.signal),"action_ts_local":stamp(p.at),"reason":p.reason(),
            "decision_id":decision(p),"proxy_known":false})
        });
        let suppression: Vec<_> =
            if matches!(e.branch, "not_evaluated" | "mr_selected" | "bo_selected") {
                vec![]
            } else {
                vec![e.branch]
            };
        let (mr, long, short) = e.setups(b);
        let input = &self.inputs[&stamp(b.ts_local)];
        let cutoff = if b.ts_local.date() < transition() {
            9
        } else {
            7
        };
        let row = json!({"schema_version":SCHEMA,"identity":self.identity,"event_seq":self.rows,
            "input_id":input.input_id,"input_sha256":input.input_sha256,"phase":phase,
            "timing":{"bar_ts_local":stamp(b.ts_local),"decision_ready_ts_local":stamp(b.ts_local+Duration::minutes(10)),
                "received_ts_utc":null,"processed_ts_utc":null,"context_cutoff_ts_local":stamp(b.ts_local.date().and_time(time(cutoff,0)))},
            "context":{"status":if e.valid() {"ready"} else {"not_ready"},"previous":previous,"previous2":previous2,
                "scale":if e.valid() {finite(pr/pl)} else {None},"previous_return":finite(ret),
                "provenance_sha256":provenance,"completeness":"reference_input_only_not_operationally_certified"},
            "counters":{"session_date":b.ts_local.date().to_string(),"mr_total":e.mr_entries,"mr_limit":2,
                "bo_total":usize::from(e.used_long)+usize::from(e.used_short),"bo_long_used":e.used_long,"bo_short_used":e.used_short},
            "bo_observation":{"count":obs.count,"first_ts_local":(obs.count>0).then(||stamp(obs.first)),
                "last_ts_local":(obs.count>0).then(||stamp(obs.last)),"day_high":finite(obs.high),"day_low":finite(obs.low)},
            "position":position,"pending":pending,
            "levels":{"mr_entry_upper":finite(pc+0.16*pr),"mr_take_close_below":finite(pc-0.020*pr),
                "mr_stop_touch_above":finite(pc+0.58*pr),"bo_entry_long":finite(pc+0.53*pr),"bo_entry_short":finite(pc-0.53*pr),
                "bo_stop1_long":finite(pc+0.51*pr),"bo_stop1_short":finite(pc-0.51*pr),
                "bo_stop2_long":finite(pc-0.35*pr),"bo_stop2_short":finite(pc+0.35*pr)},
            "signals":{"branch":e.branch,"mr_price_setup":mr,"bo_long_price_setup":long,"bo_short_price_setup":short,
                "suppression_reasons":suppression}});
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
