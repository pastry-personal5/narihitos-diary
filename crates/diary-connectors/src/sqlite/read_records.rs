//! SQLite read records helpers.

use super::*;

pub(super) fn load_workflow(
    connection: &Connection,
    day: &DiaryDay,
) -> Result<Option<Workflow>, StoreError> {
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
        Option<String>,
        Option<u32>,
        Option<bool>,
    );
    let row: Option<WorkflowRow> = connection
        .query_row(
            "SELECT state, revision, created_at, updated_at, language, style,
                    assistant_backend_id, composition_path, assistant_material_stale,
                    explicit_empty_selection,
                    selection_digest, invalidation_kind, invalidation_connector,
                    invalidation_source_id, job_status, all_verified_at, discarded_at,
                    entry_text, buffer_version, confirmed_buffer_version,
                    confirmed_entry_digest, confirmed_at, assistant_draft,
                    amendment_file_digest, amendment_receipt_version,
                    amendment_hand_edited
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
                    row.get(23)?,
                    row.get(24)?,
                    row.get(25)?,
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
    let memory_candidate_sets = load_memory_candidate_sets(connection, day)?;
    let reviewed_memory_artifact = load_reviewed_memory_artifact(connection, day)?;
    let draft_proposals = load_draft_proposals(connection, day)?;
    let paragraph_suggestions = load_paragraph_suggestions(connection, day)?;
    let source_refine_suggestions = load_source_refine_suggestions(connection, day)?;
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
    let amendment = match (row.23, row.24, row.25) {
        (None, None, None) => None,
        (Some(digest), Some(receipt_version), Some(hand_edited)) => Some(AmendmentSeed {
            file_digest: parse_digest(digest)?,
            receipt_version,
            hand_edited,
        }),
        _ => {
            return Err(StoreError::InvalidRecord(
                "partial amendment seed".to_owned(),
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
        memory_candidate_sets,
        reviewed_memory_artifact,
        draft_proposals,
        paragraph_suggestions,
        source_refine_suggestions,
        output_plan,
        approval,
        destination_progress,
        amendment,
    };
    Workflow::restore(record)
        .map(Some)
        .map_err(|error| StoreError::InvalidRecord(error.to_string()))
}

pub(super) fn load_candidates(
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

pub(super) fn load_snapshots(
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

pub(super) fn load_memories(
    connection: &Connection,
    day: &DiaryDay,
) -> Result<Vec<String>, StoreError> {
    let mut statement = connection
        .prepare("SELECT memory_text FROM reviewed_memories WHERE day = ?1 ORDER BY position")
        .map_err(map_sql_error)?;
    statement
        .query_map([day.as_str()], |row| row.get::<_, String>(0))
        .map_err(map_sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql_error)
}

pub(super) fn load_memory_candidate_sets(
    connection: &Connection,
    day: &DiaryDay,
) -> Result<Vec<MemoryCandidateSet>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT artifact_id, backend_id, composition_path, created_at,
                    content_digest, source_selection_digest, stale
             FROM assistant_memory_candidate_sets
             WHERE day = ?1 ORDER BY created_at, artifact_id",
        )
        .map_err(map_sql_error)?;
    let rows = statement
        .query_map([day.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, bool>(6)?,
            ))
        })
        .map_err(map_sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql_error)?;
    let mut sets = Vec::new();
    for (id, backend, path, created_at, digest, source_digest, stale) in rows {
        let provenance = required_provenance(backend, path)?;
        let content_digest = digest.map(parse_digest).transpose()?.ok_or_else(|| {
            StoreError::InvalidRecord("candidate set digest is missing".to_owned())
        })?;
        let source_selection_digest =
            source_digest
                .map(parse_digest)
                .transpose()?
                .ok_or_else(|| {
                    StoreError::InvalidRecord("candidate set binding is missing".to_owned())
                })?;
        let mut candidate_statement = connection
            .prepare(
                "SELECT candidate_id, candidate_text
                 FROM assistant_memory_candidates
                 WHERE day = ?1 AND artifact_id = ?2 ORDER BY position",
            )
            .map_err(map_sql_error)?;
        let candidate_rows = candidate_statement
            .query_map(params![day.as_str(), id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(map_sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_sql_error)?;
        let mut candidates = Vec::new();
        for (candidate_id, text) in candidate_rows {
            let mut source_statement = connection
                .prepare(
                    "SELECT source_snapshot_digest
                     FROM assistant_memory_candidate_sources
                     WHERE day = ?1 AND artifact_id = ?2 AND candidate_id = ?3
                     ORDER BY position",
                )
                .map_err(map_sql_error)?;
            let digests = source_statement
                .query_map(params![day.as_str(), id, candidate_id], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(map_sql_error)?
                .map(|value| value.map_err(map_sql_error).and_then(parse_digest))
                .collect::<Result<Vec<_>, _>>()?;
            candidates.push(MemoryCandidateArtifact {
                id: candidate_id,
                text,
                source_snapshot_digests: digests,
            });
        }
        sets.push(MemoryCandidateSet {
            id,
            provenance,
            created_at: Timestamp(created_at),
            content_digest,
            binding: MemoryCandidateBinding {
                source_selection_digest,
            },
            candidates,
            stale,
        });
    }
    Ok(sets)
}

pub(super) fn load_reviewed_memory_artifact(
    connection: &Connection,
    day: &DiaryDay,
) -> Result<Option<ReviewedMemoryArtifact>, StoreError> {
    let row = connection
        .query_row(
            "SELECT artifact_id, backend_id, composition_path, created_at,
                    content_digest, candidate_set_digest, memory_revision,
                    reviewed_text, stale
             FROM reviewed_memory_artifacts WHERE day = ?1",
            [day.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, bool>(8)?,
                ))
            },
        )
        .optional()
        .map_err(map_sql_error)?;
    let Some((id, backend, path, created_at, digest, binding, revision, text, stale)) = row else {
        return Ok(None);
    };
    Ok(Some(ReviewedMemoryArtifact {
        id,
        provenance: optional_provenance(backend, path)?,
        created_at: Timestamp(created_at),
        content_digest: digest
            .map(parse_digest)
            .transpose()?
            .unwrap_or_else(|| Digest::of_bytes(text.as_bytes())),
        binding: binding
            .map(parse_digest)
            .transpose()?
            .map(|candidate_set_digest| ReviewedMemoryBinding {
                candidate_set_digest,
            }),
        revision: to_u64(revision, "reviewed memory revision")?,
        text,
        stale,
    }))
}

pub(super) fn load_draft_proposals(
    connection: &Connection,
    day: &DiaryDay,
) -> Result<Vec<DraftProposal>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT artifact_id, backend_id, composition_path, created_at,
                    content_digest, source_selection_digest, language, style,
                    reviewed_memory_digest, reviewed_memory_revision,
                    captured_buffer_version, draft_text, stale,
                    populated_buffer_version
             FROM draft_proposals WHERE day = ?1 ORDER BY created_at, artifact_id",
        )
        .map_err(map_sql_error)?;
    let rows = statement
        .query_map([day.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, bool>(12)?,
                row.get::<_, Option<i64>>(13)?,
            ))
        })
        .map_err(map_sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql_error)?;
    let mut drafts = Vec::new();
    for row in rows {
        let provenance = optional_provenance(row.1, row.2)?;
        let binding = match (row.5, row.6, row.7, row.8, row.9, row.10) {
            (
                Some(source),
                Some(language),
                Some(style),
                Some(memory),
                Some(revision),
                Some(buffer),
            ) => {
                let backend_id = provenance
                    .as_ref()
                    .ok_or_else(|| {
                        StoreError::InvalidRecord("draft provenance is missing".to_owned())
                    })?
                    .backend_id
                    .clone();
                Some(DraftProposalBinding {
                    source_selection_digest: parse_digest(source)?,
                    language: WritingLanguage::parse(&language).map_err(invalid_value)?,
                    style: WritingStyle::parse(&style).map_err(invalid_value)?,
                    backend_id,
                    reviewed_memory_digest: parse_digest(memory)?,
                    reviewed_memory_revision: to_u64(revision, "reviewed memory revision")?,
                    captured_buffer_version: to_u64(buffer, "captured buffer version")?,
                })
            }
            (None, None, None, None, None, None) => None,
            _ => {
                return Err(StoreError::InvalidRecord(
                    "partial draft binding".to_owned(),
                ));
            }
        };
        drafts.push(DraftProposal {
            id: row.0.clone(),
            provenance,
            created_at: Timestamp(row.3),
            content_digest: row
                .4
                .map(parse_digest)
                .transpose()?
                .unwrap_or_else(|| Digest::of_bytes(row.11.as_bytes())),
            binding,
            text: row.11,
            reflection_ranges: load_ranges(
                connection,
                "draft_reflection_ranges",
                "artifact_id",
                day,
                &row.0,
            )?,
            stale: row.12,
            populated_buffer_version: row
                .13
                .map(|value| to_u64(value, "populated buffer version"))
                .transpose()?,
        });
    }
    Ok(drafts)
}

pub(super) fn load_paragraph_suggestions(
    connection: &Connection,
    day: &DiaryDay,
) -> Result<Vec<ParagraphSuggestion>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT suggestion_id, backend_id, composition_path, created_at,
                    content_digest, source_selection_digest, language, style,
                    buffer_version, range_start, range_end, selected_text_digest,
                    replacement_text, explanation, stale, resolution, resolved_at
             FROM paragraph_suggestions WHERE day = ?1 ORDER BY created_at, suggestion_id",
        )
        .map_err(map_sql_error)?;
    let rows = statement
        .query_map([day.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, bool>(14)?,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, Option<i64>>(16)?,
            ))
        })
        .map_err(map_sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql_error)?;
    rows.into_iter()
        .map(|row| {
            let id = row.0;
            let provenance = required_provenance(Some(row.1), Some(row.2))?;
            Ok(ParagraphSuggestion {
                id: id.clone(),
                provenance: provenance.clone(),
                created_at: Timestamp(row.3),
                content_digest: parse_digest(row.4)?,
                binding: ParagraphSuggestionBinding {
                    source_selection_digest: parse_digest(row.5)?,
                    language: WritingLanguage::parse(&row.6).map_err(invalid_value)?,
                    style: WritingStyle::parse(&row.7).map_err(invalid_value)?,
                    backend_id: provenance.backend_id,
                    buffer_version: to_u64(row.8, "suggestion buffer version")?,
                    range: ReflectionRange {
                        start: to_usize(row.9, "suggestion range start")?,
                        end: to_usize(row.10, "suggestion range end")?,
                    },
                    selected_text_digest: parse_digest(row.11)?,
                },
                replacement: row.12,
                reflection_ranges: load_ranges(
                    connection,
                    "paragraph_reflection_ranges",
                    "suggestion_id",
                    day,
                    &id,
                )?,
                explanation: row.13,
                stale: row.14,
                resolution: row
                    .15
                    .as_deref()
                    .map(SuggestionResolution::parse)
                    .transpose()
                    .map_err(invalid_value)?,
                resolved_at: row.16.map(Timestamp),
            })
        })
        .collect()
}

pub(super) fn load_source_refine_suggestions(
    connection: &Connection,
    day: &DiaryDay,
) -> Result<Vec<SourceRefineSuggestion>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT connector, source_id, original_body, suggested_body,
                binding_digest, backend_id, composition_path, created_at
         FROM source_refine_suggestions WHERE day = ?1 ORDER BY connector, source_id",
        )
        .map_err(map_sql_error)?;
    let rows = statement
        .query_map([day.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })
        .map_err(map_sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(SourceRefineSuggestion {
                source: SourceRef {
                    connector: row.0,
                    source_id: row.1,
                },
                original_body: row.2,
                suggested_body: row.3,
                binding_digest: parse_digest(row.4)?,
                provenance: required_provenance(Some(row.5), Some(row.6))?,
                created_at: Timestamp(row.7),
            })
        })
        .collect()
}

pub(super) fn load_ranges(
    connection: &Connection,
    table: &str,
    id_column: &str,
    day: &DiaryDay,
    id: &str,
) -> Result<Vec<ReflectionRange>, StoreError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT start_byte, end_byte FROM {table}
             WHERE day = ?1 AND {id_column} = ?2 ORDER BY position"
        ))
        .map_err(map_sql_error)?;
    statement
        .query_map(params![day.as_str(), id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(map_sql_error)?
        .map(|row| {
            let (start, end) = row.map_err(map_sql_error)?;
            Ok(ReflectionRange {
                start: to_usize(start, "reflection range start")?,
                end: to_usize(end, "reflection range end")?,
            })
        })
        .collect()
}

pub(super) fn optional_provenance(
    backend: Option<String>,
    path: Option<String>,
) -> Result<Option<ArtifactProvenance>, StoreError> {
    match (backend, path) {
        (None, None) => Ok(None),
        (Some(backend), Some(path)) => Ok(Some(ArtifactProvenance {
            backend_id: AssistantBackendId::parse(backend).map_err(invalid_value)?,
            composition_path: CompositionPath::parse(&path).map_err(invalid_value)?,
        })),
        _ => Err(StoreError::InvalidRecord(
            "partial assistant artifact provenance".to_owned(),
        )),
    }
}

pub(super) fn required_provenance(
    backend: Option<String>,
    path: Option<String>,
) -> Result<ArtifactProvenance, StoreError> {
    optional_provenance(backend, path)?.ok_or_else(|| {
        StoreError::InvalidRecord("assistant artifact provenance is missing".to_owned())
    })
}

type LoadedPlan = (
    Option<OutputPlan>,
    Option<Approval>,
    Vec<DestinationProgress>,
);

pub(super) fn load_plan(connection: &Connection, day: &DiaryDay) -> Result<LoadedPlan, StoreError> {
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
