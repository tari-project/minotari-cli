// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use reqwest::Method;
use tari_transaction_components::transaction_components::Transaction;
use tari_utilities::hex::to_hex;
use url::Url;

use crate::http::utils::check_transaction_size;

use super::http_client::HttpClient;
use super::types::{JsonRpcResponse, TipInfoResponse, TxQueryResponse, TxSubmissionResponse};

pub struct WalletHttpClient {
    http_client: HttpClient,
}

impl WalletHttpClient {
    pub fn new(base_url: Url) -> Result<Self, anyhow::Error> {
        let http_client = HttpClient::new(base_url)?;
        Ok(Self { http_client })
    }

    pub fn with_config(base_url: Url, max_retries: u32, timeout: Duration) -> Result<Self, anyhow::Error> {
        let http_client = HttpClient::with_config(base_url, max_retries, timeout)?;
        Ok(Self { http_client })
    }

    pub fn get_address(&self) -> String {
        self.http_client.base_url().to_string()
    }

    pub async fn get_tip_info(&self) -> Result<TipInfoResponse, anyhow::Error> {
        println!("Requesting tip info from base node");
        let response = self
            .http_client
            .send_request(Method::GET, "/get_tip_info", None)
            .await?;
        Ok(response)
    }

    pub async fn is_online(&self) -> bool {
        match self.get_tip_info().await {
            Ok(_) => {
                println!("Base node is online");
                true
            },
            Err(e) => {
                println!("Base node is offline: {}", e);
                false
            },
        }
    }

    pub async fn get_last_request_latency(&self) -> Option<Duration> {
        self.http_client.get_latency().await
    }

    pub async fn submit_transaction(&self, transaction: Transaction) -> Result<TxSubmissionResponse, anyhow::Error> {
        println!("Submitting transaction");

        check_transaction_size(&transaction).map_err(|e| anyhow::anyhow!(e.to_string()))?;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "submit_transaction",
            "params": { "transaction": transaction }
        });

        let response: JsonRpcResponse<TxSubmissionResponse> = self
            .http_client
            .send_request(Method::POST, "/json_rpc", Some(request))
            .await?;

        match response.result {
            Some(result) => {
                println!("Transaction submitted successfully");
                Ok(result)
            },
            None => {
                let error_msg = response.error.unwrap_or_else(|| "Unknown error".to_string());
                println!("Transaction submission failed: {}", error_msg);
                Err(anyhow::anyhow!("Transaction submission failed: {}", error_msg))
            },
        }
    }

    pub async fn transaction_query(
        &self,
        excess_sig_nonce: &[u8],
        excess_sig: &[u8],
    ) -> Result<TxQueryResponse, anyhow::Error> {
        println!(
            "Querying transaction with excess sig nonce {} and signature {}",
            to_hex(excess_sig_nonce),
            to_hex(excess_sig)
        );

        let path = format!(
            "/transactions?excess_sig_nonce={}&excess_sig_sig={}",
            to_hex(excess_sig_nonce),
            to_hex(excess_sig)
        );

        let response = self.http_client.send_request(Method::GET, &path, None).await?;

        println!("Transaction query successful");
        Ok(response)
    }
}
