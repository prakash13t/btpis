use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

use crate::config::OauthConfig;

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[allow(dead_code)]
    pub token_type: String,
    pub expires_in: Option<u64>,
}

pub async fn fetch_token(oauth: &OauthConfig) -> Result<TokenResponse> {
    let client = Client::new();

    let params = [
        ("grant_type", "client_credentials"),
        ("client_id", &oauth.client_id),
        ("client_secret", &oauth.client_secret),
    ];

    let response = client
        .post(&oauth.token_url)
        .form(&params)
        .send()
        .await
        .context("failed to reach token endpoint")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("token request failed with status {}: {}", status, body);
    }

    let token: TokenResponse = response
        .json()
        .await
        .context("failed to deserialise token response")?;

    Ok(token)
}
