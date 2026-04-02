use std::time::{Duration, Instant};

use chrono::Utc;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::{debug, info};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessTokenSource {
    Cached,
    Refreshed,
}

impl AccessTokenSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cached => "cached",
            Self::Refreshed => "refreshed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessTokenSnapshot {
    token: String,
    fingerprint: String,
    source: AccessTokenSource,
    refresh_count: u64,
    obtained_ts_utc: Option<i64>,
    age_ms: Option<u64>,
    ttl_remaining_ms: Option<u64>,
}

impl AccessTokenSnapshot {
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn source(&self) -> AccessTokenSource {
        self.source
    }

    pub fn refresh_count(&self) -> u64 {
        self.refresh_count
    }

    pub fn obtained_ts_utc(&self) -> Option<i64> {
        self.obtained_ts_utc
    }

    pub fn age_ms(&self) -> Option<u64> {
        self.age_ms
    }

    pub fn ttl_remaining_ms(&self) -> Option<u64> {
        self.ttl_remaining_ms
    }
}

#[derive(Debug, Clone)]
pub struct TokenProvider {
    oauth_url: String,
    refresh_token: String,
    principal_fingerprint: String,
    client: reqwest::Client,
    state: std::sync::Arc<RwLock<TokenState>>,
}

#[derive(Debug)]
struct TokenState {
    token: Option<String>,
    token_fingerprint: Option<String>,
    expires_at: Option<Instant>,
    obtained_at: Option<Instant>,
    obtained_ts_utc: Option<i64>,
    refresh_count: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(rename = "AccessToken")]
    access_token: String,
    #[serde(rename = "ExpiresIn")]
    expires_in: Option<i64>,
}

impl TokenProvider {
    pub fn new(oauth_url: impl Into<String>, refresh_token: impl Into<String>) -> Self {
        let refresh_token = refresh_token.into();
        Self {
            oauth_url: oauth_url.into(),
            principal_fingerprint: principal_fingerprint(&refresh_token),
            refresh_token,
            client: reqwest::Client::new(),
            state: std::sync::Arc::new(RwLock::new(TokenState {
                token: None,
                token_fingerprint: None,
                expires_at: None,
                obtained_at: None,
                obtained_ts_utc: None,
                refresh_count: 0,
            })),
        }
    }

    pub fn new_with_token(
        oauth_url: impl Into<String>,
        refresh_token: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        let token = token.into();
        let now = Instant::now();
        let refresh_token = refresh_token.into();
        Self {
            oauth_url: oauth_url.into(),
            principal_fingerprint: principal_fingerprint(&refresh_token),
            refresh_token,
            client: reqwest::Client::new(),
            state: std::sync::Arc::new(RwLock::new(TokenState {
                token_fingerprint: Some(access_token_fingerprint(&token)),
                token: Some(token),
                expires_at: Some(now + Duration::from_secs(60 * 60)),
                obtained_at: Some(now),
                obtained_ts_utc: Some(Utc::now().timestamp()),
                refresh_count: 0,
            })),
        }
    }

    pub async fn access_token(&self) -> anyhow::Result<String> {
        Ok(self.access_token_with_context("unspecified").await?.token)
    }

    pub async fn access_token_with_context(
        &self,
        consumer: &str,
    ) -> anyhow::Result<AccessTokenSnapshot> {
        {
            let guard = self.state.read().await;
            if let Some(snapshot) = token_snapshot_from_state(&guard, AccessTokenSource::Cached) {
                debug!(
                    consumer,
                    token_source = snapshot.source().as_str(),
                    access_token_fingerprint = snapshot.fingerprint(),
                    token_refresh_count = snapshot.refresh_count(),
                    access_token_obtained_ts_utc = snapshot.obtained_ts_utc(),
                    access_token_age_ms = snapshot.age_ms(),
                    access_token_ttl_remaining_ms = snapshot.ttl_remaining_ms(),
                    auth_principal_fingerprint = self.principal_fingerprint(),
                    "using cached alor access token"
                );
                return Ok(snapshot);
            }
        }

        let mut guard = self.state.write().await;
        if let Some(snapshot) = token_snapshot_from_state(&guard, AccessTokenSource::Cached) {
            debug!(
                consumer,
                token_source = snapshot.source().as_str(),
                access_token_fingerprint = snapshot.fingerprint(),
                token_refresh_count = snapshot.refresh_count(),
                access_token_obtained_ts_utc = snapshot.obtained_ts_utc(),
                access_token_age_ms = snapshot.age_ms(),
                access_token_ttl_remaining_ms = snapshot.ttl_remaining_ms(),
                auth_principal_fingerprint = self.principal_fingerprint(),
                "using cached alor access token"
            );
            return Ok(snapshot);
        }

        info!("refreshing alor access token");
        let response = self
            .client
            .post(&self.oauth_url)
            .query(&[("token", self.refresh_token.trim_matches('"').trim())])
            .send()
            .await?
            .error_for_status()?;

        let payload: TokenResponse = response.json().await?;
        let expires_in = payload.expires_in.unwrap_or(60 * 45);
        let obtained_at = Instant::now();
        let obtained_ts_utc = Utc::now().timestamp();
        let expires_at = obtained_at + Duration::from_secs(expires_in as u64);
        let token_fingerprint = access_token_fingerprint(&payload.access_token);

        guard.token = Some(payload.access_token.clone());
        guard.token_fingerprint = Some(token_fingerprint.clone());
        guard.expires_at = Some(expires_at);
        guard.obtained_at = Some(obtained_at);
        guard.obtained_ts_utc = Some(obtained_ts_utc);
        guard.refresh_count += 1;

        let snapshot = token_snapshot_from_state(&guard, AccessTokenSource::Refreshed)
            .expect("refreshed token should always be available");
        info!(
            consumer,
            token_source = snapshot.source().as_str(),
            access_token_fingerprint = snapshot.fingerprint(),
            token_refresh_count = snapshot.refresh_count(),
            access_token_obtained_ts_utc = snapshot.obtained_ts_utc(),
            access_token_age_ms = snapshot.age_ms(),
            access_token_ttl_remaining_ms = snapshot.ttl_remaining_ms(),
            auth_principal_fingerprint = self.principal_fingerprint(),
            expires_in,
            "refreshed alor access token"
        );

        Ok(snapshot)
    }

    pub async fn force_refresh_access_token_with_context(
        &self,
        consumer: &str,
        reason: &str,
    ) -> anyhow::Result<AccessTokenSnapshot> {
        let _ = self.invalidate(reason).await;
        self.access_token_with_context(consumer).await
    }

    pub async fn refresh_count(&self) -> u64 {
        self.state.read().await.refresh_count
    }

    pub async fn invalidate(&self, reason: &str) -> bool {
        let mut guard = self.state.write().await;
        let had_token = guard.token.is_some();
        guard.token = None;
        guard.token_fingerprint = None;
        guard.expires_at = None;
        guard.obtained_at = None;
        guard.obtained_ts_utc = None;
        info!(
            reason,
            had_token,
            refresh_count = guard.refresh_count,
            auth_principal_fingerprint = self.principal_fingerprint(),
            "invalidated cached alor access token"
        );
        had_token
    }

    pub fn principal_fingerprint(&self) -> &str {
        &self.principal_fingerprint
    }
}

fn principal_fingerprint(refresh_token: &str) -> String {
    short_sha256_fingerprint(refresh_token)
}

fn access_token_fingerprint(access_token: &str) -> String {
    short_sha256_fingerprint(access_token)
}

fn short_sha256_fingerprint(value: &str) -> String {
    let normalized = value.trim_matches('"').trim();
    let digest = Sha256::digest(normalized.as_bytes());
    format!("sha256:{}", &hex::encode(digest)[..16])
}

fn token_snapshot_from_state(
    guard: &TokenState,
    source: AccessTokenSource,
) -> Option<AccessTokenSnapshot> {
    let token = guard.token.as_ref()?;
    let expires_at = guard.expires_at;
    let not_expired = expires_at.map(|at| at > Instant::now()).unwrap_or(true);
    if !not_expired {
        return None;
    }

    let fingerprint = guard
        .token_fingerprint
        .clone()
        .unwrap_or_else(|| access_token_fingerprint(token));
    let age_ms = guard.obtained_at.map(|instant| {
        let millis = instant.elapsed().as_millis();
        u64::try_from(millis).unwrap_or(u64::MAX)
    });
    let ttl_remaining_ms = expires_at.map(|instant| {
        let remaining = instant
            .saturating_duration_since(Instant::now())
            .as_millis();
        u64::try_from(remaining).unwrap_or(u64::MAX)
    });

    Some(AccessTokenSnapshot {
        token: token.clone(),
        fingerprint,
        source,
        refresh_count: guard.refresh_count,
        obtained_ts_utc: guard.obtained_ts_utc,
        age_ms,
        ttl_remaining_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn access_token_snapshot_reports_cached_metadata() {
        let provider = TokenProvider::new_with_token(
            "http://example.test/oauth",
            "refresh-token",
            "cached-access-token",
        );

        let snapshot = provider
            .access_token_with_context("test_consumer")
            .await
            .expect("cached token snapshot");

        assert_eq!(snapshot.token(), "cached-access-token");
        assert_eq!(snapshot.source(), AccessTokenSource::Cached);
        assert_eq!(snapshot.refresh_count(), 0);
        assert!(snapshot.fingerprint().starts_with("sha256:"));
        assert!(snapshot.obtained_ts_utc().is_some());
        assert!(snapshot.age_ms().is_some());
        assert!(snapshot.ttl_remaining_ms().is_some());
    }

    #[tokio::test]
    async fn invalidate_clears_cached_token_and_expiry() {
        let provider = TokenProvider::new_with_token(
            "http://example.test/oauth",
            "refresh-token",
            "cached-access-token",
        );

        {
            let guard = provider.state.read().await;
            assert_eq!(guard.token.as_deref(), Some("cached-access-token"));
            assert!(guard.token_fingerprint.is_some());
            assert!(guard.expires_at.is_some());
            assert!(guard.obtained_at.is_some());
            assert!(guard.obtained_ts_utc.is_some());
        }

        let had_token = provider.invalidate("test_invalidation").await;
        assert!(had_token);

        let guard = provider.state.read().await;
        assert!(guard.token.is_none());
        assert!(guard.token_fingerprint.is_none());
        assert!(guard.expires_at.is_none());
        assert!(guard.obtained_at.is_none());
        assert!(guard.obtained_ts_utc.is_none());
        assert_eq!(guard.refresh_count, 0);
    }
}
