//! SQLite retention helpers.

use super::*;

pub(super) fn purge_day(
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
    for table in [
        "assistant_memory_candidate_sets",
        "reviewed_memory_artifacts",
        "draft_proposals",
        "paragraph_suggestions",
        "source_refine_suggestions",
    ] {
        transaction
            .execute(
                &format!("DELETE FROM {table} WHERE day = ?1"),
                [day.as_str()],
            )
            .map_err(map_sql_error)?;
    }
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
