use std::path::{Path, PathBuf};
use std::time::Duration;

use diary_core::{
    Approval, AssistantBackendId, AssistantSelection, CompositionPath, DateEvidence,
    DestinationKind, DestinationPlan, DestinationProgress, DestinationStatus, DiaryDay, Digest,
    EntryBuffer, EntryConfirmation, FreshnessStamp, IntentTerminal, InvalidationReason, JobStatus,
    OperationIntent, OperationLog, OutputPlan, PersistedWorkflow, Preferences, Receipt,
    ReconciliationGate, SourceAvailability, SourceCandidate, SourceRef, SourceSnapshot,
    StartOutcome, StoreError, Timestamp, Workflow, WorkflowState, WorkflowStore, WorkflowSummary,
    WritingLanguage, WritingStyle,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

const CURRENT_SCHEMA: i64 = 4;

#[derive(Clone, Debug)]
pub struct SqliteStore {
    path: PathBuf,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let store = Self {
            path: path.as_ref().to_path_buf(),
        };
        let mut connection = store.connect()?;
        migrate(&mut connection)?;
        validate_schema(&connection)?;
        Ok(store)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connect(&self) -> Result<Connection, StoreError> {
        let connection = Connection::open(&self.path).map_err(map_open_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(map_sql_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(map_sql_error)?;
        Ok(connection)
    }
}

impl WorkflowStore for SqliteStore {
    fn start_or_resume(
        &self,
        day: DiaryDay,
        now: Timestamp,
        preferences: &Preferences,
    ) -> Result<StartOutcome, StoreError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql_error)?;
        if workflow_exists(&transaction, &day)? {
            let workflow = load_workflow(&transaction, &day)?
                .ok_or_else(|| StoreError::InvalidRecord("workflow disappeared".to_owned()))?;
            transaction.commit().map_err(map_sql_error)?;
            return Ok(if workflow.state() == WorkflowState::Completed {
                StartOutcome::Completed(workflow)
            } else {
                StartOutcome::Resumed(workflow)
            });
        }

        let workflow = Workflow::new(day, now, preferences);
        insert_workflow(&transaction, &workflow.persisted())?;
        transaction.commit().map_err(map_sql_error)?;
        Ok(StartOutcome::Created(workflow))
    }

    fn load(&self, day: &DiaryDay) -> Result<Option<Workflow>, StoreError> {
        let connection = self.connect()?;
        load_workflow(&connection, day)
    }

    fn list_summaries(&self, limit: usize) -> Result<Vec<WorkflowSummary>, StoreError> {
        if limit > 256 {
            return Err(StoreError::InvalidRecord(
                "workflow summary limit exceeds 256".to_owned(),
            ));
        }
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT day, state, revision, buffer_version, updated_at, discarded_at
                 FROM workflows
                 ORDER BY CASE WHEN state = 'completed' THEN 1 ELSE 0 END,
                          updated_at DESC, day DESC
                 LIMIT ?1",
            )
            .map_err(map_sql_error)?;
        let rows = statement
            .query_map([to_i64(limit, "workflow summary limit")?], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            })
            .map_err(map_sql_error)?;
        rows.map(|row| {
            let (day, state, revision, buffer_version, updated_at, discarded_at) =
                row.map_err(map_sql_error)?;
            Ok(WorkflowSummary {
                day: DiaryDay::parse(day).map_err(invalid_value)?,
                state: WorkflowState::parse(&state).map_err(invalid_value)?,
                revision: to_u64(revision, "workflow revision")?,
                buffer_version: to_u64(buffer_version, "buffer version")?,
                updated_at: Timestamp(updated_at),
                discarded_at: discarded_at.map(Timestamp),
            })
        })
        .collect()
    }

    fn compare_and_swap(
        &self,
        expected_revision: u64,
        expected_buffer_version: Option<u64>,
        workflow: &Workflow,
    ) -> Result<(), StoreError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql_error)?;
        let actual = transaction
            .query_row(
                "SELECT revision, buffer_version FROM workflows WHERE day = ?1",
                [workflow.day().as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(map_sql_error)?
            .ok_or_else(|| StoreError::InvalidRecord("workflow is missing".to_owned()))?;
        if actual.0 != to_i64(expected_revision, "workflow revision")? {
            return Err(StoreError::StaleRevision);
        }
        if expected_buffer_version.is_some_and(|version| {
            to_i64(version, "buffer version").is_err()
                || actual.1 != i64::try_from(version).unwrap_or(-1)
        }) {
            return Err(StoreError::StaleBuffer);
        }
        replace_workflow(&transaction, &workflow.persisted())?;
        transaction.commit().map_err(map_sql_error)
    }

    fn load_preferences(&self) -> Result<Preferences, StoreError> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT last_language, last_assistant_backend_id FROM preferences WHERE singleton = 1",
                [],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .map_err(map_sql_error)
            .and_then(|(language, backend_id)| {
                Ok(Preferences {
                    last_language: language
                        .as_deref()
                        .map(WritingLanguage::parse)
                        .transpose()
                        .map_err(invalid_value)?,
                    last_assistant_backend_id: backend_id
                        .map(AssistantBackendId::parse)
                        .transpose()
                        .map_err(invalid_value)?,
                })
            })
    }

    fn save_preferences(&self, preferences: &Preferences) -> Result<(), StoreError> {
        let connection = self.connect()?;
        update_preferences(&connection, preferences)
    }

    fn compare_and_swap_with_preferences(
        &self,
        expected_revision: u64,
        workflow: &Workflow,
        preferences: &Preferences,
    ) -> Result<(), StoreError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql_error)?;
        let actual: i64 = transaction
            .query_row(
                "SELECT revision FROM workflows WHERE day = ?1",
                [workflow.day().as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_sql_error)?
            .ok_or_else(|| StoreError::InvalidRecord("workflow is missing".to_owned()))?;
        if actual != to_i64(expected_revision, "workflow revision")? {
            return Err(StoreError::StaleRevision);
        }
        replace_workflow(&transaction, &workflow.persisted())?;
        update_preferences(&transaction, preferences)?;
        transaction.commit().map_err(map_sql_error)
    }

    fn discard(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql_error)?;
        let actual: i64 = transaction
            .query_row(
                "SELECT revision FROM workflows WHERE day = ?1",
                [day.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_sql_error)?
            .ok_or_else(|| StoreError::InvalidRecord("workflow is missing".to_owned()))?;
        if actual != to_i64(expected_revision, "workflow revision")? {
            return Err(StoreError::StaleRevision);
        }
        purge_day(&transaction, day, now, true)?;
        transaction.commit().map_err(map_sql_error)
    }

    fn purge_expired(&self, now: Timestamp) -> Result<Vec<DiaryDay>, StoreError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql_error)?;
        let verified_cutoff = now.days_before(7).0;
        let abandoned_cutoff = now.days_before(30).0;
        let raw_days = {
            let mut statement = transaction
                .prepare(
                    "SELECT day FROM workflows
                     WHERE discarded_at IS NULL AND (
                       (all_verified_at IS NOT NULL AND all_verified_at <= ?1) OR
                       (all_verified_at IS NULL AND updated_at <= ?2)
                     ) ORDER BY day",
                )
                .map_err(map_sql_error)?;
            let rows = statement
                .query_map(params![verified_cutoff, abandoned_cutoff], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(map_sql_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(map_sql_error)?
        };
        let mut days = Vec::with_capacity(raw_days.len());
        for raw_day in raw_days {
            let day = DiaryDay::parse(raw_day).map_err(invalid_value)?;
            purge_day(&transaction, &day, now, false)?;
            days.push(day);
        }
        transaction.commit().map_err(map_sql_error)?;
        Ok(days)
    }
}

fn update_preferences(
    connection: &Connection,
    preferences: &Preferences,
) -> Result<(), StoreError> {
    let changed = connection
        .execute(
            "UPDATE preferences SET last_language = ?1, last_assistant_backend_id = ?2
             WHERE singleton = 1",
            params![
                preferences.last_language.map(WritingLanguage::as_str),
                preferences
                    .last_assistant_backend_id
                    .as_ref()
                    .map(AssistantBackendId::as_str)
            ],
        )
        .map_err(map_sql_error)?;
    if changed != 1 {
        return Err(StoreError::InvalidRecord(
            "preferences singleton is missing".to_owned(),
        ));
    }
    Ok(())
}

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

fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(map_sql_error)?;
    if version > CURRENT_SCHEMA {
        return Err(StoreError::UnsupportedSchema(version));
    }
    if version < 1 {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql_error)?;
        transaction
            .execute_batch(MIGRATION_1)
            .map_err(map_sql_error)?;
        transaction
            .pragma_update(None, "user_version", 1)
            .map_err(map_sql_error)?;
        transaction.commit().map_err(map_sql_error)?;
    }
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(map_sql_error)?;
    if version < 2 {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql_error)?;
        transaction
            .execute_batch(MIGRATION_2)
            .map_err(map_sql_error)?;
        transaction
            .pragma_update(None, "user_version", 2)
            .map_err(map_sql_error)?;
        transaction.commit().map_err(map_sql_error)?;
    }
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(map_sql_error)?;
    if version < 3 {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql_error)?;
        transaction
            .execute_batch(MIGRATION_3)
            .map_err(map_sql_error)?;
        transaction
            .pragma_update(None, "user_version", 3)
            .map_err(map_sql_error)?;
        transaction.commit().map_err(map_sql_error)?;
    }
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(map_sql_error)?;
    if version < 4 {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql_error)?;
        transaction
            .execute_batch(MIGRATION_4)
            .map_err(map_sql_error)?;
        transaction
            .pragma_update(None, "user_version", 4)
            .map_err(map_sql_error)?;
        transaction.commit().map_err(map_sql_error)?;
    }
    Ok(())
}

fn validate_schema(connection: &Connection) -> Result<(), StoreError> {
    let quick_check: String = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(map_sql_error)?;
    if quick_check != "ok" {
        return Err(StoreError::Corrupt(quick_check));
    }
    for table in [
        "workflows",
        "source_candidates",
        "source_snapshots",
        "reviewed_memories",
        "preferences",
        "output_plans",
        "destination_plans",
        "destination_progress",
        "operation_intents",
        "receipts",
        "receipt_returned_ids",
    ] {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1
                 )",
                [table],
                |row| row.get(0),
            )
            .map_err(map_sql_error)?;
        if !exists {
            return Err(StoreError::InvalidRecord(format!(
                "database schema is missing table {table}"
            )));
        }
    }
    let mut foreign_keys = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(map_sql_error)?;
    if foreign_keys.exists([]).map_err(map_sql_error)? {
        return Err(StoreError::InvalidRecord(
            "database contains a foreign-key violation".to_owned(),
        ));
    }
    Ok(())
}

const MIGRATION_1: &str = r#"
CREATE TABLE workflows (
  day TEXT PRIMARY KEY,
  state TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK(revision >= 0),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  language TEXT,
  style TEXT NOT NULL,
  composition_path TEXT NOT NULL,
  model_material_stale INTEGER NOT NULL CHECK(model_material_stale IN (0, 1)),
  explicit_empty_selection INTEGER NOT NULL CHECK(explicit_empty_selection IN (0, 1)),
  selection_digest TEXT,
  invalidation_kind TEXT,
  invalidation_connector TEXT,
  invalidation_source_id TEXT,
  job_status TEXT,
  all_verified_at INTEGER,
  discarded_at INTEGER,
  entry_text TEXT NOT NULL,
  buffer_version INTEGER NOT NULL CHECK(buffer_version >= 0),
  confirmed_buffer_version INTEGER,
  confirmed_entry_digest TEXT,
  confirmed_at INTEGER,
  model_draft TEXT
) STRICT;
CREATE TABLE source_candidates (
  day TEXT NOT NULL REFERENCES workflows(day) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  connector TEXT NOT NULL,
  source_id TEXT NOT NULL,
  directory_day TEXT,
  header_day TEXT,
  warning TEXT,
  PRIMARY KEY(day, position),
  UNIQUE(day, connector, source_id)
) STRICT;
CREATE TABLE source_snapshots (
  day TEXT NOT NULL REFERENCES workflows(day) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  connector TEXT NOT NULL,
  source_id TEXT NOT NULL,
  file_identity TEXT NOT NULL,
  byte_size INTEGER NOT NULL CHECK(byte_size >= 0),
  modified_at INTEGER NOT NULL,
  content_digest TEXT NOT NULL,
  readable_body TEXT NOT NULL,
  PRIMARY KEY(day, position),
  UNIQUE(day, connector, source_id)
) STRICT;
CREATE TABLE reviewed_memories (
  day TEXT NOT NULL REFERENCES workflows(day) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  memory_text TEXT NOT NULL,
  PRIMARY KEY(day, position)
) STRICT;
CREATE TABLE preferences (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  last_language TEXT,
  last_composition_path TEXT NOT NULL
) STRICT;
INSERT INTO preferences(singleton, last_language, last_composition_path)
VALUES (1, NULL, 'hosted');
"#;

const MIGRATION_2: &str = r#"
CREATE TABLE output_plans (
  day TEXT PRIMARY KEY REFERENCES workflows(day) ON DELETE CASCADE,
  plan_id TEXT NOT NULL UNIQUE,
  entry_digest TEXT NOT NULL,
  selection_digest TEXT NOT NULL,
  plan_digest TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  approval_plan_digest TEXT,
  approval_revision INTEGER,
  approved_at INTEGER,
  approval_destination_only TEXT
) STRICT;
CREATE TABLE destination_plans (
  day TEXT NOT NULL REFERENCES output_plans(day) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  kind TEXT NOT NULL,
  destination TEXT NOT NULL,
  target_identity TEXT NOT NULL,
  intended_state_digest TEXT NOT NULL,
  payload_digest TEXT NOT NULL,
  inspected_state_digest TEXT NOT NULL,
  PRIMARY KEY(day, position),
  UNIQUE(day, destination)
) STRICT;
CREATE TABLE destination_progress (
  day TEXT NOT NULL REFERENCES output_plans(day) ON DELETE CASCADE,
  destination TEXT NOT NULL,
  status TEXT NOT NULL,
  PRIMARY KEY(day, destination)
) STRICT;
CREATE TABLE operation_intents (
  attempt_id TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL,
  day TEXT NOT NULL,
  destination TEXT NOT NULL,
  target_identity TEXT NOT NULL,
  entry_digest TEXT NOT NULL,
  selection_digest TEXT NOT NULL,
  payload_digest TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  cancellation_requested INTEGER NOT NULL CHECK(cancellation_requested IN (0, 1)),
  terminal TEXT
) STRICT;
CREATE INDEX operation_intents_day_unresolved
ON operation_intents(day) WHERE terminal IS NULL;
CREATE TABLE receipts (
  receipt_id TEXT PRIMARY KEY,
  attempt_id TEXT NOT NULL UNIQUE REFERENCES operation_intents(attempt_id),
  day TEXT NOT NULL,
  destination TEXT NOT NULL,
  target_identity TEXT NOT NULL,
  version INTEGER NOT NULL CHECK(version > 0),
  entry_digest TEXT NOT NULL,
  selection_digest TEXT NOT NULL,
  payload_digest TEXT NOT NULL,
  verified_at INTEGER NOT NULL
) STRICT;
CREATE TABLE receipt_returned_ids (
  receipt_id TEXT NOT NULL REFERENCES receipts(receipt_id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  returned_id TEXT NOT NULL,
  PRIMARY KEY(receipt_id, position)
) STRICT;
"#;

const MIGRATION_3: &str = r#"
ALTER TABLE source_candidates
ADD COLUMN availability TEXT NOT NULL DEFAULT 'unknown';
"#;

const MIGRATION_4: &str = r#"
ALTER TABLE source_candidates
ADD COLUMN source_note_name TEXT NOT NULL DEFAULT '';
UPDATE source_candidates SET source_note_name = source_id;

ALTER TABLE workflows
RENAME COLUMN model_material_stale TO assistant_material_stale;
ALTER TABLE workflows
RENAME COLUMN model_draft TO assistant_draft;
ALTER TABLE workflows DROP COLUMN composition_path;
ALTER TABLE workflows ADD COLUMN assistant_backend_id TEXT;
ALTER TABLE workflows ADD COLUMN composition_path TEXT;

ALTER TABLE preferences DROP COLUMN last_composition_path;
ALTER TABLE preferences ADD COLUMN last_assistant_backend_id TEXT;
"#;

fn workflow_exists(connection: &Connection, day: &DiaryDay) -> Result<bool, StoreError> {
    connection
        .query_row(
            "SELECT 1 FROM workflows WHERE day = ?1",
            [day.as_str()],
            |_| Ok(()),
        )
        .optional()
        .map(|value| value.is_some())
        .map_err(map_sql_error)
}

fn insert_workflow(
    transaction: &Transaction<'_>,
    record: &PersistedWorkflow,
) -> Result<(), StoreError> {
    let (kind, connector, source_id) = encode_invalidation(record.invalidation_reason.as_ref());
    let confirmation = record.entry.confirmed.as_ref();
    transaction
        .execute(
            "INSERT INTO workflows (
               day, state, revision, created_at, updated_at, language, style,
               assistant_backend_id, composition_path, assistant_material_stale,
               explicit_empty_selection,
               selection_digest, invalidation_kind, invalidation_connector,
               invalidation_source_id, job_status, all_verified_at, discarded_at,
               entry_text, buffer_version, confirmed_buffer_version,
               confirmed_entry_digest, confirmed_at, assistant_draft
             ) VALUES (
               ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
               ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
             )",
            params![
                record.day.as_str(),
                record.state.as_str(),
                to_i64(record.revision, "workflow revision")?,
                record.created_at.0,
                record.updated_at.0,
                record.language.map(WritingLanguage::as_str),
                record.style.as_str(),
                record
                    .assistant_selection
                    .as_ref()
                    .map(|selection| selection.backend_id.as_str()),
                record
                    .assistant_selection
                    .as_ref()
                    .map(|selection| selection.composition_path.as_str()),
                record.assistant_material_stale,
                record.explicit_empty_selection,
                record.selection_digest.as_ref().map(Digest::as_str),
                kind,
                connector,
                source_id,
                record.job_status.map(JobStatus::as_str),
                record.all_verified_at.map(|timestamp| timestamp.0),
                record.discarded_at.map(|timestamp| timestamp.0),
                record.entry.text,
                to_i64(record.entry.version, "buffer version")?,
                confirmation
                    .map(|value| to_i64(value.buffer_version, "confirmed buffer version"))
                    .transpose()?,
                confirmation.map(|value| value.entry_digest.as_str()),
                confirmation.map(|value| value.confirmed_at.0),
                record.assistant_draft,
            ],
        )
        .map_err(map_sql_error)?;
    insert_children(transaction, record)
}

fn replace_workflow(
    transaction: &Transaction<'_>,
    record: &PersistedWorkflow,
) -> Result<(), StoreError> {
    let (kind, connector, source_id) = encode_invalidation(record.invalidation_reason.as_ref());
    let confirmation = record.entry.confirmed.as_ref();
    let changed = transaction
        .execute(
            "UPDATE workflows SET
               state = ?2, revision = ?3, created_at = ?4, updated_at = ?5,
               language = ?6, style = ?7, assistant_backend_id = ?8,
               composition_path = ?9, assistant_material_stale = ?10,
               explicit_empty_selection = ?11, selection_digest = ?12,
               invalidation_kind = ?13, invalidation_connector = ?14,
               invalidation_source_id = ?15, job_status = ?16,
               all_verified_at = ?17, discarded_at = ?18, entry_text = ?19,
               buffer_version = ?20, confirmed_buffer_version = ?21,
               confirmed_entry_digest = ?22, confirmed_at = ?23,
               assistant_draft = ?24
             WHERE day = ?1",
            params![
                record.day.as_str(),
                record.state.as_str(),
                to_i64(record.revision, "workflow revision")?,
                record.created_at.0,
                record.updated_at.0,
                record.language.map(WritingLanguage::as_str),
                record.style.as_str(),
                record
                    .assistant_selection
                    .as_ref()
                    .map(|selection| selection.backend_id.as_str()),
                record
                    .assistant_selection
                    .as_ref()
                    .map(|selection| selection.composition_path.as_str()),
                record.assistant_material_stale,
                record.explicit_empty_selection,
                record.selection_digest.as_ref().map(Digest::as_str),
                kind,
                connector,
                source_id,
                record.job_status.map(JobStatus::as_str),
                record.all_verified_at.map(|timestamp| timestamp.0),
                record.discarded_at.map(|timestamp| timestamp.0),
                record.entry.text,
                to_i64(record.entry.version, "buffer version")?,
                confirmation
                    .map(|value| to_i64(value.buffer_version, "confirmed buffer version"))
                    .transpose()?,
                confirmation.map(|value| value.entry_digest.as_str()),
                confirmation.map(|value| value.confirmed_at.0),
                record.assistant_draft,
            ],
        )
        .map_err(map_sql_error)?;
    if changed != 1 {
        return Err(StoreError::InvalidRecord("workflow is missing".to_owned()));
    }
    transaction
        .execute(
            "DELETE FROM source_candidates WHERE day = ?1",
            [record.day.as_str()],
        )
        .map_err(map_sql_error)?;
    transaction
        .execute(
            "DELETE FROM source_snapshots WHERE day = ?1",
            [record.day.as_str()],
        )
        .map_err(map_sql_error)?;
    transaction
        .execute(
            "DELETE FROM reviewed_memories WHERE day = ?1",
            [record.day.as_str()],
        )
        .map_err(map_sql_error)?;
    transaction
        .execute(
            "DELETE FROM output_plans WHERE day = ?1",
            [record.day.as_str()],
        )
        .map_err(map_sql_error)?;
    insert_children(transaction, record)
}

fn insert_children(
    transaction: &Transaction<'_>,
    record: &PersistedWorkflow,
) -> Result<(), StoreError> {
    for (position, candidate) in record.candidates.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO source_candidates (
                   day, position, connector, source_id, directory_day, header_day, warning,
                   availability, source_note_name
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.day.as_str(),
                    to_i64(position, "candidate position")?,
                    candidate.source.connector,
                    candidate.source.source_id,
                    candidate
                        .date_evidence
                        .directory_day
                        .as_ref()
                        .map(DiaryDay::as_str),
                    candidate
                        .date_evidence
                        .header_day
                        .as_ref()
                        .map(DiaryDay::as_str),
                    candidate.date_evidence.warning,
                    candidate.availability.as_str(),
                    candidate.source_note_name,
                ],
            )
            .map_err(map_sql_error)?;
    }
    for (position, snapshot) in record.snapshots.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO source_snapshots (
                   day, position, connector, source_id, file_identity, byte_size,
                   modified_at, content_digest, readable_body
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.day.as_str(),
                    to_i64(position, "snapshot position")?,
                    snapshot.source.connector,
                    snapshot.source.source_id,
                    snapshot.stamp.file_identity,
                    to_i64(snapshot.stamp.size, "source byte size")?,
                    snapshot.stamp.modified_at.0,
                    snapshot.stamp.content_digest.as_str(),
                    snapshot.body,
                ],
            )
            .map_err(map_sql_error)?;
    }
    for (position, memory) in record.reviewed_memories.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO reviewed_memories (day, position, memory_text)
                 VALUES (?1, ?2, ?3)",
                params![
                    record.day.as_str(),
                    to_i64(position, "memory position")?,
                    memory
                ],
            )
            .map_err(map_sql_error)?;
    }
    if let Some(plan) = &record.output_plan {
        let approval = record.approval.as_ref();
        transaction
            .execute(
                "INSERT INTO output_plans (
                   day, plan_id, entry_digest, selection_digest, plan_digest,
                   created_at, approval_plan_digest, approval_revision,
                   approved_at, approval_destination_only
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    record.day.as_str(),
                    plan.plan_id,
                    plan.entry_digest.as_str(),
                    plan.selection_digest.as_str(),
                    plan.plan_digest.as_str(),
                    plan.created_at.0,
                    approval.map(|value| value.plan_digest.as_str()),
                    approval
                        .map(|value| to_i64(value.workflow_revision, "approval revision"))
                        .transpose()?,
                    approval.map(|value| value.approved_at.0),
                    approval.and_then(|value| value.destination_only.as_deref()),
                ],
            )
            .map_err(map_sql_error)?;
        for (position, target) in plan.targets.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO destination_plans (
                       day, position, kind, destination, target_identity,
                       intended_state_digest, payload_digest, inspected_state_digest
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        record.day.as_str(),
                        to_i64(position, "destination position")?,
                        target.kind.as_str(),
                        target.destination,
                        target.target_identity,
                        target.intended_state_digest.as_str(),
                        target.payload_digest.as_str(),
                        target.inspected_state_digest.as_str(),
                    ],
                )
                .map_err(map_sql_error)?;
        }
        for progress in &record.destination_progress {
            transaction
                .execute(
                    "INSERT INTO destination_progress (day, destination, status)
                     VALUES (?1, ?2, ?3)",
                    params![
                        record.day.as_str(),
                        progress.destination,
                        progress.status.as_str()
                    ],
                )
                .map_err(map_sql_error)?;
        }
    } else if record.approval.is_some() || !record.destination_progress.is_empty() {
        return Err(StoreError::InvalidRecord(
            "approval or progress exists without a plan".to_owned(),
        ));
    }
    Ok(())
}

fn load_workflow(connection: &Connection, day: &DiaryDay) -> Result<Option<Workflow>, StoreError> {
    type WorkflowRow = (
        String,
        i64,
        i64,
        i64,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        bool,
        bool,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        String,
        i64,
        Option<i64>,
        Option<String>,
        Option<i64>,
        Option<String>,
    );
    let row: Option<WorkflowRow> = connection
        .query_row(
            "SELECT state, revision, created_at, updated_at, language, style,
                    assistant_backend_id, composition_path, assistant_material_stale,
                    explicit_empty_selection,
                    selection_digest, invalidation_kind, invalidation_connector,
                    invalidation_source_id, job_status, all_verified_at, discarded_at,
                    entry_text, buffer_version, confirmed_buffer_version,
                    confirmed_entry_digest, confirmed_at, assistant_draft
             FROM workflows WHERE day = ?1",
            [day.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get(13)?,
                    row.get(14)?,
                    row.get(15)?,
                    row.get(16)?,
                    row.get(17)?,
                    row.get(18)?,
                    row.get(19)?,
                    row.get(20)?,
                    row.get(21)?,
                    row.get(22)?,
                ))
            },
        )
        .optional()
        .map_err(map_sql_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let candidates = load_candidates(connection, day)?;
    let snapshots = load_snapshots(connection, day)?;
    let reviewed_memories = load_memories(connection, day)?;
    let (output_plan, approval, destination_progress) = load_plan(connection, day)?;
    let entry_confirmation = match (row.19, row.20, row.21) {
        (None, None, None) => None,
        (Some(version), Some(digest), Some(confirmed_at)) => Some(EntryConfirmation {
            buffer_version: to_u64(version, "confirmed buffer version")?,
            entry_digest: parse_digest(digest)?,
            confirmed_at: Timestamp(confirmed_at),
        }),
        _ => {
            return Err(StoreError::InvalidRecord(
                "partial entry confirmation".to_owned(),
            ));
        }
    };
    let record = PersistedWorkflow {
        day: day.clone(),
        state: WorkflowState::parse(&row.0).map_err(invalid_value)?,
        revision: to_u64(row.1, "workflow revision")?,
        created_at: Timestamp(row.2),
        updated_at: Timestamp(row.3),
        language: row
            .4
            .as_deref()
            .map(WritingLanguage::parse)
            .transpose()
            .map_err(invalid_value)?,
        style: WritingStyle::parse(&row.5).map_err(invalid_value)?,
        assistant_selection: match (row.6, row.7) {
            (None, None) => None,
            (Some(backend_id), Some(path)) => Some(AssistantSelection::new(
                AssistantBackendId::parse(backend_id).map_err(invalid_value)?,
                CompositionPath::parse(&path).map_err(invalid_value)?,
            )),
            _ => {
                return Err(StoreError::InvalidRecord(
                    "partial assistant selection".to_owned(),
                ));
            }
        },
        assistant_material_stale: row.8,
        explicit_empty_selection: row.9,
        selection_digest: row.10.map(parse_digest).transpose()?,
        invalidation_reason: decode_invalidation(row.11, row.12, row.13)?,
        job_status: row
            .14
            .as_deref()
            .map(JobStatus::parse)
            .transpose()
            .map_err(invalid_value)?,
        all_verified_at: row.15.map(Timestamp),
        discarded_at: row.16.map(Timestamp),
        entry: EntryBuffer {
            text: row.17,
            version: to_u64(row.18, "buffer version")?,
            confirmed: entry_confirmation,
        },
        candidates,
        snapshots,
        reviewed_memories,
        assistant_draft: row.22,
        output_plan,
        approval,
        destination_progress,
    };
    Workflow::restore(record)
        .map(Some)
        .map_err(|error| StoreError::InvalidRecord(error.to_string()))
}

fn load_candidates(
    connection: &Connection,
    day: &DiaryDay,
) -> Result<Vec<SourceCandidate>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT connector, source_id, directory_day, header_day, warning, availability,
                    source_note_name
             FROM source_candidates WHERE day = ?1 ORDER BY position",
        )
        .map_err(map_sql_error)?;
    let mut rows = statement.query([day.as_str()]).map_err(map_sql_error)?;
    let mut candidates = Vec::new();
    while let Some(row) = rows.next().map_err(map_sql_error)? {
        let directory_day: Option<String> = row.get(2).map_err(map_sql_error)?;
        let header_day: Option<String> = row.get(3).map_err(map_sql_error)?;
        candidates.push(SourceCandidate {
            source: SourceRef::new(
                row.get::<_, String>(0).map_err(map_sql_error)?,
                row.get::<_, String>(1).map_err(map_sql_error)?,
            )
            .map_err(invalid_value)?,
            source_note_name: row.get(6).map_err(map_sql_error)?,
            date_evidence: DateEvidence {
                directory_day: directory_day
                    .map(DiaryDay::parse)
                    .transpose()
                    .map_err(invalid_value)?,
                header_day: header_day
                    .map(DiaryDay::parse)
                    .transpose()
                    .map_err(invalid_value)?,
                warning: row.get(4).map_err(map_sql_error)?,
            },
            availability: SourceAvailability::parse(
                &row.get::<_, String>(5).map_err(map_sql_error)?,
            )
            .map_err(invalid_value)?,
        });
    }
    Ok(candidates)
}

fn load_snapshots(
    connection: &Connection,
    day: &DiaryDay,
) -> Result<Vec<SourceSnapshot>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT connector, source_id, file_identity, byte_size, modified_at,
                    content_digest, readable_body
             FROM source_snapshots WHERE day = ?1 ORDER BY position",
        )
        .map_err(map_sql_error)?;
    let mut rows = statement.query([day.as_str()]).map_err(map_sql_error)?;
    let mut snapshots = Vec::new();
    while let Some(row) = rows.next().map_err(map_sql_error)? {
        let source = SourceRef::new(
            row.get::<_, String>(0).map_err(map_sql_error)?,
            row.get::<_, String>(1).map_err(map_sql_error)?,
        )
        .map_err(invalid_value)?;
        snapshots.push(SourceSnapshot {
            source,
            stamp: FreshnessStamp::new(
                row.get::<_, String>(2).map_err(map_sql_error)?,
                to_u64(row.get(3).map_err(map_sql_error)?, "source byte size")?,
                Timestamp(row.get(4).map_err(map_sql_error)?),
                parse_digest(row.get(5).map_err(map_sql_error)?)?,
            )
            .map_err(invalid_value)?,
            body: row.get(6).map_err(map_sql_error)?,
        });
    }
    Ok(snapshots)
}

fn load_memories(connection: &Connection, day: &DiaryDay) -> Result<Vec<String>, StoreError> {
    let mut statement = connection
        .prepare("SELECT memory_text FROM reviewed_memories WHERE day = ?1 ORDER BY position")
        .map_err(map_sql_error)?;
    statement
        .query_map([day.as_str()], |row| row.get::<_, String>(0))
        .map_err(map_sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql_error)
}

type LoadedPlan = (
    Option<OutputPlan>,
    Option<Approval>,
    Vec<DestinationProgress>,
);

fn load_plan(connection: &Connection, day: &DiaryDay) -> Result<LoadedPlan, StoreError> {
    type PlanRow = (
        String,
        String,
        String,
        String,
        i64,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<String>,
    );
    let row: Option<PlanRow> = connection
        .query_row(
            "SELECT plan_id, entry_digest, selection_digest, plan_digest, created_at,
                    approval_plan_digest, approval_revision, approved_at,
                    approval_destination_only
             FROM output_plans WHERE day = ?1",
            [day.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .optional()
        .map_err(map_sql_error)?;
    let Some(row) = row else {
        return Ok((None, None, Vec::new()));
    };
    let mut target_statement = connection
        .prepare(
            "SELECT kind, destination, target_identity, intended_state_digest,
                    payload_digest, inspected_state_digest
             FROM destination_plans WHERE day = ?1 ORDER BY position",
        )
        .map_err(map_sql_error)?;
    let mut target_rows = target_statement
        .query([day.as_str()])
        .map_err(map_sql_error)?;
    let mut targets = Vec::new();
    while let Some(target_row) = target_rows.next().map_err(map_sql_error)? {
        targets.push(
            DestinationPlan::new(
                DestinationKind::parse(&target_row.get::<_, String>(0).map_err(map_sql_error)?)
                    .map_err(invalid_value)?,
                target_row.get::<_, String>(1).map_err(map_sql_error)?,
                target_row.get::<_, String>(2).map_err(map_sql_error)?,
                parse_digest(target_row.get(3).map_err(map_sql_error)?)?,
                parse_digest(target_row.get(4).map_err(map_sql_error)?)?,
                parse_digest(target_row.get(5).map_err(map_sql_error)?)?,
            )
            .map_err(invalid_value)?,
        );
    }
    let plan = OutputPlan::new(
        row.0,
        parse_digest(row.1)?,
        parse_digest(row.2)?,
        targets,
        Timestamp(row.4),
    )
    .map_err(invalid_value)?;
    if plan.plan_digest != parse_digest(row.3)? {
        return Err(StoreError::InvalidRecord(
            "stored plan digest does not match its bindings".to_owned(),
        ));
    }
    let approval = match (row.5, row.6, row.7) {
        (None, None, None) => None,
        (Some(digest), Some(revision), Some(approved_at)) => Some(Approval {
            plan_digest: parse_digest(digest)?,
            workflow_revision: to_u64(revision, "approval revision")?,
            approved_at: Timestamp(approved_at),
            destination_only: row.8,
        }),
        _ => {
            return Err(StoreError::InvalidRecord(
                "partial approval binding".to_owned(),
            ));
        }
    };
    let mut progress_statement = connection
        .prepare(
            "SELECT destination, status FROM destination_progress
             WHERE day = ?1 ORDER BY destination",
        )
        .map_err(map_sql_error)?;
    let mut progress_rows = progress_statement
        .query([day.as_str()])
        .map_err(map_sql_error)?;
    let mut progress = Vec::new();
    while let Some(progress_row) = progress_rows.next().map_err(map_sql_error)? {
        progress.push(DestinationProgress {
            destination: progress_row.get(0).map_err(map_sql_error)?,
            status: DestinationStatus::parse(
                &progress_row.get::<_, String>(1).map_err(map_sql_error)?,
            )
            .map_err(invalid_value)?,
        });
    }
    Ok((Some(plan), approval, progress))
}

fn purge_day(
    transaction: &Transaction<'_>,
    day: &DiaryDay,
    now: Timestamp,
    _explicit: bool,
) -> Result<(), StoreError> {
    let state: Option<String> = transaction
        .query_row(
            "SELECT state FROM workflows WHERE day = ?1",
            [day.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sql_error)?;
    let Some(state) = state else {
        return Err(StoreError::InvalidRecord("workflow is missing".to_owned()));
    };
    let completed =
        WorkflowState::parse(&state).map_err(invalid_value)? == WorkflowState::Completed;
    transaction
        .execute(
            "UPDATE workflows SET
               state = CASE WHEN ?3 THEN state ELSE 'started' END,
               revision = revision + 1,
               updated_at = ?2,
               entry_text = '',
               buffer_version = buffer_version + 1,
               confirmed_buffer_version = NULL,
               confirmed_entry_digest = NULL,
               confirmed_at = NULL,
               assistant_draft = NULL,
               assistant_material_stale = 0,
               invalidation_kind = NULL,
               invalidation_connector = NULL,
               invalidation_source_id = NULL,
               job_status = CASE WHEN ?3 THEN job_status ELSE NULL END,
               discarded_at = ?2
             WHERE day = ?1",
            params![day.as_str(), now.0, completed],
        )
        .map_err(map_sql_error)?;
    transaction
        .execute(
            "UPDATE source_snapshots SET readable_body = '' WHERE day = ?1",
            [day.as_str()],
        )
        .map_err(map_sql_error)?;
    transaction
        .execute(
            "DELETE FROM reviewed_memories WHERE day = ?1",
            [day.as_str()],
        )
        .map_err(map_sql_error)?;
    transaction
        .execute("DELETE FROM output_plans WHERE day = ?1", [day.as_str()])
        .map_err(map_sql_error)?;
    if !completed {
        transaction
            .execute(
                "DELETE FROM source_candidates WHERE day = ?1",
                [day.as_str()],
            )
            .map_err(map_sql_error)?;
        transaction
            .execute(
                "DELETE FROM source_snapshots WHERE day = ?1",
                [day.as_str()],
            )
            .map_err(map_sql_error)?;
        transaction
            .execute(
                "UPDATE workflows SET selection_digest = NULL,
                                      explicit_empty_selection = 0
                 WHERE day = ?1",
                [day.as_str()],
            )
            .map_err(map_sql_error)?;
    }
    Ok(())
}

fn read_intent(row: &rusqlite::Row<'_>) -> Result<OperationIntent, StoreError> {
    let intent = OperationIntent {
        attempt_id: row.get(0).map_err(map_sql_error)?,
        plan_id: row.get(1).map_err(map_sql_error)?,
        day: DiaryDay::parse(row.get::<_, String>(2).map_err(map_sql_error)?)
            .map_err(invalid_value)?,
        destination: row.get(3).map_err(map_sql_error)?,
        target_identity: row.get(4).map_err(map_sql_error)?,
        entry_digest: parse_digest(row.get(5).map_err(map_sql_error)?)?,
        selection_digest: parse_digest(row.get(6).map_err(map_sql_error)?)?,
        payload_digest: parse_digest(row.get(7).map_err(map_sql_error)?)?,
        created_at: Timestamp(row.get(8).map_err(map_sql_error)?),
        cancellation_requested: row.get(9).map_err(map_sql_error)?,
    };
    intent.validate().map_err(invalid_value)?;
    Ok(intent)
}

fn load_intent(
    connection: &Connection,
    attempt_id: &str,
    unresolved_only: bool,
) -> Result<Option<OperationIntent>, StoreError> {
    let sql = if unresolved_only {
        "SELECT attempt_id, plan_id, day, destination, target_identity,
                entry_digest, selection_digest, payload_digest, created_at,
                cancellation_requested
         FROM operation_intents WHERE attempt_id = ?1 AND terminal IS NULL"
    } else {
        "SELECT attempt_id, plan_id, day, destination, target_identity,
                entry_digest, selection_digest, payload_digest, created_at,
                cancellation_requested
         FROM operation_intents WHERE attempt_id = ?1"
    };
    let mut statement = connection.prepare(sql).map_err(map_sql_error)?;
    let mut rows = statement.query([attempt_id]).map_err(map_sql_error)?;
    rows.next()
        .map_err(map_sql_error)?
        .map(read_intent)
        .transpose()
}

fn encode_invalidation(
    reason: Option<&InvalidationReason>,
) -> (Option<&'static str>, Option<&str>, Option<&str>) {
    match reason {
        None => (None, None, None),
        Some(InvalidationReason::SelectionChanged) => (Some("selection-changed"), None, None),
        Some(InvalidationReason::SourceChanged(source)) => (
            Some("source-changed"),
            Some(source.connector.as_str()),
            Some(source.source_id.as_str()),
        ),
        Some(InvalidationReason::FreshnessUnverifiable(source)) => (
            Some("freshness-unverifiable"),
            Some(source.connector.as_str()),
            Some(source.source_id.as_str()),
        ),
        Some(InvalidationReason::EntryEdited) => (Some("entry-edited"), None, None),
        Some(InvalidationReason::CompositionChoiceChanged) => {
            (Some("composition-choice-changed"), None, None)
        }
        Some(InvalidationReason::OutputBindingChanged) => {
            (Some("output-binding-changed"), None, None)
        }
    }
}

fn decode_invalidation(
    kind: Option<String>,
    connector: Option<String>,
    source_id: Option<String>,
) -> Result<Option<InvalidationReason>, StoreError> {
    let source = || -> Result<SourceRef, StoreError> {
        SourceRef::new(
            connector.clone().ok_or_else(|| {
                StoreError::InvalidRecord("missing invalidation connector".to_owned())
            })?,
            source_id.clone().ok_or_else(|| {
                StoreError::InvalidRecord("missing invalidation source".to_owned())
            })?,
        )
        .map_err(invalid_value)
    };
    match kind.as_deref() {
        None if connector.is_none() && source_id.is_none() => Ok(None),
        Some("selection-changed") => Ok(Some(InvalidationReason::SelectionChanged)),
        Some("source-changed") => Ok(Some(InvalidationReason::SourceChanged(source()?))),
        Some("freshness-unverifiable") => {
            Ok(Some(InvalidationReason::FreshnessUnverifiable(source()?)))
        }
        Some("entry-edited") => Ok(Some(InvalidationReason::EntryEdited)),
        Some("composition-choice-changed") => {
            Ok(Some(InvalidationReason::CompositionChoiceChanged))
        }
        Some("output-binding-changed") => Ok(Some(InvalidationReason::OutputBindingChanged)),
        _ => Err(StoreError::InvalidRecord(
            "unknown or partial invalidation reason".to_owned(),
        )),
    }
}

fn parse_digest(value: String) -> Result<Digest, StoreError> {
    Digest::parse(value).map_err(|error| StoreError::InvalidRecord(error.to_string()))
}

fn to_i64(value: impl TryInto<i64>, name: &'static str) -> Result<i64, StoreError> {
    value
        .try_into()
        .map_err(|_| StoreError::InvalidRecord(format!("{name} is out of range")))
}

fn to_u64(value: i64, name: &'static str) -> Result<u64, StoreError> {
    value
        .try_into()
        .map_err(|_| StoreError::InvalidRecord(format!("{name} is negative")))
}

fn to_u32(value: i64, name: &'static str) -> Result<u32, StoreError> {
    value
        .try_into()
        .map_err(|_| StoreError::InvalidRecord(format!("{name} is out of range")))
}

fn invalid_value(error: impl std::fmt::Display) -> StoreError {
    StoreError::InvalidRecord(error.to_string())
}

fn map_open_error(error: rusqlite::Error) -> StoreError {
    match &error {
        rusqlite::Error::SqliteFailure(code, _)
            if matches!(
                code.code,
                rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt
            ) =>
        {
            StoreError::Corrupt(error.to_string())
        }
        _ => StoreError::Unavailable(error.to_string()),
    }
}

fn map_sql_error(error: rusqlite::Error) -> StoreError {
    match &error {
        rusqlite::Error::SqliteFailure(code, _)
            if matches!(
                code.code,
                rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt
            ) =>
        {
            StoreError::Corrupt(error.to_string())
        }
        _ => StoreError::Unavailable(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};

    use diary_core::{
        Clock, DateEvidence, Destination, InspectionOutcome, PublicationCoordinator,
        PublicationResult, SourceCandidate, WriteOutcome, start_default_workflow,
    };

    use super::*;

    static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn empty_database_migrates_reopens_and_every_prior_schema_upgrades() {
        let scratch = Scratch::new("migration");
        let store = SqliteStore::open(scratch.db()).unwrap();
        drop(store);
        assert_eq!(schema_version(scratch.db()), CURRENT_SCHEMA);
        SqliteStore::open(scratch.db()).unwrap();

        let prior = Scratch::new("migration-v1");
        let connection = Connection::open(prior.db()).unwrap();
        connection.execute_batch(MIGRATION_1).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        drop(connection);
        SqliteStore::open(prior.db()).unwrap();
        assert_eq!(schema_version(prior.db()), CURRENT_SCHEMA);

        let prior = Scratch::new("migration-v2");
        let connection = Connection::open(prior.db()).unwrap();
        connection.execute_batch(MIGRATION_1).unwrap();
        connection.execute_batch(MIGRATION_2).unwrap();
        connection.pragma_update(None, "user_version", 2).unwrap();
        drop(connection);
        SqliteStore::open(prior.db()).unwrap();
        assert_eq!(schema_version(prior.db()), CURRENT_SCHEMA);

        let prior = Scratch::new("migration-v3-with-data");
        let connection = Connection::open(prior.db()).unwrap();
        connection.execute_batch(MIGRATION_1).unwrap();
        connection.execute_batch(MIGRATION_2).unwrap();
        connection.execute_batch(MIGRATION_3).unwrap();
        connection
            .execute(
                "INSERT INTO workflows (
                   day, state, revision, created_at, updated_at, language, style,
                   composition_path, model_material_stale, explicit_empty_selection,
                   selection_digest, invalidation_kind, invalidation_connector,
                   invalidation_source_id, job_status, all_verified_at, discarded_at,
                   entry_text, buffer_version, confirmed_buffer_version,
                   confirmed_entry_digest, confirmed_at, model_draft
                 ) VALUES (
                   '2026-10-29', 'discovered', 1, 1, 2, 'en-US', 'concise',
                   'manual', 0, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
                   'Recovered text', 0, NULL, NULL, NULL, 'Legacy draft'
                 )",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO source_candidates (
                   day, position, connector, source_id, directory_day, header_day,
                   warning, availability
                 ) VALUES (
                   '2026-10-29', 0, 'ayumi', '2026/10/29/note.md',
                   '2026-10-29', NULL, NULL, 'resident'
                 )",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO workflows (
                   day, state, revision, created_at, updated_at, language, style,
                   composition_path, model_material_stale, explicit_empty_selection,
                   selection_digest, invalidation_kind, invalidation_connector,
                   invalidation_source_id, job_status, all_verified_at, discarded_at,
                   entry_text, buffer_version, confirmed_buffer_version,
                   confirmed_entry_digest, confirmed_at, model_draft
                 ) VALUES (
                   '2026-10-28', 'started', 0, 1, 1, NULL, 'concise',
                   'hosted', 0, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
                   '', 0, NULL, NULL, NULL, NULL
                 )",
                [],
            )
            .unwrap();
        let preserved_digest = Digest::of_bytes(b"preserved-synthetic");
        connection
            .execute(
                "INSERT INTO source_snapshots (
                   day, position, connector, source_id, file_identity, byte_size,
                   modified_at, content_digest, readable_body
                 ) VALUES (
                   '2026-10-28', 0, 'fake', 'snapshot-1', 'file-1', 19,
                   2, ?1, 'Preserved snapshot.'
                 )",
                [preserved_digest.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO output_plans (
                   day, plan_id, entry_digest, selection_digest, plan_digest,
                   created_at, approval_plan_digest, approval_revision,
                   approved_at, approval_destination_only
                 ) VALUES (
                   '2026-10-28', 'plan-preserved', ?1, ?1, ?1,
                   3, NULL, NULL, NULL, NULL
                 )",
                [preserved_digest.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO destination_plans (
                   day, position, kind, destination, target_identity,
                   intended_state_digest, payload_digest, inspected_state_digest
                 ) VALUES (
                   '2026-10-28', 0, 'local', 'local', 'target-preserved',
                   ?1, ?1, ?1
                 )",
                [preserved_digest.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO destination_progress (day, destination, status)
                 VALUES ('2026-10-28', 'local', 'pending')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO operation_intents (
                   attempt_id, plan_id, day, destination, target_identity,
                   entry_digest, selection_digest, payload_digest, created_at,
                   cancellation_requested, terminal
                 ) VALUES (
                   'attempt-migrated', 'plan-migrated', '2026-10-29', 'local',
                   'target-migrated', ?1, ?1, ?1, 3, 0, NULL
                 )",
                [Digest::of_bytes(b"synthetic").as_str()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO operation_intents (
                   attempt_id, plan_id, day, destination, target_identity,
                   entry_digest, selection_digest, payload_digest, created_at,
                   cancellation_requested, terminal
                 ) VALUES (
                   'attempt-receipted', 'plan-receipted', '2026-10-29', 'notion',
                   'target-receipted', ?1, ?1, ?1, 4, 0, 'verified'
                 )",
                [Digest::of_bytes(b"receipted-synthetic").as_str()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO receipts (
                   receipt_id, attempt_id, day, destination, target_identity,
                   version, entry_digest, selection_digest, payload_digest, verified_at
                 ) VALUES (
                   'receipt-migrated', 'attempt-receipted', '2026-10-29', 'notion',
                   'target-receipted', 1, ?1, ?1, ?1, 5
                 )",
                [Digest::of_bytes(b"receipted-synthetic").as_str()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO receipt_returned_ids (receipt_id, position, returned_id)
                 VALUES ('receipt-migrated', 0, 'block-synthetic')",
                [],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 3).unwrap();
        drop(connection);

        let store = SqliteStore::open(prior.db()).unwrap();
        let migrated = store.load(&day("2026-10-29")).unwrap().unwrap();
        assert_eq!(migrated.state(), WorkflowState::Discovered);
        assert_eq!(migrated.entry().text, "Recovered text");
        assert_eq!(migrated.assistant_selection(), None);
        assert_eq!(
            migrated.candidates()[0].source_note_name,
            "2026/10/29/note.md"
        );
        assert_eq!(store.load_preferences().unwrap(), Preferences::default());
        assert_eq!(
            store
                .unresolved_intents(Some(&day("2026-10-29")))
                .unwrap()
                .len(),
            1
        );
        let receipts = store.receipts(&day("2026-10-29")).unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].receipt_id, "receipt-migrated");
        assert_eq!(receipts[0].returned_ids, ["block-synthetic"]);
        let connection = Connection::open(prior.db()).unwrap();
        let migrated_draft: String = connection
            .query_row(
                "SELECT assistant_draft FROM workflows WHERE day = '2026-10-29'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migrated_draft, "Legacy draft");
        let preserved: (String, String, String, String) = connection
            .query_row(
                "SELECT s.readable_body, p.plan_id, d.target_identity, g.status
                 FROM source_snapshots s
                 JOIN output_plans p ON p.day = s.day
                 JOIN destination_plans d ON d.day = p.day
                 JOIN destination_progress g ON g.day = p.day
                 WHERE s.day = '2026-10-28'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            preserved,
            (
                "Preserved snapshot.".to_owned(),
                "plan-preserved".to_owned(),
                "target-preserved".to_owned(),
                "pending".to_owned(),
            )
        );
        assert_eq!(schema_version(prior.db()), CURRENT_SCHEMA);
    }

    #[test]
    fn unsupported_and_corrupt_schema_fail_without_reset() {
        let newer = Scratch::new("newer-schema");
        let connection = Connection::open(newer.db()).unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        drop(connection);
        assert_eq!(
            SqliteStore::open(newer.db()).unwrap_err(),
            StoreError::UnsupportedSchema(99)
        );
        assert_eq!(schema_version(newer.db()), 99);

        let incomplete = Scratch::new("incomplete-schema");
        let connection = Connection::open(incomplete.db()).unwrap();
        connection
            .pragma_update(None, "user_version", CURRENT_SCHEMA)
            .unwrap();
        drop(connection);
        assert!(matches!(
            SqliteStore::open(incomplete.db()),
            Err(StoreError::InvalidRecord(_))
        ));
        assert_eq!(schema_version(incomplete.db()), CURRENT_SCHEMA);

        let corrupt = Scratch::new("corrupt");
        fs::write(corrupt.db(), b"not a sqlite database").unwrap();
        assert!(matches!(
            SqliteStore::open(corrupt.db()),
            Err(StoreError::Corrupt(_))
        ));
        assert_eq!(fs::read(corrupt.db()).unwrap(), b"not a sqlite database");
    }

    #[test]
    fn current_schema_rejects_partial_assistant_selection_and_invalid_source_note_name() {
        let partial = Scratch::new("partial-assistant-selection");
        let store = SqliteStore::open(partial.db()).unwrap();
        create_started(&store, "2026-10-29");
        drop(store);
        let connection = Connection::open(partial.db()).unwrap();
        connection
            .execute(
                "UPDATE workflows SET assistant_backend_id = 'hosted-default'
                 WHERE day = '2026-10-29'",
                [],
            )
            .unwrap();
        drop(connection);
        let store = SqliteStore::open(partial.db()).unwrap();
        assert_eq!(
            store.load(&day("2026-10-29")).unwrap_err(),
            StoreError::InvalidRecord("partial assistant selection".to_owned())
        );

        let invalid_name = Scratch::new("invalid-source-note-name");
        let store = SqliteStore::open(invalid_name.db()).unwrap();
        let mut workflow = create_started(&store, "2026-10-29");
        workflow
            .discover(
                workflow.revision(),
                vec![SourceCandidate {
                    source: SourceRef::new("fake", "note-1").unwrap(),
                    source_note_name: "note-1.md".to_owned(),
                    date_evidence: DateEvidence {
                        directory_day: Some(day("2026-10-29")),
                        header_day: None,
                        warning: None,
                    },
                    availability: SourceAvailability::Resident,
                }],
                Timestamp(1),
            )
            .unwrap();
        store.compare_and_swap(0, Some(0), &workflow).unwrap();
        drop(store);
        let connection = Connection::open(invalid_name.db()).unwrap();
        connection
            .execute(
                "UPDATE source_candidates SET source_note_name = ''
                 WHERE day = '2026-10-29'",
                [],
            )
            .unwrap();
        drop(connection);
        let store = SqliteStore::open(invalid_name.db()).unwrap();
        assert!(matches!(
            store.load(&day("2026-10-29")),
            Err(StoreError::InvalidRecord(_))
        ));
    }

    #[test]
    fn fixed_clock_starts_one_local_day_and_completed_day_is_preserved() {
        let scratch = Scratch::new("default-day");
        let store = SqliteStore::open(scratch.db()).unwrap();
        let clock = FixedClock {
            now: Timestamp(0),
            day: day("2026-10-29"),
        };
        assert!(matches!(
            start_default_workflow(&store, &clock).unwrap(),
            StartOutcome::Created(_)
        ));
        let resumed = match start_default_workflow(&store, &clock).unwrap() {
            StartOutcome::Resumed(workflow) => workflow,
            _ => panic!("unfinished day must resume"),
        };
        let mut completed = resumed;
        make_confirmed(&mut completed);
        make_completed(&mut completed, "default-day-plan", Timestamp(50));
        store.compare_and_swap(0, Some(0), &completed).unwrap();
        assert!(matches!(
            start_default_workflow(&store, &clock).unwrap(),
            StartOutcome::Completed(_)
        ));
    }

    #[test]
    fn preferences_and_full_workflow_reopen_without_prohibited_metadata() {
        let scratch = Scratch::new("roundtrip");
        let store = SqliteStore::open(scratch.db()).unwrap();
        store
            .save_preferences(&Preferences {
                last_language: Some(WritingLanguage::KoKr),
                last_assistant_backend_id: Some(
                    AssistantBackendId::parse("apple-foundation-models").unwrap(),
                ),
            })
            .unwrap();
        drop(store);

        let store = SqliteStore::open(scratch.db()).unwrap();
        assert_eq!(
            store.load_preferences().unwrap(),
            Preferences {
                last_language: Some(WritingLanguage::KoKr),
                last_assistant_backend_id: Some(
                    AssistantBackendId::parse("apple-foundation-models").unwrap(),
                ),
            }
        );
        let mut workflow = create_started(&store, "2026-10-28");
        make_confirmed(&mut workflow);
        workflow
            .set_assistant_selection(
                workflow.revision(),
                Some(AssistantSelection::new(
                    AssistantBackendId::parse("apple-foundation-models").unwrap(),
                    CompositionPath::AppleOnDevice,
                )),
                Timestamp(6),
            )
            .unwrap();
        workflow
            .confirm_entry(workflow.revision(), workflow.entry().version, Timestamp(6))
            .unwrap();
        let plan = OutputPlan::new(
            "persisted-plan",
            workflow
                .entry()
                .confirmed
                .as_ref()
                .unwrap()
                .entry_digest
                .clone(),
            workflow.selection_digest().unwrap().clone(),
            vec![target("local")],
            Timestamp(7),
        )
        .unwrap();
        workflow
            .preview_output(workflow.revision(), plan.clone(), Timestamp(7))
            .unwrap();
        workflow
            .approve(workflow.revision(), &plan.plan_digest, Timestamp(8))
            .unwrap();
        store.compare_and_swap(0, Some(0), &workflow).unwrap();
        let reopened = SqliteStore::open(scratch.db())
            .unwrap()
            .load(&day("2026-10-28"))
            .unwrap()
            .unwrap();
        assert_eq!(reopened, workflow);
        assert_eq!(reopened.entry().text, "An invented ordinary day.");

        let schema: String = Connection::open(scratch.db())
            .unwrap()
            .query_row(
                "SELECT group_concat(sql, ' ') FROM sqlite_master WHERE sql IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for prohibited in [
            "credential",
            "token",
            "front_matter",
            "location",
            "weather",
            "attachment",
            "audio",
            "notion_root",
        ] {
            assert!(!schema.to_ascii_lowercase().contains(prohibited));
        }
    }

    #[test]
    fn compare_and_swap_rejects_concurrent_workflow_and_buffer_updates() {
        let scratch = Scratch::new("concurrency");
        let store = SqliteStore::open(scratch.db()).unwrap();
        let workflow = create_started(&store, "2026-10-28");
        let mut first = workflow.clone();
        let mut second = workflow;
        first.discover(0, Vec::new(), Timestamp(1)).unwrap();
        second.discover(0, Vec::new(), Timestamp(1)).unwrap();
        store.compare_and_swap(0, None, &first).unwrap();
        assert_eq!(
            store.compare_and_swap(0, None, &second),
            Err(StoreError::StaleRevision)
        );

        let mut current = store.load(&day("2026-10-28")).unwrap().unwrap();
        current
            .confirm_sources(current.revision(), Vec::new(), true, Timestamp(2))
            .unwrap();
        current
            .begin_manual_composition(current.revision(), Timestamp(3))
            .unwrap();
        current
            .set_language(current.revision(), WritingLanguage::EnUs, Timestamp(4))
            .unwrap();
        store.compare_and_swap(1, Some(0), &current).unwrap();
        let mut editor_a = store.load(&day("2026-10-28")).unwrap().unwrap();
        let mut editor_b = editor_a.clone();
        let revision = editor_a.revision();
        editor_a
            .save_entry(0, "editor A".to_owned(), Timestamp(5))
            .unwrap();
        editor_b
            .save_entry(0, "editor B".to_owned(), Timestamp(5))
            .unwrap();
        store
            .compare_and_swap(revision, Some(0), &editor_a)
            .unwrap();
        assert_eq!(
            store.compare_and_swap(revision, Some(0), &editor_b),
            Err(StoreError::StaleBuffer)
        );
        let recovered = SqliteStore::open(scratch.db())
            .unwrap()
            .load(&day("2026-10-28"))
            .unwrap()
            .unwrap();
        assert_eq!(recovered.entry().text, "editor A");
        assert_eq!(recovered.entry().version, 1);

        let race = create_started(&store, "2026-10-30");
        let mut left = race.clone();
        let mut right = race;
        left.discover(0, Vec::new(), Timestamp(1)).unwrap();
        right.discover(0, Vec::new(), Timestamp(1)).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let left_store = store.clone();
        let left_barrier = Arc::clone(&barrier);
        let left_result = std::thread::spawn(move || {
            left_barrier.wait();
            left_store.compare_and_swap(0, None, &left)
        });
        let right_store = store.clone();
        let right_barrier = Arc::clone(&barrier);
        let right_result = std::thread::spawn(move || {
            right_barrier.wait();
            right_store.compare_and_swap(0, None, &right)
        });
        let results = [left_result.join().unwrap(), right_result.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(StoreError::StaleRevision)))
                .count(),
            1
        );
    }

    #[test]
    fn intent_precedes_effect_crash_recovery_is_per_day_and_duplicate_is_idempotent() {
        let scratch = Scratch::new("intent");
        let store = SqliteStore::open(scratch.db()).unwrap();
        let target = target("local");
        let intent = OperationIntent::new(
            "attempt-1",
            "plan-1",
            day("2026-10-28"),
            &target,
            Digest::of_bytes(b"entry"),
            Digest::of_bytes(b"selection"),
            Timestamp(1),
        )
        .unwrap();
        assert!(store.record_intent(&intent).unwrap());
        assert!(!store.record_intent(&intent).unwrap());
        let mut reused_attempt = intent.clone();
        reused_attempt.plan_id = "different-plan".to_owned();
        assert!(matches!(
            store.record_intent(&reused_attempt),
            Err(StoreError::InvalidRecord(_))
        ));
        let mismatched_receipt = Receipt {
            receipt_id: "bad-receipt".to_owned(),
            attempt_id: intent.attempt_id.clone(),
            day: day("2026-10-29"),
            destination: intent.destination.clone(),
            target_identity: intent.target_identity.clone(),
            version: 1,
            entry_digest: intent.entry_digest.clone(),
            selection_digest: intent.selection_digest.clone(),
            payload_digest: intent.payload_digest.clone(),
            returned_ids: Vec::new(),
            verified_at: Timestamp(1),
        };
        assert!(matches!(
            store.record_receipt(&mismatched_receipt),
            Err(StoreError::InvalidRecord(_))
        ));
        let mut destination = FakeDestination {
            store: store.clone(),
            effect_applied: false,
            inspection: InspectionOutcome::Matching {
                receipt_id: "receipt-1".to_owned(),
                returned_ids: vec!["external-1".to_owned()],
            },
        };
        assert!(matches!(
            destination.write(&intent),
            WriteOutcome::Verified { .. }
        ));
        drop(store);

        let reopened = SqliteStore::open(scratch.db()).unwrap();
        assert_eq!(
            reopened.reconciliation_gate(&day("2026-10-28")).unwrap(),
            ReconciliationGate::Reconciling
        );
        assert_eq!(
            reopened.reconciliation_gate(&day("2026-10-29")).unwrap(),
            ReconciliationGate::Clear
        );
        reopened.request_cancellation("attempt-1").unwrap();
        assert!(reopened.unresolved_intents(None).unwrap()[0].cancellation_requested);
        let result = PublicationCoordinator::reconcile(
            &reopened,
            &mut destination,
            &intent,
            1,
            Timestamp(2),
        )
        .unwrap();
        assert!(matches!(result, PublicationResult::Verified(_)));
        assert_eq!(
            reopened.reconciliation_gate(&day("2026-10-28")).unwrap(),
            ReconciliationGate::Clear
        );
        let duplicate =
            PublicationCoordinator::execute(&reopened, &mut destination, &intent, 1, Timestamp(3))
                .unwrap();
        assert_eq!(duplicate, PublicationResult::DuplicateCommand);
        assert_eq!(reopened.receipts(&day("2026-10-28")).unwrap().len(), 1);

        let absent_intent = OperationIntent::new(
            "attempt-absent",
            "plan-absent",
            day("2026-10-29"),
            &target,
            Digest::of_bytes(b"entry"),
            Digest::of_bytes(b"selection"),
            Timestamp(4),
        )
        .unwrap();
        assert!(reopened.record_intent(&absent_intent).unwrap());
        let mut absent_destination = FakeDestination {
            store: reopened.clone(),
            effect_applied: true,
            inspection: InspectionOutcome::Absent,
        };
        assert_eq!(
            PublicationCoordinator::reconcile(
                &reopened,
                &mut absent_destination,
                &absent_intent,
                1,
                Timestamp(5),
            )
            .unwrap(),
            PublicationResult::SafeToRetry
        );
        assert_eq!(
            reopened.reconciliation_gate(&absent_intent.day).unwrap(),
            ReconciliationGate::Clear
        );
    }

    #[test]
    fn retention_boundaries_restart_and_preserve_receipts_and_unresolved_intents() {
        let scratch = Scratch::new("retention");
        let store = SqliteStore::open(scratch.db()).unwrap();
        let mut workflow = create_started(&store, "2026-10-28");
        make_confirmed(&mut workflow);
        make_completed(&mut workflow, "plan-v1", Timestamp(100));
        store.compare_and_swap(0, Some(0), &workflow).unwrap();

        let first_intent = OperationIntent::new(
            "verified-v1",
            "plan-v1",
            day("2026-10-28"),
            &target("local"),
            Digest::of_bytes(b"entry-v1"),
            Digest::of_bytes(b"selection"),
            Timestamp(100),
        )
        .unwrap();
        store.record_intent(&first_intent).unwrap();
        store
            .record_receipt(&Receipt {
                receipt_id: "receipt-v1".to_owned(),
                attempt_id: first_intent.attempt_id.clone(),
                day: first_intent.day.clone(),
                destination: first_intent.destination.clone(),
                target_identity: first_intent.target_identity.clone(),
                version: 1,
                entry_digest: first_intent.entry_digest.clone(),
                selection_digest: first_intent.selection_digest.clone(),
                payload_digest: first_intent.payload_digest.clone(),
                returned_ids: vec!["local-v1".to_owned()],
                verified_at: Timestamp(100),
            })
            .unwrap();

        let current = store.load(&day("2026-10-28")).unwrap().unwrap();
        let mut amended = current.clone();
        amended
            .seed_amendment(
                amended.revision(),
                "An amended invented day.".to_owned(),
                Timestamp(150),
            )
            .unwrap();
        amended
            .confirm_entry(amended.revision(), amended.entry().version, Timestamp(160))
            .unwrap();
        make_completed(&mut amended, "plan-v2", Timestamp(200));
        store
            .compare_and_swap(current.revision(), Some(current.entry().version), &amended)
            .unwrap();
        let second_intent = OperationIntent::new(
            "verified-v2",
            "plan-v2",
            day("2026-10-28"),
            &target("local"),
            Digest::of_bytes(b"entry-v2"),
            Digest::of_bytes(b"selection"),
            Timestamp(200),
        )
        .unwrap();
        store.record_intent(&second_intent).unwrap();
        store
            .record_receipt(&Receipt {
                receipt_id: "receipt-v2".to_owned(),
                attempt_id: second_intent.attempt_id.clone(),
                day: second_intent.day.clone(),
                destination: second_intent.destination.clone(),
                target_identity: second_intent.target_identity.clone(),
                version: 2,
                entry_digest: second_intent.entry_digest.clone(),
                selection_digest: second_intent.selection_digest.clone(),
                payload_digest: second_intent.payload_digest.clone(),
                returned_ids: vec!["local-v2".to_owned()],
                verified_at: Timestamp(200),
            })
            .unwrap();
        assert!(
            store
                .purge_expired(Timestamp(100 + 7 * Timestamp::SECONDS_PER_DAY))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .purge_expired(Timestamp(200 + 7 * Timestamp::SECONDS_PER_DAY))
                .unwrap(),
            vec![day("2026-10-28")]
        );
        let recovered = store.load(&day("2026-10-28")).unwrap().unwrap();
        assert!(recovered.entry().text.is_empty());
        assert!(
            recovered
                .snapshots()
                .iter()
                .all(|snapshot| snapshot.body.is_empty())
        );
        let receipts = store.receipts(&day("2026-10-28")).unwrap();
        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts[0].version, 1);
        assert_eq!(receipts[1].version, 2);
        assert!(
            store
                .purge_expired(Timestamp(200 + 7 * Timestamp::SECONDS_PER_DAY))
                .unwrap()
                .is_empty()
        );

        let abandoned = create_started(&store, "2026-10-29");
        let intent = OperationIntent::new(
            "unresolved",
            "plan-x",
            abandoned.day().clone(),
            &target("remote"),
            Digest::of_bytes(b"abandoned-entry"),
            Digest::of_bytes(b"abandoned-selection"),
            Timestamp(0),
        )
        .unwrap();
        store.record_intent(&intent).unwrap();
        assert!(
            store
                .purge_expired(Timestamp(30 * Timestamp::SECONDS_PER_DAY - 1))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .purge_expired(Timestamp(30 * Timestamp::SECONDS_PER_DAY))
                .unwrap(),
            vec![day("2026-10-29")]
        );
        assert_eq!(
            store
                .unresolved_intents(Some(abandoned.day()))
                .unwrap()
                .len(),
            1
        );
        let purged_abandoned = store.load(abandoned.day()).unwrap().unwrap();
        store
            .discard(
                purged_abandoned.day(),
                purged_abandoned.revision(),
                Timestamp(99),
            )
            .unwrap();
        assert_eq!(
            store
                .unresolved_intents(Some(abandoned.day()))
                .unwrap()
                .len(),
            1
        );

        let restart_day = day("2026-10-30");
        let before_discard = create_started(&store, restart_day.as_str());
        store
            .discard(&restart_day, before_discard.revision(), Timestamp(10))
            .unwrap();
        let discarded = store.load(&restart_day).unwrap().unwrap();
        assert_eq!(discarded.revision(), before_discard.revision() + 1);
        assert_eq!(
            discarded.entry().version,
            before_discard.entry().version + 1
        );
        let mut restarted = discarded.clone();
        restarted
            .discover(restarted.revision(), Vec::new(), Timestamp(20))
            .unwrap();
        store
            .compare_and_swap(
                discarded.revision(),
                Some(discarded.entry().version),
                &restarted,
            )
            .unwrap();
        assert!(
            store
                .purge_expired(Timestamp(20 + 30 * Timestamp::SECONDS_PER_DAY - 1))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .purge_expired(Timestamp(20 + 30 * Timestamp::SECONDS_PER_DAY))
                .unwrap(),
            vec![restart_day]
        );
    }

    struct FixedClock {
        now: Timestamp,
        day: DiaryDay,
    }

    impl Clock for FixedClock {
        fn now(&self) -> Timestamp {
            self.now
        }

        fn local_day(&self) -> DiaryDay {
            self.day.clone()
        }
    }

    struct FakeDestination {
        store: SqliteStore,
        effect_applied: bool,
        inspection: InspectionOutcome,
    }

    impl Destination for FakeDestination {
        fn write(&mut self, intent: &OperationIntent) -> WriteOutcome {
            assert_eq!(
                self.store.reconciliation_gate(&intent.day).unwrap(),
                ReconciliationGate::Reconciling,
                "intent must be durable before the effect"
            );
            self.effect_applied = true;
            WriteOutcome::Verified {
                receipt_id: "crash-window-receipt".to_owned(),
                returned_ids: vec!["external-1".to_owned()],
            }
        }

        fn inspect(&mut self, _intent: &OperationIntent) -> InspectionOutcome {
            assert!(self.effect_applied);
            self.inspection.clone()
        }
    }

    fn create_started(store: &SqliteStore, raw_day: &str) -> Workflow {
        match store
            .start_or_resume(day(raw_day), Timestamp(0), &Preferences::default())
            .unwrap()
        {
            StartOutcome::Created(workflow) => workflow,
            _ => panic!("fixture day must be new"),
        }
    }

    fn make_confirmed(workflow: &mut Workflow) {
        let source = SourceRef::new("fake", "note-1").unwrap();
        workflow
            .discover(
                workflow.revision(),
                vec![SourceCandidate {
                    source: source.clone(),
                    source_note_name: "note-1.md".to_owned(),
                    date_evidence: DateEvidence {
                        directory_day: Some(workflow.day().clone()),
                        header_day: Some(workflow.day().clone()),
                        warning: None,
                    },
                    availability: SourceAvailability::Resident,
                }],
                Timestamp(1),
            )
            .unwrap();
        let body = "A synthetic note.";
        workflow
            .confirm_sources(
                workflow.revision(),
                vec![SourceSnapshot {
                    source,
                    stamp: FreshnessStamp::new(
                        "synthetic-file",
                        body.len() as u64,
                        Timestamp(1),
                        Digest::of_bytes(body.as_bytes()),
                    )
                    .unwrap(),
                    body: body.to_owned(),
                }],
                false,
                Timestamp(2),
            )
            .unwrap();
        workflow
            .begin_manual_composition(workflow.revision(), Timestamp(3))
            .unwrap();
        workflow
            .set_language(workflow.revision(), WritingLanguage::EnUs, Timestamp(4))
            .unwrap();
        workflow
            .save_entry(0, "An invented ordinary day.".to_owned(), Timestamp(5))
            .unwrap();
        workflow
            .confirm_entry(workflow.revision(), 1, Timestamp(6))
            .unwrap();
    }

    fn make_completed(workflow: &mut Workflow, plan_id: &str, now: Timestamp) {
        let plan = OutputPlan::new(
            plan_id,
            workflow
                .entry()
                .confirmed
                .as_ref()
                .unwrap()
                .entry_digest
                .clone(),
            workflow.selection_digest().unwrap().clone(),
            vec![target("local")],
            now,
        )
        .unwrap();
        workflow
            .preview_output(workflow.revision(), plan.clone(), now)
            .unwrap();
        workflow
            .approve(workflow.revision(), &plan.plan_digest, now)
            .unwrap();
        workflow
            .begin_publication(workflow.revision(), now)
            .unwrap();
        workflow
            .finish_publication(
                workflow.revision(),
                vec![DestinationProgress {
                    destination: "local".to_owned(),
                    status: DestinationStatus::Verified,
                }],
                now,
            )
            .unwrap();
    }

    fn target(name: &str) -> DestinationPlan {
        DestinationPlan::new(
            if name == "local" {
                DestinationKind::Local
            } else {
                DestinationKind::Remote
            },
            name,
            format!("{name}-target"),
            Digest::of_bytes(format!("{name}-state").as_bytes()),
            Digest::of_bytes(format!("{name}-payload").as_bytes()),
            Digest::of_bytes(format!("{name}-inspected").as_bytes()),
        )
        .unwrap()
    }

    fn schema_version(path: impl AsRef<Path>) -> i64 {
        Connection::open(path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    fn day(value: &str) -> DiaryDay {
        DiaryDay::parse(value).unwrap()
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            loop {
                let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "nd-p1-m4-{label}-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("scratch directory should be created: {error}"),
                }
            }
        }

        fn db(&self) -> PathBuf {
            self.0.join("private.sqlite3")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("failed to remove test scratch directory: {error}");
            }
        }
    }
}
