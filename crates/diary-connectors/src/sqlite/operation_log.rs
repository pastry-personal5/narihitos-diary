//! SQLite operation log implementation.

use super::*;

impl OperationLog for SqliteStore {
    fn record_intent(&self, intent: &OperationIntent) -> Result<bool, StoreError> {
        intent.validate().map_err(invalid_value)?;
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql_error)?;
        let existing = load_intent(&transaction, &intent.attempt_id, false)?;
        if let Some(existing) = existing {
            if existing.attempt_id != intent.attempt_id
                || existing.plan_id != intent.plan_id
                || existing.day != intent.day
                || existing.destination != intent.destination
                || existing.target_identity != intent.target_identity
                || existing.entry_digest != intent.entry_digest
                || existing.selection_digest != intent.selection_digest
                || existing.payload_digest != intent.payload_digest
                || existing.created_at != intent.created_at
            {
                return Err(StoreError::InvalidRecord(
                    "attempt ID was reused for a different operation".to_owned(),
                ));
            }
            transaction.commit().map_err(map_sql_error)?;
            return Ok(false);
        }
        let unresolved: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM operation_intents
                 WHERE day = ?1 AND terminal IS NULL",
                [intent.day.as_str()],
                |row| row.get(0),
            )
            .map_err(map_sql_error)?;
        if unresolved != 0 {
            return Err(StoreError::InvalidRecord(
                "workflow day has an unresolved operation intent".to_owned(),
            ));
        }
        transaction
            .execute(
                "INSERT INTO operation_intents (
                   attempt_id, plan_id, day, destination, target_identity,
                   entry_digest, selection_digest, payload_digest, created_at,
                   cancellation_requested, terminal
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)
                 ",
                params![
                    intent.attempt_id,
                    intent.plan_id,
                    intent.day.as_str(),
                    intent.destination,
                    intent.target_identity,
                    intent.entry_digest.as_str(),
                    intent.selection_digest.as_str(),
                    intent.payload_digest.as_str(),
                    intent.created_at.0,
                    intent.cancellation_requested
                ],
            )
            .map_err(map_sql_error)?;
        transaction.commit().map_err(map_sql_error)?;
        Ok(true)
    }

    fn mark_terminal(&self, attempt_id: &str, terminal: IntentTerminal) -> Result<(), StoreError> {
        let connection = self.connect()?;
        let changed = connection
            .execute(
                "UPDATE operation_intents SET terminal = ?1
                 WHERE attempt_id = ?2 AND terminal IS NULL",
                params![terminal.as_str(), attempt_id],
            )
            .map_err(map_sql_error)?;
        if changed == 0 {
            return Err(StoreError::InvalidRecord(
                "intent is missing or already terminal".to_owned(),
            ));
        }
        Ok(())
    }

    fn record_receipt(&self, receipt: &Receipt) -> Result<(), StoreError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql_error)?;
        let intent = load_intent(&transaction, &receipt.attempt_id, true)?
            .ok_or_else(|| StoreError::InvalidRecord("receipt intent is missing".to_owned()))?;
        receipt.validate_against(&intent).map_err(invalid_value)?;
        transaction
            .execute(
                "INSERT INTO receipts (
                   receipt_id, attempt_id, day, destination, target_identity,
                   version, entry_digest, selection_digest, payload_digest, verified_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    receipt.receipt_id,
                    receipt.attempt_id,
                    receipt.day.as_str(),
                    receipt.destination,
                    receipt.target_identity,
                    i64::from(receipt.version),
                    receipt.entry_digest.as_str(),
                    receipt.selection_digest.as_str(),
                    receipt.payload_digest.as_str(),
                    receipt.verified_at.0
                ],
            )
            .map_err(map_sql_error)?;
        for (position, returned_id) in receipt.returned_ids.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO receipt_returned_ids (receipt_id, position, returned_id)
                     VALUES (?1, ?2, ?3)",
                    params![
                        receipt.receipt_id,
                        to_i64(position, "returned ID position")?,
                        returned_id
                    ],
                )
                .map_err(map_sql_error)?;
        }
        let changed = transaction
            .execute(
                "UPDATE operation_intents SET terminal = 'verified' WHERE attempt_id = ?1",
                [&receipt.attempt_id],
            )
            .map_err(map_sql_error)?;
        if changed != 1 {
            return Err(StoreError::InvalidRecord(
                "receipt intent was not resolved".to_owned(),
            ));
        }
        transaction.commit().map_err(map_sql_error)
    }

    fn unresolved_intents(
        &self,
        day: Option<&DiaryDay>,
    ) -> Result<Vec<OperationIntent>, StoreError> {
        let connection = self.connect()?;
        let sql = if day.is_some() {
            "SELECT attempt_id, plan_id, day, destination, target_identity,
                    entry_digest, selection_digest, payload_digest, created_at,
                    cancellation_requested
             FROM operation_intents WHERE terminal IS NULL AND day = ?1 ORDER BY created_at"
        } else {
            "SELECT attempt_id, plan_id, day, destination, target_identity,
                    entry_digest, selection_digest, payload_digest, created_at,
                    cancellation_requested
             FROM operation_intents WHERE terminal IS NULL ORDER BY created_at"
        };
        let mut statement = connection.prepare(sql).map_err(map_sql_error)?;
        let mut intents = Vec::new();
        if let Some(day) = day {
            let mut rows = statement.query([day.as_str()]).map_err(map_sql_error)?;
            while let Some(row) = rows.next().map_err(map_sql_error)? {
                intents.push(read_intent(row)?);
            }
        } else {
            let mut rows = statement.query([]).map_err(map_sql_error)?;
            while let Some(row) = rows.next().map_err(map_sql_error)? {
                intents.push(read_intent(row)?);
            }
        }
        Ok(intents)
    }

    fn request_cancellation(&self, attempt_id: &str) -> Result<(), StoreError> {
        let connection = self.connect()?;
        let changed = connection
            .execute(
                "UPDATE operation_intents SET cancellation_requested = 1
                 WHERE attempt_id = ?1 AND terminal IS NULL",
                [attempt_id],
            )
            .map_err(map_sql_error)?;
        if changed == 0 {
            return Err(StoreError::InvalidRecord(
                "unresolved intent is missing".to_owned(),
            ));
        }
        Ok(())
    }

    fn reconciliation_gate(&self, day: &DiaryDay) -> Result<ReconciliationGate, StoreError> {
        let connection = self.connect()?;
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM operation_intents WHERE day = ?1 AND terminal IS NULL",
                [day.as_str()],
                |row| row.get(0),
            )
            .map_err(map_sql_error)?;
        Ok(if count == 0 {
            ReconciliationGate::Clear
        } else {
            ReconciliationGate::Reconciling
        })
    }

    fn receipts(&self, day: &DiaryDay) -> Result<Vec<Receipt>, StoreError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT receipt_id, attempt_id, day, destination, target_identity,
                        version, entry_digest, selection_digest, payload_digest, verified_at
                 FROM receipts WHERE day = ?1 ORDER BY version, destination",
            )
            .map_err(map_sql_error)?;
        let mut rows = statement.query([day.as_str()]).map_err(map_sql_error)?;
        let mut receipts = Vec::new();
        while let Some(row) = rows.next().map_err(map_sql_error)? {
            let receipt_id: String = row.get(0).map_err(map_sql_error)?;
            let mut id_statement = connection
                .prepare(
                    "SELECT returned_id FROM receipt_returned_ids
                     WHERE receipt_id = ?1 ORDER BY position",
                )
                .map_err(map_sql_error)?;
            let returned_ids = id_statement
                .query_map([&receipt_id], |id_row| id_row.get::<_, String>(0))
                .map_err(map_sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_sql_error)?;
            let receipt = Receipt {
                receipt_id,
                attempt_id: row.get(1).map_err(map_sql_error)?,
                day: DiaryDay::parse(row.get::<_, String>(2).map_err(map_sql_error)?)
                    .map_err(invalid_value)?,
                destination: row.get(3).map_err(map_sql_error)?,
                target_identity: row.get(4).map_err(map_sql_error)?,
                version: to_u32(row.get(5).map_err(map_sql_error)?, "receipt version")?,
                entry_digest: parse_digest(row.get(6).map_err(map_sql_error)?)?,
                selection_digest: parse_digest(row.get(7).map_err(map_sql_error)?)?,
                payload_digest: parse_digest(row.get(8).map_err(map_sql_error)?)?,
                returned_ids,
                verified_at: Timestamp(row.get(9).map_err(map_sql_error)?),
            };
            let intent = load_intent(&connection, &receipt.attempt_id, false)?
                .ok_or_else(|| StoreError::InvalidRecord("receipt intent is missing".to_owned()))?;
            receipt.validate_against(&intent).map_err(invalid_value)?;
            receipts.push(receipt);
        }
        Ok(receipts)
    }
}
