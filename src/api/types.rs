use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use tari_common_types::tari_address::TariAddress;

// Helper struct for serializing/deserializing TariAddress as a hex string
#[derive(Debug, Clone, PartialEq, Eq, utoipa::ToSchema)]
#[schema(value_type = String)]
pub struct TariAddressHex(pub TariAddress);

impl Serialize for TariAddressHex {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_hex())
    }
}

impl<'de> Deserialize<'de> for TariAddressHex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        TariAddress::from_hex(&s).map(TariAddressHex).map_err(de::Error::custom)
    }
}
