//! SQLite write records helpers.

use super::*;

pub(super) fn workflow_exists(connection: &Connection, day: &DiaryDay) -> Result<bool, StoreError> {
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

pub(super) fn insert_workflow(
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
               confirmed_entry_digest, confirmed_at, assistant_draft,
               amendment_file_digest, amendment_receipt_version,
               amendment_hand_edited
             ) VALUES (
               ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
               ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
               ?25, ?26, ?27
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
                record
                    .amendment
                    .as_ref()
                    .map(|seed| seed.file_digest.as_str()),
                record.amendment.as_ref().map(|seed| seed.receipt_version),
                record.amendment.as_ref().map(|seed| seed.hand_edited),
            ],
        )
        .map_err(map_sql_error)?;
    insert_children(transaction, record)
}

pub(super) fn replace_workflow(
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
               assistant_draft = ?24, amendment_file_digest = ?25,
               amendment_receipt_version = ?26, amendment_hand_edited = ?27
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
                record
                    .amendment
                    .as_ref()
                    .map(|seed| seed.file_digest.as_str()),
                record.amendment.as_ref().map(|seed| seed.receipt_version),
                record.amendment.as_ref().map(|seed| seed.hand_edited),
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
                [record.day.as_str()],
            )
            .map_err(map_sql_error)?;
    }
    transaction
        .execute(
            "DELETE FROM output_plans WHERE day = ?1",
            [record.day.as_str()],
        )
        .map_err(map_sql_error)?;
    insert_children(transaction, record)
}

pub(super) fn insert_children(
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
    insert_assistant_artifacts(transaction, record)?;
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

pub(super) fn insert_assistant_artifacts(
    transaction: &Transaction<'_>,
    record: &PersistedWorkflow,
) -> Result<(), StoreError> {
    for set in &record.memory_candidate_sets {
        transaction
            .execute(
                "INSERT INTO assistant_memory_candidate_sets (
                   day, artifact_id, backend_id, composition_path, created_at,
                   content_digest, source_selection_digest, stale
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    record.day.as_str(),
                    set.id,
                    set.provenance.backend_id.as_str(),
                    set.provenance.composition_path.as_str(),
                    set.created_at.0,
                    set.content_digest.as_str(),
                    set.binding.source_selection_digest.as_str(),
                    set.stale,
                ],
            )
            .map_err(map_sql_error)?;
        for (position, candidate) in set.candidates.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO assistant_memory_candidates (
                       day, artifact_id, position, candidate_id, candidate_text
                     ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        record.day.as_str(),
                        set.id,
                        to_i64(position, "assistant candidate position")?,
                        candidate.id,
                        candidate.text,
                    ],
                )
                .map_err(map_sql_error)?;
            for (source_position, digest) in candidate.source_snapshot_digests.iter().enumerate() {
                transaction
                    .execute(
                        "INSERT INTO assistant_memory_candidate_sources (
                           day, artifact_id, candidate_id, position, source_snapshot_digest
                         ) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            record.day.as_str(),
                            set.id,
                            candidate.id,
                            to_i64(source_position, "assistant candidate source position")?,
                            digest.as_str(),
                        ],
                    )
                    .map_err(map_sql_error)?;
            }
        }
    }
    if let Some(reviewed) = &record.reviewed_memory_artifact {
        transaction
            .execute(
                "INSERT INTO reviewed_memory_artifacts (
                   day, artifact_id, backend_id, composition_path, created_at,
                   content_digest, candidate_set_digest, memory_revision,
                   reviewed_text, stale
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    record.day.as_str(),
                    reviewed.id,
                    reviewed
                        .provenance
                        .as_ref()
                        .map(|value| value.backend_id.as_str()),
                    reviewed
                        .provenance
                        .as_ref()
                        .map(|value| value.composition_path.as_str()),
                    reviewed.created_at.0,
                    reviewed.content_digest.as_str(),
                    reviewed
                        .binding
                        .as_ref()
                        .map(|value| value.candidate_set_digest.as_str()),
                    to_i64(reviewed.revision, "reviewed memory revision")?,
                    reviewed.text,
                    reviewed.stale,
                ],
            )
            .map_err(map_sql_error)?;
    }
    for draft in &record.draft_proposals {
        let binding = draft.binding.as_ref();
        transaction
            .execute(
                "INSERT INTO draft_proposals (
                   day, artifact_id, backend_id, composition_path, created_at,
                   content_digest, source_selection_digest, language, style,
                   reviewed_memory_digest, reviewed_memory_revision,
                   captured_buffer_version, draft_text, stale, populated_buffer_version
                 ) VALUES (
                   ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                   ?13, ?14, ?15
                 )",
                params![
                    record.day.as_str(),
                    draft.id,
                    draft
                        .provenance
                        .as_ref()
                        .map(|value| value.backend_id.as_str()),
                    draft
                        .provenance
                        .as_ref()
                        .map(|value| value.composition_path.as_str()),
                    draft.created_at.0,
                    draft.content_digest.as_str(),
                    binding.map(|value| value.source_selection_digest.as_str()),
                    binding.map(|value| value.language.as_str()),
                    binding.map(|value| value.style.as_str()),
                    binding.map(|value| value.reviewed_memory_digest.as_str()),
                    binding
                        .map(|value| to_i64(
                            value.reviewed_memory_revision,
                            "reviewed memory revision"
                        ))
                        .transpose()?,
                    binding
                        .map(|value| to_i64(
                            value.captured_buffer_version,
                            "captured buffer version"
                        ))
                        .transpose()?,
                    draft.text,
                    draft.stale,
                    draft
                        .populated_buffer_version
                        .map(|value| to_i64(value, "populated buffer version"))
                        .transpose()?,
                ],
            )
            .map_err(map_sql_error)?;
        insert_ranges(
            transaction,
            "draft_reflection_ranges",
            "artifact_id",
            record.day.as_str(),
            &draft.id,
            &draft.reflection_ranges,
        )?;
    }
    for suggestion in &record.paragraph_suggestions {
        transaction
            .execute(
                "INSERT INTO paragraph_suggestions (
                   day, suggestion_id, backend_id, composition_path, created_at,
                   content_digest, source_selection_digest, language, style,
                   buffer_version, range_start, range_end, selected_text_digest,
                   replacement_text, explanation, stale, resolution, resolved_at
                 ) VALUES (
                   ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                   ?13, ?14, ?15, ?16, ?17, ?18
                 )",
                params![
                    record.day.as_str(),
                    suggestion.id,
                    suggestion.provenance.backend_id.as_str(),
                    suggestion.provenance.composition_path.as_str(),
                    suggestion.created_at.0,
                    suggestion.content_digest.as_str(),
                    suggestion.binding.source_selection_digest.as_str(),
                    suggestion.binding.language.as_str(),
                    suggestion.binding.style.as_str(),
                    to_i64(
                        suggestion.binding.buffer_version,
                        "suggestion buffer version"
                    )?,
                    to_i64(suggestion.binding.range.start, "suggestion range start")?,
                    to_i64(suggestion.binding.range.end, "suggestion range end")?,
                    suggestion.binding.selected_text_digest.as_str(),
                    suggestion.replacement,
                    suggestion.explanation,
                    suggestion.stale,
                    suggestion.resolution.map(SuggestionResolution::as_str),
                    suggestion.resolved_at.map(|value| value.0),
                ],
            )
            .map_err(map_sql_error)?;
        insert_ranges(
            transaction,
            "paragraph_reflection_ranges",
            "suggestion_id",
            record.day.as_str(),
            &suggestion.id,
            &suggestion.reflection_ranges,
        )?;
    }
    for suggestion in &record.source_refine_suggestions {
        transaction
            .execute(
                "INSERT INTO source_refine_suggestions (
               day, connector, source_id, original_body, suggested_body,
               binding_digest, backend_id, composition_path, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.day.as_str(),
                    suggestion.source.connector,
                    suggestion.source.source_id,
                    suggestion.original_body,
                    suggestion.suggested_body,
                    suggestion.binding_digest.as_str(),
                    suggestion.provenance.backend_id.as_str(),
                    suggestion.provenance.composition_path.as_str(),
                    suggestion.created_at.0
                ],
            )
            .map_err(map_sql_error)?;
    }
    Ok(())
}

pub(super) fn insert_ranges(
    transaction: &Transaction<'_>,
    table: &str,
    id_column: &str,
    day: &str,
    id: &str,
    ranges: &[ReflectionRange],
) -> Result<(), StoreError> {
    let sql = format!(
        "INSERT INTO {table} (day, {id_column}, position, start_byte, end_byte)
         VALUES (?1, ?2, ?3, ?4, ?5)"
    );
    for (position, range) in ranges.iter().enumerate() {
        transaction
            .execute(
                &sql,
                params![
                    day,
                    id,
                    to_i64(position, "reflection range position")?,
                    to_i64(range.start, "reflection range start")?,
                    to_i64(range.end, "reflection range end")?,
                ],
            )
            .map_err(map_sql_error)?;
    }
    Ok(())
}
