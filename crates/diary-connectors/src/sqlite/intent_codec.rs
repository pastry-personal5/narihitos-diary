//! SQLite intent codec helpers.

use super::*;

pub(super) fn read_intent(row: &rusqlite::Row<'_>) -> Result<OperationIntent, StoreError> {
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

pub(super) fn load_intent(
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
