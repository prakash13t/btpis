use crate::config::OauthConfig;
use anyhow::{Context, Result};
use reqwest::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::Deserialize;

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

pub async fn fetch_csrf_token(client: &Client, base_url: &str, token: &str) -> Result<String> {
    let url = format!("{}/api/v1/", base_url.trim_end_matches('/'));

    let response = client
        .get(&url)
        .header(ACCEPT, "application/xml")
        .header("x-csrf-token", "Fetch")
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .context("failed to fetch iflow metadata for CSRF token")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("CSRF token request failed with status {}: {}", status, body);
    }

    let csrf_token = response
        .headers()
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim()
        .to_string();

    if csrf_token.is_empty() {
        anyhow::bail!("CSRF token header was missing from the response");
    }

    Ok(csrf_token)
}
