use std::time::{Duration, Instant};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::{debug, info};

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
    expires_at: Option<Instant>,
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
                expires_at: None,
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
        let refresh_token = refresh_token.into();
        Self {
            oauth_url: oauth_url.into(),
            principal_fingerprint: principal_fingerprint(&refresh_token),
            refresh_token,
            client: reqwest::Client::new(),
            state: std::sync::Arc::new(RwLock::new(TokenState {
                token: Some(token),
                expires_at: Some(Instant::now() + Duration::from_secs(60 * 60)),
                refresh_count: 0,
            })),
        }
    }

    pub async fn access_token(&self) -> anyhow::Result<String> {
        {
            let guard = self.state.read().await;
            let token = guard.token.as_ref();
            let not_expired = guard
                .expires_at
                .map(|at| at > Instant::now())
                .unwrap_or(true);
            if let (Some(token), true) = (token, not_expired) {
                return Ok(token.clone());
            }
        }

        let mut guard = self.state.write().await;
        let token = guard.token.as_ref();
        let not_expired = guard
            .expires_at
            .map(|at| at > Instant::now())
            .unwrap_or(true);
        if let (Some(token), true) = (token, not_expired) {
            return Ok(token.clone());
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
        let expires_at = Instant::now() + Duration::from_secs(expires_in as u64);

        guard.token = Some(payload.access_token.clone());
        guard.expires_at = Some(expires_at);
        guard.refresh_count += 1;

        debug!(
            refresh_count = guard.refresh_count,
            expires_in, "token refreshed"
        );

        Ok(payload.access_token)
    }

    pub async fn refresh_count(&self) -> u64 {
        self.state.read().await.refresh_count
    }

    pub async fn invalidate(&self, reason: &str) -> bool {
        let mut guard = self.state.write().await;
        let had_token = guard.token.is_some();
        guard.token = None;
        guard.expires_at = None;
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
    let normalized = refresh_token.trim_matches('"').trim();
    let digest = Sha256::digest(normalized.as_bytes());
    format!("sha256:{}", &hex::encode(digest)[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert!(guard.expires_at.is_some());
        }

        let had_token = provider.invalidate("test_invalidation").await;
        assert!(had_token);

        let guard = provider.state.read().await;
        assert!(guard.token.is_none());
        assert!(guard.expires_at.is_none());
        assert_eq!(guard.refresh_count, 0);
    }
}
