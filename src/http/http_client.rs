// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::time::{Duration, Instant};

use reqwest::Method;
use serde::de::DeserializeOwned;
use tokio::sync::RwLock;
use url::Url;

use super::error::HttpError;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_MAX_RETRIES: u32 = 3;

pub(crate) struct HttpClient {
    base_url: Url,
    client: reqwest_middleware::ClientWithMiddleware,
    last_latency: RwLock<Option<(Duration, Instant)>>,
}

impl HttpClient {
    pub fn new(base_url: Url) -> Result<Self, anyhow::Error> {
        Self::with_config(base_url, DEFAULT_MAX_RETRIES, Duration::from_secs(DEFAULT_TIMEOUT_SECS))
    }

    pub fn with_config(base_url: Url, max_retries: u32, timeout: Duration) -> Result<Self, anyhow::Error> {
        let retry_policy = reqwest_retry::policies::ExponentialBackoff::builder().build_with_max_retries(max_retries);

        let inner_client = reqwest::Client::builder().timeout(timeout).build()?;

        let client = reqwest_middleware::ClientBuilder::new(inner_client)
            .with(reqwest_retry::RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        Ok(Self {
            base_url,
            client,
            last_latency: RwLock::new(None),
        })
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub async fn send_request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T, HttpError> {
        let start = Instant::now();
        let url = self.base_url.join(path)?;

        let req = match method {
            Method::GET => self.client.get(url),
            Method::POST => {
                let req = self.client.post(url);
                if let Some(body) = body {
                    req.body(serde_json::to_string(&body)?)
                        .header("Content-Type", "application/json")
                } else {
                    req
                }
            },
            _ => return Err(HttpError::UnsupportedMethod),
        };

        let resp = req.send().await?;
        let latency = start.elapsed();
        self.update_latency(latency).await;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "Failed to read response body".into());
            return Err(HttpError::ServerError { status, body });
        }

        Ok(resp.json().await?)
    }

    async fn update_latency(&self, duration: Duration) {
        *self.last_latency.write().await = Some((duration, Instant::now()));
    }

    pub async fn get_latency(&self) -> Option<Duration> {
        self.last_latency.read().await.map(|(d, _)| d)
    }
}
