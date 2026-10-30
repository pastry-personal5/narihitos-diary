//! Forward-only schema migrations and validation.

use super::*;

pub(super) fn user_version(connection: &Connection) -> Result<i64, StoreError> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(map_sql_error)
}

/// Applies every step newer than the stored `user_version`, each in its own
/// immediate transaction together with its version bump, so a crash or a
/// failing step leaves the database at the last fully applied version.
pub(super) fn run_migrations(
    connection: &mut Connection,
    migrations: &[Migration],
    observer: &dyn MigrationObserver,
) -> Result<(), StoreError> {
    let started = Instant::now();
    let latest = migrations.last().map_or(0, |migration| migration.version);
    let initial = user_version(connection)?;
    if initial < 0 || initial > latest {
        observer.observe(MigrationEvent::Failed {
            from: initial,
            to: latest,
            elapsed_ms: elapsed_ms(started),
            reason: "unsupported-schema",
        });
        return Err(StoreError::UnsupportedSchema(initial));
    }
    if initial == latest {
        observer.observe(MigrationEvent::Current { version: initial });
        return Ok(());
    }
    observer.observe(MigrationEvent::Started {
        from: initial,
        to: latest,
    });
    for migration in migrations.iter().filter(|m| m.version > initial) {
        let step_started = Instant::now();
        match apply_step(connection, migration) {
            Ok(()) => observer.observe(MigrationEvent::StepApplied {
                from: migration.version - 1,
                to: migration.version,
                elapsed_ms: elapsed_ms(step_started),
            }),
            Err((error, reason)) => {
                observer.observe(MigrationEvent::Failed {
                    from: migration.version - 1,
                    to: migration.version,
                    elapsed_ms: elapsed_ms(step_started),
                    reason,
                });
                return Err(error);
            }
        }
    }
    observer.observe(MigrationEvent::Finished {
        from: initial,
        to: latest,
        elapsed_ms: elapsed_ms(started),
    });
    Ok(())
}

/// Runs one step. The stored version is re-read *inside* the immediate
/// transaction: another process may have applied this step since the caller
/// looked, and re-running it would fail on its own `CREATE TABLE`.
pub(super) fn apply_step(
    connection: &mut Connection,
    migration: &Migration,
) -> Result<(), (StoreError, &'static str)> {
    let step_failed = |error| (map_sql_error(error), "step-failed");
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(step_failed)?;
    let stored = user_version(&transaction).map_err(|error| (error, "step-failed"))?;
    if stored >= migration.version {
        return Ok(());
    }
    if stored != migration.version - 1 {
        return Err((
            StoreError::InvalidRecord("migration steps are not contiguous".to_owned()),
            "migration-gap",
        ));
    }
    transaction
        .execute_batch(migration.sql)
        .map_err(step_failed)?;
    transaction
        .pragma_update(None, "user_version", migration.version)
        .map_err(step_failed)?;
    transaction.commit().map_err(step_failed)
}

pub(super) fn validate_schema(connection: &Connection) -> Result<(), StoreError> {
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
        "assistant_memory_candidate_sets",
        "assistant_memory_candidates",
        "assistant_memory_candidate_sources",
        "reviewed_memory_artifacts",
        "draft_proposals",
        "draft_reflection_ranges",
        "paragraph_suggestions",
        "paragraph_reflection_ranges",
        "source_refine_suggestions",
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

pub(super) const MIGRATION_1: &str = r#"
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

pub(super) const MIGRATION_2: &str = r#"
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

pub(super) const MIGRATION_3: &str = r#"
ALTER TABLE source_candidates
ADD COLUMN availability TEXT NOT NULL DEFAULT 'unknown';
"#;

pub(super) const MIGRATION_4: &str = r#"
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

pub(super) const MIGRATION_6: &str = r#"
ALTER TABLE workflows ADD COLUMN amendment_file_digest TEXT;
ALTER TABLE workflows ADD COLUMN amendment_receipt_version INTEGER;
ALTER TABLE workflows ADD COLUMN amendment_hand_edited INTEGER;
"#;

pub(super) const MIGRATION_5: &str = r#"
CREATE TABLE assistant_memory_candidate_sets (
  day TEXT NOT NULL REFERENCES workflows(day) ON DELETE CASCADE,
  artifact_id TEXT NOT NULL,
  backend_id TEXT,
  composition_path TEXT,
  created_at INTEGER NOT NULL,
  content_digest TEXT,
  source_selection_digest TEXT,
  stale INTEGER NOT NULL CHECK(stale IN (0, 1)),
  PRIMARY KEY(day, artifact_id)
) STRICT;
CREATE TABLE assistant_memory_candidates (
  day TEXT NOT NULL,
  artifact_id TEXT NOT NULL,
  position INTEGER NOT NULL CHECK(position >= 0),
  candidate_id TEXT NOT NULL,
  candidate_text TEXT NOT NULL,
  PRIMARY KEY(day, artifact_id, position),
  UNIQUE(day, artifact_id, candidate_id),
  FOREIGN KEY(day, artifact_id)
    REFERENCES assistant_memory_candidate_sets(day, artifact_id) ON DELETE CASCADE
) STRICT;
CREATE TABLE assistant_memory_candidate_sources (
  day TEXT NOT NULL,
  artifact_id TEXT NOT NULL,
  candidate_id TEXT NOT NULL,
  position INTEGER NOT NULL CHECK(position >= 0),
  source_snapshot_digest TEXT NOT NULL,
  PRIMARY KEY(day, artifact_id, candidate_id, position),
  FOREIGN KEY(day, artifact_id, candidate_id)
    REFERENCES assistant_memory_candidates(day, artifact_id, candidate_id) ON DELETE CASCADE
) STRICT;
CREATE TABLE reviewed_memory_artifacts (
  day TEXT PRIMARY KEY REFERENCES workflows(day) ON DELETE CASCADE,
  artifact_id TEXT NOT NULL UNIQUE,
  backend_id TEXT,
  composition_path TEXT,
  created_at INTEGER NOT NULL,
  content_digest TEXT,
  candidate_set_digest TEXT,
  memory_revision INTEGER NOT NULL CHECK(memory_revision >= 0),
  reviewed_text TEXT NOT NULL,
  stale INTEGER NOT NULL CHECK(stale IN (0, 1))
) STRICT;
CREATE TABLE draft_proposals (
  day TEXT NOT NULL REFERENCES workflows(day) ON DELETE CASCADE,
  artifact_id TEXT NOT NULL,
  backend_id TEXT,
  composition_path TEXT,
  created_at INTEGER NOT NULL,
  content_digest TEXT,
  source_selection_digest TEXT,
  language TEXT,
  style TEXT,
  reviewed_memory_digest TEXT,
  reviewed_memory_revision INTEGER,
  captured_buffer_version INTEGER,
  draft_text TEXT NOT NULL,
  stale INTEGER NOT NULL CHECK(stale IN (0, 1)),
  populated_buffer_version INTEGER,
  PRIMARY KEY(day, artifact_id)
) STRICT;
CREATE TABLE draft_reflection_ranges (
  day TEXT NOT NULL,
  artifact_id TEXT NOT NULL,
  position INTEGER NOT NULL CHECK(position >= 0),
  start_byte INTEGER NOT NULL CHECK(start_byte >= 0),
  end_byte INTEGER NOT NULL CHECK(end_byte > start_byte),
  PRIMARY KEY(day, artifact_id, position),
  FOREIGN KEY(day, artifact_id)
    REFERENCES draft_proposals(day, artifact_id) ON DELETE CASCADE
) STRICT;
CREATE TABLE paragraph_suggestions (
  day TEXT NOT NULL REFERENCES workflows(day) ON DELETE CASCADE,
  suggestion_id TEXT NOT NULL,
  backend_id TEXT NOT NULL,
  composition_path TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  content_digest TEXT NOT NULL,
  source_selection_digest TEXT NOT NULL,
  language TEXT NOT NULL,
  style TEXT NOT NULL,
  buffer_version INTEGER NOT NULL CHECK(buffer_version >= 0),
  range_start INTEGER NOT NULL CHECK(range_start >= 0),
  range_end INTEGER NOT NULL CHECK(range_end > range_start),
  selected_text_digest TEXT NOT NULL,
  replacement_text TEXT NOT NULL,
  explanation TEXT,
  stale INTEGER NOT NULL CHECK(stale IN (0, 1)),
  resolution TEXT CHECK(resolution IS NULL OR resolution IN ('accepted', 'rejected')),
  resolved_at INTEGER,
  PRIMARY KEY(day, suggestion_id)
) STRICT;
CREATE TABLE paragraph_reflection_ranges (
  day TEXT NOT NULL,
  suggestion_id TEXT NOT NULL,
  position INTEGER NOT NULL CHECK(position >= 0),
  start_byte INTEGER NOT NULL CHECK(start_byte >= 0),
  end_byte INTEGER NOT NULL CHECK(end_byte > start_byte),
  PRIMARY KEY(day, suggestion_id, position),
  FOREIGN KEY(day, suggestion_id)
    REFERENCES paragraph_suggestions(day, suggestion_id) ON DELETE CASCADE
) STRICT;
CREATE TABLE source_refine_suggestions (
  day TEXT NOT NULL REFERENCES workflows(day) ON DELETE CASCADE,
  connector TEXT NOT NULL,
  source_id TEXT NOT NULL,
  original_body TEXT NOT NULL,
  suggested_body TEXT NOT NULL,
  binding_digest TEXT NOT NULL,
  backend_id TEXT NOT NULL,
  composition_path TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(day, connector, source_id)
) STRICT;

INSERT INTO reviewed_memory_artifacts (
  day, artifact_id, backend_id, composition_path, created_at, content_digest,
  candidate_set_digest, memory_revision, reviewed_text, stale
)
SELECT
  w.day, 'migrated-memory-' || replace(w.day, '-', ''),
  w.assistant_backend_id, w.composition_path, w.updated_at, NULL, NULL, 0,
  (SELECT group_concat(memory_text, char(10)) FROM (
     SELECT memory_text FROM reviewed_memories rm
     WHERE rm.day = w.day ORDER BY position
   )),
  1
FROM workflows w
WHERE EXISTS (SELECT 1 FROM reviewed_memories rm WHERE rm.day = w.day);

INSERT INTO draft_proposals (
  day, artifact_id, backend_id, composition_path, created_at, content_digest,
  source_selection_digest, language, style, reviewed_memory_digest,
  reviewed_memory_revision, captured_buffer_version, draft_text, stale,
  populated_buffer_version
)
SELECT
  day, 'migrated-draft-' || replace(day, '-', ''), assistant_backend_id,
  composition_path, updated_at, NULL, NULL, NULL, NULL,
  NULL, NULL, NULL, assistant_draft, 1, NULL
FROM workflows
WHERE assistant_draft IS NOT NULL;
"#;
