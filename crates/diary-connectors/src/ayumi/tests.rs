use super::*;
use std::{
    fs,
    fs::FileTimes,
    os::unix::fs::{PermissionsExt, symlink},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

static TEMP_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

fn temp() -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("nd-ayumi-{}-{sequence}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("Ayumi/2026/10/28")).unwrap();
    path
}
#[test]
fn bounded_discovery_parses_dates_and_filters_audio() {
    let root = temp();
    fs::write(root.join("Ayumi/2026/10/28/n.md"), "---\r\ncreated_at: 2026-10-27T23:00:00-09:00\r\n---\r\nkeep\r\n[a](secret.m4a)\r\n[x](https://x)").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let found = catalog
        .discover(&DiaryDay::parse("2026-10-29").unwrap())
        .unwrap();
    assert_eq!(
        found,
        catalog
            .discover(&DiaryDay::parse("2026-10-29").unwrap())
            .unwrap()
    );
    assert_eq!(found.candidates.len(), 1);
    assert!(found.candidates[0].date_evidence.warning.is_some());
    let ReadOutcome::Read(snapshot) = catalog.read_selected(&found.candidates[0].source).unwrap()
    else {
        panic!()
    };
    assert_eq!(snapshot.snapshot.body, "keep\n[x](https://x)");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn discovery_is_bounded_across_years_and_reports_unsupported_entries() {
    let root = temp();
    for relative in ["2025/12/31", "2026/01/01", "2026/01/02", "2026/01/03"] {
        fs::create_dir_all(root.join("Ayumi").join(relative)).unwrap();
    }
    fs::write(root.join("Ayumi/2025/12/31/a.md"), "a").unwrap();
    fs::write(root.join("Ayumi/2026/01/01/b.md"), "b").unwrap();
    fs::write(root.join("Ayumi/2026/01/02/c.md"), "c").unwrap();
    fs::write(root.join("Ayumi/2026/01/02/not-markdown.txt"), "x").unwrap();
    fs::create_dir(root.join("Ayumi/2026/01/02/not-a-file.md")).unwrap();
    fs::write(root.join("Ayumi/2026/01/03/outside.md"), "outside").unwrap();

    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let found = catalog
        .discover(&DiaryDay::parse("2026-01-01").unwrap())
        .unwrap();
    let ids: Vec<_> = found
        .candidates
        .iter()
        .map(|candidate| candidate.source.source_id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["2025/12/31/a.md", "2026/01/01/b.md", "2026/01/02/c.md"]
    );
    assert_eq!(
        found.warnings,
        [
            "source entry is not a regular file",
            "source entry is not a Markdown file"
        ]
    );
    let _ = fs::remove_dir_all(root);
}
#[test]
fn rejects_escapes_and_detects_digest_change() {
    let root = temp();
    for invalid in ["", ".", "..", "nested/Ayumi", "nested\\Ayumi"] {
        assert!(matches!(
            AyumiCatalog::new(&root, invalid),
            Err(SourceReadError::InvalidDirectoryName)
        ));
    }
    fs::write(root.join("Ayumi/2026/10/28/n.md"), "one").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();
    let ReadOutcome::Read(snapshot) = catalog.read_selected(&source).unwrap() else {
        panic!()
    };
    assert!(snapshot.snapshot.stamp.file_identity.contains(':'));
    let original_modified = fs::metadata(root.join("Ayumi/2026/10/28/n.md"))
        .unwrap()
        .modified()
        .unwrap();
    fs::write(root.join("Ayumi/2026/10/28/n.md"), "two").unwrap();
    fs::File::options()
        .write(true)
        .open(root.join("Ayumi/2026/10/28/n.md"))
        .unwrap()
        .set_times(FileTimes::new().set_modified(original_modified))
        .unwrap();
    assert_eq!(
        catalog.revalidate(&snapshot.snapshot).unwrap(),
        ReadOutcome::Changed
    );
    let ReadOutcome::Read(current) = catalog.read_selected(&source).unwrap() else {
        panic!()
    };
    fs::write(root.join("Ayumi/2026/10/28/replacement"), "two").unwrap();
    fs::rename(
        root.join("Ayumi/2026/10/28/replacement"),
        root.join("Ayumi/2026/10/28/n.md"),
    )
    .unwrap();
    assert_eq!(
        catalog.revalidate(&current.snapshot).unwrap(),
        ReadOutcome::Changed
    );
    let ReadOutcome::Read(current) = catalog.read_selected(&source).unwrap() else {
        panic!()
    };
    fs::remove_file(root.join("Ayumi/2026/10/28/n.md")).unwrap();
    assert_eq!(
        catalog.revalidate(&current.snapshot).unwrap(),
        ReadOutcome::Unverifiable
    );
    assert_eq!(
        catalog.read_selected(&SourceRef::new(CONNECTOR, "../x.md").unwrap()),
        Err(SourceReadError::UnknownSource)
    );
    assert_eq!(
        catalog.read_selected(&SourceRef::new(CONNECTOR, "private/n.md").unwrap()),
        Err(SourceReadError::UnknownSource)
    );
    assert_eq!(
        catalog.read_selected(&SourceRef::new(CONNECTOR, "2026/13/28/hidden.md").unwrap()),
        Err(SourceReadError::UnknownSource)
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parser_preserves_safe_body_and_validates_complete_offset_timestamp() {
    let parsed = parse_markdown(
            b"\xef\xbb\xbf---\r\ncreated_at: '2026-10-28T23:59:60+09:00'\r\nbroken metadata\r\n---\r\nkeep\r\n[remote](https://example.test/a.mp3)\r\n",
        )
        .unwrap();
    assert_eq!(parsed.header_day.unwrap().as_str(), "2026-10-28");
    assert_eq!(parsed.body, "keep\n[remote](https://example.test/a.mp3)");
    assert_eq!(parsed.warning.as_deref(), Some("front matter is malformed"));
    assert!(!parsed.ambiguous_attachment);

    let invalid = parse_markdown(b"---\ncreated_at: 2026-10-28T25:00:00+09:00\n---\nbody").unwrap();
    assert!(invalid.header_day.is_none());
    assert_eq!(
        invalid.warning.as_deref(),
        Some("created_at is invalid or lacks an offset")
    );

    let empty = parse_markdown(b"---\ncreated_at: 2026-10-28T00:00:00Z\n---").unwrap();
    assert_eq!(empty.body, "");
    assert!(!empty.ambiguous_attachment);
    let unclosed = parse_markdown(b"---\ncreated_at: secret\nbody").unwrap();
    assert!(unclosed.ambiguous_attachment);
    assert_eq!(unclosed.body, "");

    let absent = parse_markdown(b"ordinary body\n").unwrap();
    assert_eq!(absent.body, "ordinary body");
    assert!(absent.header_day.is_none());
    assert!(absent.warning.is_none());
    let unknown_fields = parse_markdown(b"---\nweather: clear\n---\nbody").unwrap();
    assert_eq!(unknown_fields.body, "body");
    assert!(unknown_fields.warning.is_none());
    assert!(parse_markdown(&[0xff, 0xfe]).is_none());
    assert!(parse_markdown(b"body\rwith a legacy separator").is_none());

    let nested =
        parse_markdown(b"---\nmetadata:\n  created_at: 2026-10-28T00:00:00Z\n---\nbody").unwrap();
    assert!(nested.header_day.is_none());

    let duplicate = parse_markdown(
        b"---\ncreated_at: 2026-10-28T00:00:00Z\ncreated_at: 2026-10-29T00:00:00Z\n---\nbody",
    )
    .unwrap();
    assert!(duplicate.header_day.is_none());
    assert_eq!(
        duplicate.warning.as_deref(),
        Some("front matter is malformed")
    );
}

#[test]
fn parser_removes_only_standalone_local_audio_and_withholds_ambiguous_paths() {
    let safe = parse_markdown(
            b"prose\n[audio](recording.M4A?download=1)\n[page](notes.md)\n[remote](https://example.test/recording.wav)",
        )
        .unwrap();
    assert_eq!(
        safe.body,
        "prose\n[page](notes.md)\n[remote](https://example.test/recording.wav)"
    );
    assert!(!safe.ambiguous_attachment);

    let ambiguous = parse_markdown(b"prose [audio](private/recording.mp3) remains").unwrap();
    assert!(ambiguous.ambiguous_attachment);
    assert_eq!(ambiguous.body, "");

    for unsafe_body in [
        b"[page](notes.md) [audio](private/recording.mp3)".as_slice(),
        b"[audio]: private/recording.m4a".as_slice(),
        b"![waveform](private/recording.wav)".as_slice(),
    ] {
        let parsed = parse_markdown(unsafe_body).unwrap();
        assert!(parsed.ambiguous_attachment);
        assert_eq!(parsed.body, "");
    }

    let titled = parse_markdown(b"[audio](private/recording.mp3 \"voice memo\")").unwrap();
    assert!(!titled.ambiguous_attachment);
    assert_eq!(titled.body, "");
}

#[test]
fn parser_collects_standalone_local_audio_links_removed_from_body() {
    let parsed = parse_markdown(b"prose\n[audio](recording.m4a)\nmore prose").unwrap();
    assert_eq!(parsed.audio_links, vec!["recording.m4a".to_owned()]);
    assert_eq!(parsed.body, "prose\nmore prose");

    let ambiguous = parse_markdown(b"prose [audio](private/recording.mp3) remains").unwrap();
    assert!(ambiguous.audio_links.is_empty());

    let traversal =
        parse_markdown(b"prose\n[audio](../outside/recording.m4a)\nmore prose").unwrap();
    assert!(traversal.audio_links.is_empty());
    assert!(traversal.ambiguous_attachment);

    let absolute = parse_markdown(b"prose\n[audio](/Users/x/recording.m4a)\nmore prose").unwrap();
    assert!(absolute.audio_links.is_empty());
    assert!(absolute.ambiguous_attachment);
}

#[test]
fn discovery_and_selected_reads_enforce_the_byte_limit_while_reading() {
    let root = temp();
    let path = root.join("Ayumi/2026/10/28/large.md");
    let file = fs::File::create(&path).unwrap();
    file.set_len(MAX_SOURCE_BYTES + 1).unwrap();
    drop(file);
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let found = catalog
        .discover_extra_day(&DiaryDay::parse("2026-10-28").unwrap())
        .unwrap();
    assert_eq!(found.candidates.len(), 1);
    assert_eq!(
        found.candidates[0].date_evidence.warning.as_deref(),
        Some("source exceeds the readable size limit")
    );
    assert_eq!(
        catalog.read_selected(&found.candidates[0].source).unwrap(),
        ReadOutcome::Unreadable
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn discovery_does_not_follow_date_directory_symlinks() {
    let root = temp();
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("private.md"), "must not be read").unwrap();
    symlink(&outside, root.join("Ayumi/2026/10/29")).unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let found = catalog
        .discover_extra_day(&DiaryDay::parse("2026-10-29").unwrap())
        .unwrap();
    assert!(found.candidates.is_empty());
    assert_eq!(
        found.warnings,
        vec!["source date directory is not a regular directory"]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn helper_backed_source_skips_cloud_body_and_reads_only_selected_snapshot() {
    let root = temp();
    fs::write(root.join("Ayumi/2026/10/28/cloud.md"), [0xff, 0xfe]).unwrap();
    fs::write(root.join("Ayumi/2026/10/28/resident.md"), "catalog body").unwrap();
    let helper = root.join("fake-helper.sh");
    fs::write(
            &helper,
            r#"#!/bin/sh
input=$(tail -c +5)
id=$(printf '%s' "$input" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
if printf '%s' "$input" | grep -q 'icloud-availability'; then
  if printf '%s' "$input" | grep -q 'cloud.md'; then status=cloud-only; else status=resident; fi
  body=$(printf '{"ok":true,"protocol_version":2,"request_id":"%s","value":{"status":"%s"}}' "$id" "$status")
elif printf '%s' "$input" | grep -q 'icloud-coordinated-read'; then
  directory=$(printf '%s' "$input" | sed -n 's/.*"handoff_directory":"\([^"]*\)".*/\1/p')
  file="$directory/nd-source-selected.bin"
  printf '%s' '---
created_at: 2026-10-28T09:00:00+09:00
---
bridge body
[audio](private.m4a)' > "$file"
  chmod 600 "$file"
  size=$(wc -c < "$file" | tr -d ' ')
  body=$(printf '{"ok":true,"protocol_version":2,"request_id":"%s","value":{"file_identity":"4:5","handoff_name":"nd-source-selected.bin","modified_at":123,"size":%s}}' "$id" "$size")
else
  body=$(printf '{"ok":true,"protocol_version":2,"request_id":"%s","value":{"status":"resident"}}' "$id")
fi
length=${#body}
printf '%b%b%b%b%s' "\\$(printf '%03o' $(((length >> 24) & 255)))" "\\$(printf '%03o' $(((length >> 16) & 255)))" "\\$(printf '%03o' $(((length >> 8) & 255)))" "\\$(printf '%03o' $((length & 255)))" "$body"
"#,
        )
        .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    let bridge = AppleBridgeClient::new(
        &helper,
        "Ayumi",
        root.join("handoff"),
        // Generous: the fake helper is a multi-process sh script, and under a
        // fully parallel test run a 2 s budget intermittently timed out.
        Duration::from_secs(10),
        2,
    )
    .unwrap();
    let source = AyumiSource::new(AyumiCatalog::new(&root, "Ayumi").unwrap(), bridge);
    let discovered = SourceCatalog::discover(
        &source,
        CatalogRequest::ExplicitDate(DiaryDay::parse("2026-10-28").unwrap()),
    )
    .unwrap();
    assert_eq!(discovered.candidates.len(), 2);
    assert_eq!(
        discovered.candidates[0].availability,
        SourceAvailability::CloudOnly
    );
    assert_eq!(
        discovered.candidates[0].date_evidence.warning.as_deref(),
        Some("source needs download")
    );
    SourceReader::request_download(&source, &discovered.candidates[0].source).unwrap();
    let selected = &discovered.candidates[1].source;
    let SourceReadOutcome::Read(read) = SourceReader::read_selected(&source, selected).unwrap()
    else {
        panic!()
    };
    assert_eq!(read.snapshot.body, "bridge body");
    assert_eq!(read.snapshot.stamp.file_identity, "4:5");
    assert_eq!(
        SourceReader::revalidate(&source, &read.snapshot).unwrap(),
        FreshnessOutcome::Fresh
    );
    let mut workflow = diary_core::Workflow::new(
        DiaryDay::parse("2026-10-28").unwrap(),
        Timestamp(1),
        &diary_core::Preferences::default(),
    );
    workflow
        .discover(0, discovered.candidates, Timestamp(2))
        .unwrap();
    workflow
        .confirm_sources(
            workflow.revision(),
            vec![read.snapshot],
            false,
            Timestamp(3),
        )
        .unwrap();
    assert_eq!(workflow.snapshots().len(), 1);
    for checkpoint in [
        diary_core::FreshnessCheckpoint::PostRead,
        diary_core::FreshnessCheckpoint::BeforeDrafting,
        diary_core::FreshnessCheckpoint::BeforeEntryConfirmation,
        diary_core::FreshnessCheckpoint::BeforePreview,
        diary_core::FreshnessCheckpoint::BeforeFirstWrite,
    ] {
        diary_core::revalidate_workflow_sources(&mut workflow, &source, checkpoint, Timestamp(4))
            .unwrap();
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn overwrite_body_replaces_text_and_returns_new_stamp() {
    let root = temp();
    let path = root.join("Ayumi/2026/10/28/n.md");
    fs::write(
        &path,
        "---\ncreated_at: 2026-10-28T09:00:00+09:00\n---\noriginal body",
    )
    .unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();
    let ReadOutcome::Read(selected) = catalog.read_selected(&source).unwrap() else {
        panic!()
    };
    let expected_digest = Digest::of_bytes(selected.snapshot.body.as_bytes());
    assert_eq!(selected.snapshot.body, "original body");

    let new_stamp = catalog
        .overwrite_body(&source, "new body", &expected_digest)
        .unwrap();

    let on_disk = fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk,
        "---\ncreated_at: 2026-10-28T09:00:00+09:00\n---\nnew body"
    );
    let ReadOutcome::Read(refreshed) = catalog.read_selected(&source).unwrap() else {
        panic!()
    };
    assert_eq!(refreshed.snapshot.body, "new body");
    assert_eq!(
        new_stamp.content_digest,
        refreshed.snapshot.stamp.content_digest
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn overwrite_body_rejects_stale_digest() {
    let root = temp();
    let path = root.join("Ayumi/2026/10/28/n.md");
    fs::write(&path, "original body").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();

    let wrong_digest = Digest::of_bytes(b"not the current body");
    let result = catalog.overwrite_body(&source, "new body", &wrong_digest);
    assert_eq!(result, Err(SourceWriteError::Stale));
    assert_eq!(fs::read_to_string(&path).unwrap(), "original body");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn overwrite_body_accepts_its_own_previously_returned_digest_on_a_second_call() {
    let root = temp();
    let note_path = root.join("Ayumi/2026/10/28/n.md");
    let audio_path = root.join("Ayumi/2026/10/28/recording.m4a");
    fs::write(&note_path, "line1\n[audio](recording.m4a)").unwrap();
    fs::write(&audio_path, b"fake audio bytes").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();
    let ReadOutcome::Read(selected) = catalog.read_selected(&source).unwrap() else {
        panic!()
    };
    let first_digest = Digest::of_bytes(selected.snapshot.body.as_bytes());

    let first_stamp = catalog
        .overwrite_body(&source, "line1 v2", &first_digest)
        .unwrap();

    // A real caller naturally holds the raw `content_digest` this
    // method just returned (the only digest value it hands back), not
    // a freshly recomputed body digest. Using it directly as
    // `expected_content_digest` for the very next call must not
    // spuriously fail, even though the on-disk body is no longer
    // byte-identical to "line1 v2" (it now also carries the
    // reattached audio line).
    let second_result = catalog.overwrite_body(&source, "line1 v3", &first_stamp.content_digest);
    assert!(
        second_result.is_ok(),
        "expected the second call to succeed using the first call's returned digest, got {second_result:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn overwrite_body_round_trips_a_bracket_sourced_destination_with_whitespace() {
    let root = temp();
    let note_path = root.join("Ayumi/2026/10/28/n.md");
    let audio_path = root.join("Ayumi/2026/10/28/my rec.m4a");
    fs::write(&note_path, "keep\n[audio](<my rec.m4a>)\nmore").unwrap();
    fs::write(&audio_path, b"fake audio bytes").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();
    let ReadOutcome::Read(selected) = catalog.read_selected(&source).unwrap() else {
        panic!()
    };
    assert_eq!(selected.snapshot.body, "keep\nmore");
    let digest = Digest::of_bytes(selected.snapshot.body.as_bytes());

    catalog
        .overwrite_body(&source, "updated keep\nmore", &digest)
        .unwrap();

    // A bare destination would be cut at the first space on re-parse,
    // so the whitespace-containing destination must be re-emitted in
    // bracket form to survive the round trip.
    assert_eq!(
        fs::read_to_string(&note_path).unwrap(),
        "updated keep\nmore\n\n[audio](<my rec.m4a>)"
    );

    let deleted = catalog.delete_source(&source).unwrap();
    assert!(!note_path.exists());
    assert!(
        !audio_path.exists(),
        "delete_source must still find the whitespace-destination audio file after the round trip"
    );
    assert_eq!(deleted.len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn overwrite_body_reattaches_audio_links_so_delete_can_still_find_them() {
    let root = temp();
    let note_path = root.join("Ayumi/2026/10/28/n.md");
    let audio_path = root.join("Ayumi/2026/10/28/recording.m4a");
    fs::write(&note_path, "keep\n[audio](recording.m4a)\nmore").unwrap();
    fs::write(&audio_path, b"fake audio bytes").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();
    let ReadOutcome::Read(selected) = catalog.read_selected(&source).unwrap() else {
        panic!()
    };
    assert_eq!(selected.snapshot.body, "keep\nmore");
    let digest = Digest::of_bytes(selected.snapshot.body.as_bytes());

    catalog
        .overwrite_body(&source, "updated keep\nmore", &digest)
        .unwrap();

    // The audio link the parser withheld from the readable body must be
    // re-appended on disk, or a later read/delete can never find it again.
    assert_eq!(
        fs::read_to_string(&note_path).unwrap(),
        "updated keep\nmore\n\n[audio](recording.m4a)"
    );

    // The blank separator line survives re-parsing as a trailing blank
    // line in the readable body (the parser only strips the
    // attachment-only line itself, consistent with how it has always
    // handled a blank line preceding an attachment); what matters is
    // that the real prose is intact and the audio link is still found.
    let ReadOutcome::Read(refreshed) = catalog.read_selected(&source).unwrap() else {
        panic!()
    };
    assert_eq!(refreshed.snapshot.body, "updated keep\nmore\n");

    let deleted = catalog.delete_source(&source).unwrap();
    assert!(!note_path.exists());
    assert!(
        !audio_path.exists(),
        "delete_source must still find and delete the re-attached audio file"
    );
    assert_eq!(deleted.len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn overwrite_body_rejects_when_ambiguous_attachment_appeared_since_last_read() {
    let root = temp();
    let path = root.join("Ayumi/2026/10/28/n.md");
    fs::write(&path, "keep text").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();
    let ReadOutcome::Read(selected) = catalog.read_selected(&source).unwrap() else {
        panic!()
    };
    assert_eq!(selected.snapshot.body, "keep text");
    let digest = Digest::of_bytes(selected.snapshot.body.as_bytes());

    // Simulate another device adding an unseen, ambiguous attachment
    // line. It is stripped from the readable body (so the body digest
    // alone still matches) but the file now carries content the parser
    // could not safely surface to the caller. (`read_selected` reports
    // this file as `Unreadable` once it is ambiguous, which is exactly
    // why `overwrite_body` cannot rely on the readable-body digest
    // alone to detect it.)
    fs::write(&path, "keep text\n[audio]: private/recording.m4a").unwrap();

    let result = catalog.overwrite_body(&source, "new body", &digest);
    assert_eq!(result, Err(SourceWriteError::Stale));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "keep text\n[audio]: private/recording.m4a"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn overwrite_body_rejects_when_front_matter_becomes_unclosed_since_last_read() {
    let root = temp();
    let path = root.join("Ayumi/2026/10/28/n.md");
    fs::write(&path, "---\ncreated_at: 2026-10-28T09:00:00+09:00\n---\n").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();
    let ReadOutcome::Read(selected) = catalog.read_selected(&source).unwrap() else {
        panic!()
    };
    assert_eq!(selected.snapshot.body, "");
    let digest = Digest::of_bytes(selected.snapshot.body.as_bytes());

    // Simulate the front matter becoming unclosed (so the parser can no
    // longer see any body at all). Its readable body is also empty, so
    // the empty-body digest alone would wrongly match the caller's
    // stale digest and silently destroy the hidden content.
    fs::write(
        &path,
        "---\ncreated_at: 2026-10-28T09:00:00+09:00\nsome more content without closing",
    )
    .unwrap();

    let result = catalog.overwrite_body(&source, "new body", &digest);
    assert_eq!(result, Err(SourceWriteError::Stale));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "---\ncreated_at: 2026-10-28T09:00:00+09:00\nsome more content without closing"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_source_removes_note_and_its_audio() {
    let root = temp();
    let note_path = root.join("Ayumi/2026/10/28/n.md");
    let audio_path = root.join("Ayumi/2026/10/28/recording.m4a");
    fs::write(&note_path, "prose\n[audio](recording.m4a)\nmore prose").unwrap();
    fs::write(&audio_path, b"fake audio bytes").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();

    let deleted = catalog.delete_source(&source).unwrap();

    assert!(!note_path.exists());
    assert!(!audio_path.exists());
    assert_eq!(deleted.len(), 2);
    assert!(deleted[0].ends_with("recording.m4a"), "audio deleted first");
    assert!(deleted[1].ends_with("n.md"), "note deleted last");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_source_rejects_traversal_via_symlinked_subdirectory() {
    // Task 2's string-level `destination_is_path_safe` check (no leading
    // `/`, no `..` component) already excludes an attempt like
    // `../../../x.m4a`, so that shape never reaches `delete_source`'s own
    // defense. This test uses a destination that passes the upstream
    // string check cleanly (`sub/recording.m4a`, no `..`, no leading `/`)
    // but where `sub` is a symlink whose target escapes the note's
    // directory — the exact case the resolved/canonicalized
    // `starts_with` check exists to catch.
    let root = temp();
    let outside_dir = root.join("outside");
    fs::create_dir_all(&outside_dir).unwrap();
    let outside_audio = outside_dir.join("recording.m4a");
    fs::write(&outside_audio, b"must not be deleted").unwrap();
    let note_dir = root.join("Ayumi/2026/10/28");
    symlink(&outside_dir, note_dir.join("sub")).unwrap();
    let note_path = note_dir.join("n.md");
    fs::write(&note_path, "prose\n[audio](sub/recording.m4a)\nmore prose").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();

    let result = catalog.delete_source(&source);

    assert_eq!(result, Err(SourceWriteError::UnsafePath));
    assert!(
        note_path.exists(),
        "the all-or-nothing design means the note itself must survive \
             when any one of its audio links is unsafe"
    );
    assert_eq!(
        fs::read(&outside_audio).unwrap(),
        b"must not be deleted",
        "the file the symlink escapes to must be untouched"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_source_rejects_audio_link_with_query_or_fragment_character() {
    // `is_local_audio_destination` strips a trailing `?...`/`#...` only
    // to decide whether a destination *looks like* audio; the literal
    // destination string (including the `?`/`#` suffix) is what
    // `delete_source` would otherwise resolve and delete. A literal
    // sibling file with that exact crafted name must never be reachable
    // through this path.
    let root = temp();
    let note_path = root.join("Ayumi/2026/10/28/n.md");
    let sibling_path = root.join("Ayumi/2026/10/28/other.m4a?.md");
    fs::write(&sibling_path, b"unrelated sibling note").unwrap();
    fs::write(&note_path, "prose\n[audio](other.m4a?.md)\nmore").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();

    let result = catalog.delete_source(&source);

    assert_eq!(result, Err(SourceWriteError::UnsafePath));
    assert!(
        note_path.exists(),
        "nothing should be deleted when any audio link is unsafe"
    );
    assert_eq!(fs::read(&sibling_path).unwrap(), b"unrelated sibling note");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_source_dedupes_duplicate_audio_links() {
    let root = temp();
    let note_path = root.join("Ayumi/2026/10/28/n.md");
    let audio_path = root.join("Ayumi/2026/10/28/recording.m4a");
    fs::write(
        &note_path,
        "prose\n[audio](recording.m4a)\nmore\n[audio](./recording.m4a)\nend",
    )
    .unwrap();
    fs::write(&audio_path, b"fake audio bytes").unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();

    let deleted = catalog.delete_source(&source).unwrap();

    assert!(!note_path.exists());
    assert!(!audio_path.exists());
    assert_eq!(
        deleted.len(),
        2,
        "the duplicate alias must collapse to a single delete, not fail the second remove"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_source_with_ambiguous_attachment_line_deletes_note_but_no_audio() {
    let root = temp();
    let note_path = root.join("Ayumi/2026/10/28/n.md");
    let safe_audio = root.join("Ayumi/2026/10/28/safe.m4a");
    let ambiguous_target = root.join("Ayumi/2026/10/28/private");
    fs::create_dir_all(&ambiguous_target).unwrap();
    let ambiguous_audio = ambiguous_target.join("ambiguous.mp3");
    fs::write(&safe_audio, b"safe audio bytes").unwrap();
    fs::write(&ambiguous_audio, b"ambiguous audio bytes").unwrap();
    fs::write(
        &note_path,
        "prose\n[audio](safe.m4a)\nprose [audio](private/ambiguous.mp3) remains",
    )
    .unwrap();
    let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();

    let deleted = catalog.delete_source(&source).unwrap();

    assert!(!note_path.exists());
    assert!(!safe_audio.exists());
    assert!(
        ambiguous_audio.exists(),
        "ambiguous attachment target must be left untouched"
    );
    assert_eq!(deleted.len(), 2);
    assert!(deleted.iter().any(|p| p.ends_with("safe.m4a")));
    assert!(deleted.iter().any(|p| p.ends_with("n.md")));
    let _ = fs::remove_dir_all(root);
}
