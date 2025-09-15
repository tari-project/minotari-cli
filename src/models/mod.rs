// Change depending on sql type.
pub type Id = i64;

pub struct ScannedTipBlock {
    pub id: Id,
    pub account_id: Id,
    pub height: u64,
    pub hash: Vec<u8>,
}
