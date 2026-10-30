use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use diary_core::{
    AmendmentSeed, Approval, ArtifactProvenance, AssistantBackendId, AssistantSelection,
    CompositionPath, DateEvidence, DestinationKind, DestinationPlan, DestinationProgress,
    DestinationStatus, DiaryDay, Digest, DraftProposal, DraftProposalBinding, EntryBuffer,
    EntryConfirmation, FreshnessStamp, IntentTerminal, InvalidationReason, JobStatus,
    MemoryCandidateArtifact, MemoryCandidateBinding, MemoryCandidateSet, OperationIntent,
    OperationLog, OutputPlan, ParagraphSuggestion, ParagraphSuggestionBinding, PersistedWorkflow,
    Preferences, Receipt, ReconciliationGate, ReflectionRange, ReviewedMemoryArtifact,
    ReviewedMemoryBinding, SourceAvailability, SourceCandidate, SourceRef, SourceRefineSuggestion,
    SourceSnapshot, StartOutcome, StoreError, SuggestionResolution, Timestamp, Workflow,
    WorkflowState, WorkflowStore, WorkflowSummary, WritingLanguage, WritingStyle,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

mod intent_codec;
mod operation_log;
mod read_records;
mod retention;
mod schema;
mod value_codec;
mod workflow_store;
mod write_records;
use intent_codec::*;
use read_records::*;
use retention::*;
use schema::*;
use value_codec::*;
use write_records::*;

/// One forward-only schema step. `version` is the `PRAGMA user_version` the
/// database holds once `sql` has been applied.
struct Migration {
    version: i64,
    sql: &'static str,
}

/// Every schema step, oldest first. **Append only**: a migration that has
/// shipped is never edited, because databases already past it will not run it
/// again. To change the schema, add `MIGRATION_{n+1}` here with
/// `version: n + 1`; `CURRENT_SCHEMA` follows from the last entry, and the
/// `migration_list_is_contiguous_from_one` test rejects gaps or reordering.
const MIGRATIONS: [Migration; 6] = [
    Migration {
        version: 1,
        sql: MIGRATION_1,
    },
    Migration {
        version: 2,
        sql: MIGRATION_2,
    },
    Migration {
        version: 3,
        sql: MIGRATION_3,
    },
    Migration {
        version: 4,
        sql: MIGRATION_4,
    },
    Migration {
        version: 5,
        sql: MIGRATION_5,
    },
    Migration {
        version: 6,
        sql: MIGRATION_6,
    },
];

const CURRENT_SCHEMA: i64 = MIGRATIONS[MIGRATIONS.len() - 1].version;

/// A content-free report of schema-migration progress: schema versions,
/// durations, and a fixed `reason` token on failure; never SQL, paths, or
/// stored data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationEvent {
    /// The database is already at the newest schema; nothing to apply.
    Current { version: i64 },
    /// Pending steps were found and are about to run.
    Started { from: i64, to: i64 },
    /// One step committed.
    StepApplied { from: i64, to: i64, elapsed_ms: u64 },
    /// Every pending step committed and the schema validated.
    Finished { from: i64, to: i64, elapsed_ms: u64 },
    /// Migration stopped. `from`/`to` name the step (or, for
    /// `unsupported-schema`, the stored and newest supported versions).
    /// `reason` is one of `unsupported-schema`, `step-failed`,
    /// `migration-gap`, or `schema-validation-failed`.
    Failed {
        from: i64,
        to: i64,
        elapsed_ms: u64,
        reason: &'static str,
    },
}

/// Receives [`MigrationEvent`]s synchronously while a store opens.
pub trait MigrationObserver {
    fn observe(&self, event: MigrationEvent);
}

struct NoMigrationObserver;

impl MigrationObserver for NoMigrationObserver {
    fn observe(&self, _event: MigrationEvent) {}
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[derive(Clone, Debug)]
pub struct SqliteStore {
    path: PathBuf,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_observed(path, &NoMigrationObserver)
    }

    /// Like [`Self::open`], reporting each migration step to `observer`.
    pub fn open_observed(
        path: impl AsRef<Path>,
        observer: &dyn MigrationObserver,
    ) -> Result<Self, StoreError> {
        let store = Self {
            path: path.as_ref().to_path_buf(),
        };
        let started = Instant::now();
        let mut connection = store.connect()?;
        run_migrations(&mut connection, &MIGRATIONS, observer)?;
        if let Err(error) = validate_schema(&connection) {
            let version = user_version(&connection).unwrap_or(CURRENT_SCHEMA);
            observer.observe(MigrationEvent::Failed {
                from: version,
                to: version,
                elapsed_ms: elapsed_ms(started),
                reason: "schema-validation-failed",
            });
            return Err(error);
        }
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

#[cfg(test)]
mod tests;
