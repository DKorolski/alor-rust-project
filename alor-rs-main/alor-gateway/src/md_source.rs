//! Isolated one-instrument WS MD source. Never constructs a Supervisor or CWS.
pub mod config;
mod runner;
pub mod storage;

pub use runner::run;

pub const VERSION: &str = "shadow_gateway_ws_closed_m10_v2";
pub const MAX_ROWS: usize = 4500;
pub const MAX_AUDIT_ROWS: usize = 1024;

#[cfg(test)]
mod tests;

pub(crate) fn digest(value: &impl serde::Serialize) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

pub fn digest_for_error(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(crate) fn local(ts: i64) -> anyhow::Result<chrono::NaiveDateTime> {
    use anyhow::Context;
    chrono::DateTime::from_timestamp(ts, 0)
        .and_then(|d| d.naive_utc().checked_add_signed(chrono::Duration::hours(3)))
        .context("invalid source timestamp")
}
