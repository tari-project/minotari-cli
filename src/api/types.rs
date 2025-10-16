use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use tari_common_types::tari_address::TariAddress;

// Helper struct for serializing/deserializing TariAddress as a hex string
#[derive(Debug, Clone, PartialEq, Eq, utoipa::ToSchema)]
#[schema(value_type = String)]
pub struct TariAddressBase58(pub TariAddress);

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
