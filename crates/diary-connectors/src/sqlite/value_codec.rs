//! SQLite value codec helpers.

use super::*;

pub(super) fn encode_invalidation(
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

pub(super) fn decode_invalidation(
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

pub(super) fn parse_digest(value: String) -> Result<Digest, StoreError> {
    Digest::parse(value).map_err(|error| StoreError::InvalidRecord(error.to_string()))
}

pub(super) fn to_i64(value: impl TryInto<i64>, name: &'static str) -> Result<i64, StoreError> {
    value
        .try_into()
        .map_err(|_| StoreError::InvalidRecord(format!("{name} is out of range")))
}

pub(super) fn to_u64(value: i64, name: &'static str) -> Result<u64, StoreError> {
    value
        .try_into()
        .map_err(|_| StoreError::InvalidRecord(format!("{name} is negative")))
}

pub(super) fn to_usize(value: i64, name: &'static str) -> Result<usize, StoreError> {
    value
        .try_into()
        .map_err(|_| StoreError::InvalidRecord(format!("{name} is out of range")))
}

pub(super) fn to_u32(value: i64, name: &'static str) -> Result<u32, StoreError> {
    value
        .try_into()
        .map_err(|_| StoreError::InvalidRecord(format!("{name} is out of range")))
}

pub(super) fn invalid_value(error: impl std::fmt::Display) -> StoreError {
    StoreError::InvalidRecord(error.to_string())
}

pub(super) fn map_open_error(error: rusqlite::Error) -> StoreError {
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

pub(super) fn map_sql_error(error: rusqlite::Error) -> StoreError {
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
