use std::str::FromStr;

use tari_common_types::tari_address::TariAddress;

use super::types::TransactionSource;

/// Tolerance for amount-based input matching (approximately 1 XTM).
pub const AMOUNT_MATCHING_TOLERANCE: u64 = 999_999;

/// Format microTari to human-readable string (e.g., "1,234.567890 XTM").
pub fn format_micro_tari(micro_tari: u64) -> String {
    let whole_tari = micro_tari / 1_000_000;
    let fractional = micro_tari % 1_000_000;

    if whole_tari >= 1000 {
        let formatted_whole = format_with_thousands_separator(whole_tari);
        format!("{}.{:06} XTM", formatted_whole, fractional)
    } else {
        format!("{}.{:06} XTM", whole_tari, fractional)
    }
}

fn format_with_thousands_separator(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);

    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }

    result.chars().rev().collect()
}

/// Convert a Tari address (base58) to emoji representation.
pub fn address_to_emoji(address: &str) -> Option<String> {
    TariAddress::from_str(address).ok().map(|addr| addr.to_emoji_string())
}

/// Determine transaction source from available data.
pub fn determine_transaction_source(
    is_coinbase: bool,
    has_sender_address: bool,
    has_recipient_address: bool,
) -> TransactionSource {
    if is_coinbase {
        TransactionSource::Coinbase
    } else if has_sender_address && !has_recipient_address {
        TransactionSource::OneSided
    } else if !has_sender_address && !has_recipient_address {
        TransactionSource::OneSided
    } else {
        TransactionSource::Transfer
    }
}

/// Convert numeric output_type to string representation.
pub fn output_type_from_number(n: u64) -> String {
    match n {
        0 => "Standard".to_string(),
        1 => "Coinbase".to_string(),
        2 => "Burn".to_string(),
        3 => "ValidatorNodeRegistration".to_string(),
        4 => "CodeTemplateRegistration".to_string(),
        _ => format!("Unknown({})", n),
    }
}
