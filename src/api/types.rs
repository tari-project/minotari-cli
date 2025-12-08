use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use tari_common_types::tari_address::TariAddress;
use tari_transaction_components::{tari_amount::MicroMinotari, transaction_components::WalletOutput};

// Helper struct for serializing/deserializing TariAddress as a hex string
#[derive(Debug, Clone, PartialEq, Eq, utoipa::ToSchema)]
#[schema(value_type = String)]
pub struct TariAddressBase58(pub TariAddress);

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct LockFundsResult {
    #[schema(value_type = Vec<Object>)]
    pub utxos: Vec<WalletOutput>,

    pub requires_change_output: bool,

    #[schema(value_type = u64)]
    pub total_value: MicroMinotari,

    #[schema(value_type = u64)]
    pub fee_without_change: MicroMinotari,

    #[schema(value_type = u64)]
    pub fee_with_change: MicroMinotari,
}

impl Serialize for TariAddressBase58 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_base58())
    }
}

impl<'de> Deserialize<'de> for TariAddressBase58 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        TariAddress::from_base58(&s)
            .map(TariAddressBase58)
            .map_err(de::Error::custom)
    }
}
