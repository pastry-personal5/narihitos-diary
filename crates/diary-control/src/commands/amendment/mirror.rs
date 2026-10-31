//! Amendment mirror use cases.

use diary_core::Receipt;

use super::*;

impl CommandService {
    pub(in crate::commands) fn receipts_of(
        &self,
        day: &DiaryDay,
    ) -> Result<Vec<Receipt>, CommandError> {
        self.store.receipts(day).map_err(CommandError::Store)
    }

    /// Latest receipt of `destination` made at or before `version`.
    pub(in crate::commands) fn owned_receipt(
        receipts: &[Receipt],
        destination: &str,
        version: u32,
    ) -> Option<Receipt> {
        receipts
            .iter()
            .filter(|receipt| receipt.destination == destination && receipt.version <= version)
            .max_by_key(|receipt| (receipt.version, receipt.verified_at.0))
            .cloned()
    }
}
