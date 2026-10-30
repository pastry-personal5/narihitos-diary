//! SQLite workflow store implementation.

use super::*;

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

pub(super) fn update_preferences(
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
