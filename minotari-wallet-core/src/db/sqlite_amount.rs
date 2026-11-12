//! Serde readers for amount columns. `MicroMinotari` derives a newtype `Deserialize`, which `from_rows` cannot fill
//! from an integer column.

use serde::{Deserialize, Deserializer};
use tari_transaction_components::MicroMinotari;

pub(crate) fn micro_minotari<'de, D: Deserializer<'de>>(deserializer: D) -> Result<MicroMinotari, D::Error> {
    u64::deserialize(deserializer).map(MicroMinotari)
}

pub(crate) fn micro_minotari_opt<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<MicroMinotari>, D::Error> {
    Ok(Option::<u64>::deserialize(deserializer)?.map(MicroMinotari))
}
