use serde::{Deserialize, Serialize};

use super::formatting::output_type_from_number;

/// Parsed wallet output data with extracted fields.
#[derive(Debug, Clone)]
pub struct ParsedWalletOutput {
    pub output_type: String,
    pub is_coinbase: bool,
    pub coinbase_extra: Option<String>,
    /// Hashes of outputs that were spent as inputs for this transaction (hex encoded).
    pub sent_output_hashes: Vec<String>,
}

impl ParsedWalletOutput {
    /// Parse wallet_output_json and extract all relevant fields.
    pub fn from_json(json_str: &str) -> Option<Self> {
        if let Ok(partial) = serde_json::from_str::<WalletOutputPartial>(json_str) {
            let output_type = Self::parse_output_type(&partial.features.output_type);
            let is_coinbase = output_type.eq_ignore_ascii_case("coinbase");
            let coinbase_extra = if is_coinbase {
                partial.features.coinbase_extra.clone()
            } else {
                None
            };

            let sent_output_hashes = Self::extract_sent_output_hashes(&partial.payment_id);

            return Some(Self {
                output_type,
                is_coinbase,
                coinbase_extra,
                sent_output_hashes,
            });
        }

        Self::from_json_fallback(json_str)
    }

    fn from_json_fallback(json_str: &str) -> Option<Self> {
        let value = serde_json::from_str::<serde_json::Value>(json_str).ok()?;
        let features = value.get("features");

        let output_type = features
            .and_then(|f| f.get("output_type"))
            .map(Self::parse_output_type)
            .unwrap_or_else(|| "Standard".to_string());

        let is_coinbase = output_type.eq_ignore_ascii_case("coinbase");

        let coinbase_extra = if is_coinbase {
            features
                .and_then(|f| f.get("coinbase_extra"))
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        } else {
            None
        };

        let payment_id = value.get("payment_id").cloned().unwrap_or(serde_json::Value::Null);
        let sent_output_hashes = Self::extract_sent_output_hashes(&payment_id);

        Some(Self {
            output_type,
            is_coinbase,
            coinbase_extra,
            sent_output_hashes,
        })
    }

    fn parse_output_type(value: &serde_json::Value) -> String {
        if let Some(s) = value.as_str() {
            s.to_string()
        } else if let Some(n) = value.as_u64() {
            output_type_from_number(n)
        } else {
            "Standard".to_string()
        }
    }

    fn extract_sent_output_hashes(payment_id: &serde_json::Value) -> Vec<String> {
        let hashes_array = payment_id
            .get("inner")
            .and_then(|inner| inner.get("TransactionInfo"))
            .and_then(|tx_info| tx_info.get("sent_output_hashes"))
            .and_then(|hashes| hashes.as_array());

        let Some(hashes_array) = hashes_array else {
            return Vec::new();
        };

        hashes_array
            .iter()
            .filter_map(|hash_value| {
                let bytes_array = hash_value.as_array()?;
                let bytes: Vec<u8> = bytes_array.iter().filter_map(|b| b.as_u64().map(|n| n as u8)).collect();

                if bytes.is_empty() {
                    None
                } else {
                    Some(hex::encode(&bytes))
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletOutputPartial {
    pub features: OutputFeaturesPartial,
    #[serde(default)]
    pub payment_id: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutputFeaturesPartial {
    #[serde(default)]
    pub output_type: serde_json::Value,
    #[serde(default)]
    pub coinbase_extra: Option<String>,
    #[serde(default)]
    pub maturity: Option<u64>,
}
