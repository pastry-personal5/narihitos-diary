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
                log.mark_terminal(&intent.attempt_id, IntentTerminal::Changed)?;
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
                log.mark_terminal(&intent.attempt_id, IntentTerminal::Changed)?;
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

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::{DestinationKind, DestinationPlan, DiaryDay, Digest, ReconciliationGate};

    #[derive(Default)]
    struct FakeLog {
        intents: RefCell<Vec<OperationIntent>>,
        receipts: RefCell<Vec<Receipt>>,
        terminals: RefCell<Vec<String>>,
        fail_next_receipt: Cell<bool>,
    }

    impl OperationLog for FakeLog {
        fn record_intent(&self, intent: &OperationIntent) -> Result<bool, StoreError> {
            self.intents.borrow_mut().push(intent.clone());
            Ok(true)
        }

        fn mark_terminal(&self, attempt_id: &str, _: IntentTerminal) -> Result<(), StoreError> {
            self.terminals.borrow_mut().push(attempt_id.to_owned());
            Ok(())
        }

        fn record_receipt(&self, receipt: &Receipt) -> Result<(), StoreError> {
            if self.fail_next_receipt.replace(false) {
                return Err(StoreError::Unavailable("receipt write failed".into()));
            }
            self.receipts.borrow_mut().push(receipt.clone());
            Ok(())
        }

        fn unresolved_intents(
            &self,
            _: Option<&DiaryDay>,
        ) -> Result<Vec<OperationIntent>, StoreError> {
            Ok(self
                .intents
                .borrow()
                .iter()
                .filter(|intent| {
                    !self.terminals.borrow().contains(&intent.attempt_id)
                        && !self
                            .receipts
                            .borrow()
                            .iter()
                            .any(|receipt| receipt.attempt_id == intent.attempt_id)
                })
                .cloned()
                .collect())
        }

        fn request_cancellation(&self, _: &str) -> Result<(), StoreError> {
            Ok(())
        }

        fn reconciliation_gate(&self, _: &DiaryDay) -> Result<ReconciliationGate, StoreError> {
            Ok(if self.unresolved_intents(None)?.is_empty() {
                ReconciliationGate::Clear
            } else {
                ReconciliationGate::Reconciling
            })
        }

        fn receipts(&self, _: &DiaryDay) -> Result<Vec<Receipt>, StoreError> {
            Ok(self.receipts.borrow().clone())
        }
    }

    /// A destination whose write always lands and verifies.
    #[derive(Default)]
    struct LandedDestination {
        writes: usize,
    }

    impl Destination for LandedDestination {
        fn write(&mut self, _: &OperationIntent) -> WriteOutcome {
            self.writes += 1;
            WriteOutcome::Verified {
                receipt_id: "month".into(),
                returned_ids: vec!["block".into()],
            }
        }

        fn inspect(&mut self, _: &OperationIntent) -> InspectionOutcome {
            InspectionOutcome::Matching {
                receipt_id: "month".into(),
                returned_ids: vec!["block".into()],
            }
        }
    }

    fn intent() -> OperationIntent {
        let target = DestinationPlan::new(
            DestinationKind::Remote,
            "notion",
            "root",
            Digest::of_bytes(b"intended"),
            Digest::of_bytes(b"payload"),
            Digest::of_bytes(b"inspected"),
        )
        .unwrap();
        OperationIntent::new(
            "attempt",
            "plan",
            DiaryDay::parse("2026-10-30").unwrap(),
            &target,
            Digest::of_bytes(b"entry"),
            Digest::of_bytes(b"selection"),
            Timestamp(1),
        )
        .unwrap()
    }

    #[test]
    fn failing_to_persist_the_receipt_keeps_the_intent_unresolved_and_reconciles_without_rewriting()
    {
        let log = FakeLog::default();
        let mut destination = LandedDestination::default();
        let intent = intent();
        log.fail_next_receipt.set(true);

        let failed =
            PublicationCoordinator::execute(&log, &mut destination, &intent, 2, Timestamp(2));
        assert!(matches!(failed, Err(PublicationError::Store(_))));
        // The effect happened exactly once, but nothing claims it is done: the
        // intent stays unresolved so the day remains gated.
        assert_eq!(destination.writes, 1);
        assert!(log.receipts.borrow().is_empty());
        assert!(log.terminals.borrow().is_empty());
        assert_eq!(log.unresolved_intents(None).unwrap().len(), 1);
        assert_eq!(
            log.reconciliation_gate(&intent.day).unwrap(),
            ReconciliationGate::Reconciling
        );

        // Recovery inspects the destination and adopts the section; it never writes again.
        let adopted =
            PublicationCoordinator::reconcile(&log, &mut destination, &intent, 2, Timestamp(3))
                .unwrap();
        assert!(matches!(adopted, PublicationResult::Verified(_)));
        assert_eq!(destination.writes, 1);
        assert_eq!(log.receipts.borrow().len(), 1);
        assert!(log.unresolved_intents(None).unwrap().is_empty());
    }

    #[test]
    fn failing_to_persist_a_reconciled_receipt_leaves_the_intent_unresolved() {
        let log = FakeLog::default();
        let mut destination = LandedDestination::default();
        let intent = intent();
        log.record_intent(&intent).unwrap();
        log.fail_next_receipt.set(true);

        let failed =
            PublicationCoordinator::reconcile(&log, &mut destination, &intent, 2, Timestamp(2));
        assert!(matches!(failed, Err(PublicationError::Store(_))));
        assert_eq!(log.unresolved_intents(None).unwrap().len(), 1);
        // The next attempt succeeds and resolves it, still without a write.
        assert!(matches!(
            PublicationCoordinator::reconcile(&log, &mut destination, &intent, 2, Timestamp(3)),
            Ok(PublicationResult::Verified(_))
        ));
        assert_eq!(destination.writes, 0);
        assert!(log.unresolved_intents(None).unwrap().is_empty());
    }
}
