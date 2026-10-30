//! Intent-before-effect publication and reconciliation policy.

use std::fmt;

use crate::ports::IntentTerminal;
use crate::{
    Destination, InspectionOutcome, OperationIntent, OperationLog, Receipt, StoreError, Timestamp,
    WriteOutcome,
};

pub struct PublicationCoordinator;

impl PublicationCoordinator {
    pub fn execute(
        log: &(impl OperationLog + ?Sized),
        destination: &mut impl Destination,
        intent: &OperationIntent,
        receipt_version: u32,
        now: Timestamp,
    ) -> Result<PublicationResult, PublicationError> {
        if !log.record_intent(intent)? {
            return Ok(PublicationResult::DuplicateCommand);
        }
        match destination.write(intent) {
            WriteOutcome::Verified {
                receipt_id,
                returned_ids,
            } => {
                let receipt = make_receipt(intent, receipt_id, returned_ids, receipt_version, now);
                receipt
                    .validate_against(intent)
                    .map_err(|_| PublicationError::InvalidOutcome)?;
                log.record_receipt(&receipt)?;
                Ok(PublicationResult::Verified(Box::new(receipt)))
            }
            WriteOutcome::DefiniteNoEffect => {
                log.mark_terminal(&intent.attempt_id, IntentTerminal::DefiniteNoEffect)?;
                Ok(PublicationResult::DefiniteNoEffect)
            }
            WriteOutcome::Conflict => {
                log.mark_terminal(&intent.attempt_id, IntentTerminal::Conflict)?;
                Ok(PublicationResult::Conflict)
            }
            WriteOutcome::AmbiguousWrite | WriteOutcome::Unavailable => {
                Ok(PublicationResult::Reconciling)
            }
        }
    }

    pub fn reconcile(
        log: &(impl OperationLog + ?Sized),
        destination: &mut impl Destination,
        intent: &OperationIntent,
        receipt_version: u32,
        now: Timestamp,
    ) -> Result<PublicationResult, PublicationError> {
        match destination.inspect(intent) {
            InspectionOutcome::Matching {
                receipt_id,
                returned_ids,
            } => {
                let receipt = make_receipt(intent, receipt_id, returned_ids, receipt_version, now);
                receipt
                    .validate_against(intent)
                    .map_err(|_| PublicationError::InvalidOutcome)?;
                log.record_receipt(&receipt)?;
                Ok(PublicationResult::Verified(Box::new(receipt)))
            }
            InspectionOutcome::Absent => {
                log.mark_terminal(&intent.attempt_id, IntentTerminal::DefiniteNoEffect)?;
                Ok(PublicationResult::SafeToRetry)
            }
            InspectionOutcome::Conflict => {
                log.mark_terminal(&intent.attempt_id, IntentTerminal::Conflict)?;
                Ok(PublicationResult::Conflict)
            }
            InspectionOutcome::Unavailable => Ok(PublicationResult::Reconciling),
        }
    }
}

fn make_receipt(
    intent: &OperationIntent,
    receipt_id: String,
    returned_ids: Vec<String>,
    version: u32,
    now: Timestamp,
) -> Receipt {
    Receipt {
        receipt_id,
        attempt_id: intent.attempt_id.clone(),
        day: intent.day.clone(),
        destination: intent.destination.clone(),
        target_identity: intent.target_identity.clone(),
        version,
        entry_digest: intent.entry_digest.clone(),
        selection_digest: intent.selection_digest.clone(),
        payload_digest: intent.payload_digest.clone(),
        returned_ids,
        verified_at: now,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicationResult {
    Verified(Box<Receipt>),
    DefiniteNoEffect,
    Conflict,
    Reconciling,
    SafeToRetry,
    DuplicateCommand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicationError {
    Store(StoreError),
    InvalidOutcome,
}

impl From<StoreError> for PublicationError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl fmt::Display for PublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(formatter),
            Self::InvalidOutcome => formatter.write_str("destination returned an invalid receipt"),
        }
    }
}

impl std::error::Error for PublicationError {}
