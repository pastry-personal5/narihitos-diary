use super::*;
use std::os::unix::fs::OpenOptionsExt;

#[test]
fn strict_response_parser_rejects_extra_frames_and_mismatched_ids() {
    let valid = encode_frame(
        br#"{"ok":true,"protocol_version":2,"request_id":"r","value":{"status":"resident"}}"#,
    )
    .unwrap();
    assert_eq!(
        object_string(&parse_response(&valid, "r").unwrap(), "status"),
        Ok("resident")
    );
    let mut extra = valid.clone();
    extra.extend_from_slice(&encode_frame(b"{}").unwrap());
    assert_eq!(
        parse_response(&extra, "r"),
        Err(SourceAccessError::HelperProtocol)
    );
    assert_eq!(
        parse_response(&valid, "other"),
        Err(SourceAccessError::HelperProtocol)
    );
    assert_eq!(
            parse_response(&encode_frame(br#"{"ok":true,"protocol_version":2,"request_id":"r","value":{"status":"resident",}}"#).unwrap(), "r"),
            Err(SourceAccessError::HelperProtocol)
        );
    assert_eq!(
            parse_response(&encode_frame(br#"{"error":{"code":"permission_denied","message":"request rejected"},"ok":false,"protocol_version":2,"request_id":"r"}"#).unwrap(), "r"),
            Err(SourceAccessError::PermissionDenied)
        );
    assert_eq!(
            parse_response(&encode_frame(br#"{"error":{"code":"oversize","message":"request rejected"},"ok":false,"protocol_version":2,"request_id":"r"}"#).unwrap(), "r"),
            Err(SourceAccessError::Oversized)
        );
}

#[test]
fn handoff_directory_rejects_unsafe_mode_and_cleans_owned_orphans() {
    let root = std::env::temp_dir().join(format!(
        "nd-bridge-client-{}-{}",
        std::process::id(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let orphan = root.join("nd-source-orphan.bin");
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&orphan)
        .unwrap();
    let client =
        AppleBridgeClient::new("missing", "Ayumi", &root, Duration::from_secs(1), 1).unwrap();
    assert!(!orphan.exists());
    drop(client);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn concurrency_permit_is_bounded_and_released() {
    let counter = Arc::new(Mutex::new(0));
    let permit = Permit::acquire(&counter, 1).unwrap();
    assert!(matches!(
        Permit::acquire(&counter, 1),
        Err(SourceAccessError::Busy)
    ));
    drop(permit);
    assert!(Permit::acquire(&counter, 1).is_ok());
}

#[test]
fn helper_lookup_prefers_installed_layout_then_development_checkout() {
    if std::env::var_os("ND_APPLE_BRIDGE_PATH").is_some() {
        return;
    }
    let scratch = scratch("lookup");
    let nd = scratch.join("bin/nd");
    let installed = scratch.join("libexec/apple-bridge");
    let development = scratch.join("workspace/helpers/apple-bridge/.build/debug/apple-bridge");
    fs::create_dir_all(nd.parent().unwrap()).unwrap();
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::create_dir_all(development.parent().unwrap()).unwrap();
    fs::write(&installed, "").unwrap();
    fs::write(&development, "").unwrap();
    assert_eq!(
        AppleBridgeClient::locate_executable(&nd, &scratch.join("workspace")).unwrap(),
        installed
    );
    fs::remove_file(&installed).unwrap();
    assert_eq!(
        AppleBridgeClient::locate_executable(&nd, &scratch.join("workspace")).unwrap(),
        development
    );
    let _ = fs::remove_dir_all(scratch);
}

#[test]
fn fake_helper_covers_success_protocol_crash_timeout_cancellation_and_missing() {
    let scratch = scratch("outcomes");
    let handoff = scratch.join("handoff");
    let success = script(
        &scratch,
        "success.sh",
        r#"input=$(tail -c +5)
id=$(printf '%s' "$input" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
body=$(printf '{"ok":true,"protocol_version":2,"request_id":"%s","value":{"status":"resident"}}' "$id")
length=${#body}
printf '%b%b%b%b%s' "\\$(printf '%03o' $(((length >> 24) & 255)))" "\\$(printf '%03o' $(((length >> 16) & 255)))" "\\$(printf '%03o' $(((length >> 8) & 255)))" "\\$(printf '%03o' $((length & 255)))" "$body"
"#,
    );
    let client =
        AppleBridgeClient::new(&success, "Ayumi", &handoff, Duration::from_secs(5), 1).unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/29/n.md").unwrap();
    assert_eq!(
        client.availability(&source, &AtomicBool::new(false)),
        Ok(BridgeAvailability::Resident)
    );

    let malformed = script(
        &scratch,
        "malformed.sh",
        "cat >/dev/null\nprintf '\\000\\000\\000\\002{}\\000\\000\\000\\002{}'\n",
    );
    let malformed_client =
        AppleBridgeClient::new(malformed, "Ayumi", &handoff, Duration::from_secs(5), 1).unwrap();
    assert_eq!(
        malformed_client.availability(&source, &AtomicBool::new(false)),
        Err(SourceAccessError::HelperProtocol)
    );

    let crash = script(&scratch, "crash.sh", "cat >/dev/null\nexit 7\n");
    let crash_client =
        AppleBridgeClient::new(crash, "Ayumi", &handoff, Duration::from_secs(5), 1).unwrap();
    assert_eq!(
        crash_client.availability(&source, &AtomicBool::new(false)),
        Err(SourceAccessError::HelperCrashed)
    );

    let slow = script(&scratch, "slow.sh", "cat >/dev/null\nsleep 2\n");
    let slow_client =
        AppleBridgeClient::new(slow, "Ayumi", &handoff, Duration::from_millis(20), 1).unwrap();
    assert_eq!(
        slow_client.availability(&source, &AtomicBool::new(false)),
        Err(SourceAccessError::TimedOut)
    );
    assert_eq!(
        slow_client.availability(&source, &AtomicBool::new(true)),
        Err(SourceAccessError::Cancelled)
    );

    let missing_client = AppleBridgeClient::new(
        scratch.join("missing-helper"),
        "Ayumi",
        &handoff,
        Duration::from_secs(1),
        1,
    )
    .unwrap();
    assert_eq!(
        missing_client.availability(&source, &AtomicBool::new(false)),
        Err(SourceAccessError::HelperMissing)
    );
    let _ = fs::remove_dir_all(scratch);
}

#[test]
fn fake_helper_handoff_accepts_large_private_file_and_always_cleans_it() {
    let scratch = scratch("handoff");
    let handoff = scratch.join("private");
    let reader = script(
        &scratch,
        "reader.sh",
        r#"input=$(tail -c +5)
id=$(printf '%s' "$input" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
directory=$(printf '%s' "$input" | sed -n 's/.*"handoff_directory":"\([^"]*\)".*/\1/p')
dd if=/dev/zero of="$directory/nd-source-fake.bin" bs=70000 count=1 2>/dev/null
chmod 600 "$directory/nd-source-fake.bin"
body=$(printf '{"ok":true,"protocol_version":2,"request_id":"%s","value":{"file_identity":"9:7","handoff_name":"nd-source-fake.bin","modified_at":123,"size":70000}}' "$id")
length=${#body}
printf '%b%b%b%b%s' "\\$(printf '%03o' $(((length >> 24) & 255)))" "\\$(printf '%03o' $(((length >> 16) & 255)))" "\\$(printf '%03o' $(((length >> 8) & 255)))" "\\$(printf '%03o' $((length & 255)))" "$body"
"#,
    );
    let client =
        AppleBridgeClient::new(reader, "Ayumi", &handoff, Duration::from_secs(10), 1).unwrap();
    let source = SourceRef::new(CONNECTOR, "2026/10/29/n.md").unwrap();
    let read = client
        .coordinated_read(&source, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(read.bytes.len(), 70_000);
    assert_eq!(read.file_identity, "9:7");
    assert!(!handoff.join("nd-source-fake.bin").exists());

    let failing = script(
        &scratch,
        "failing-reader.sh",
        r#"input=$(cat)
directory=$(printf '%s' "$input" | sed -n 's/.*"handoff_directory":"\([^"]*\)".*/\1/p')
printf partial > "$directory/nd-source-partial.bin"
chmod 600 "$directory/nd-source-partial.bin"
printf '{}\n{}\n'
"#,
    );
    let failing_client =
        AppleBridgeClient::new(failing, "Ayumi", &handoff, Duration::from_secs(10), 1).unwrap();
    assert_eq!(
        failing_client.coordinated_read(&source, &AtomicBool::new(false)),
        Err(SourceAccessError::HelperProtocol)
    );
    assert!(!handoff.join("nd-source-partial.bin").exists());
    let _ = fs::remove_dir_all(scratch);
}

#[test]
fn concurrent_reads_use_isolated_handoff_directories() {
    let scratch = scratch("concurrent-handoff");
    let handoff = scratch.join("private");
    let marker = scratch.join("slow-ready");
    let reader = script(
        &scratch,
        "concurrent-reader.sh",
        &format!(
            r#"input=$(tail -c +5)
id=$(printf '%s' "$input" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
directory=$(printf '%s' "$input" | sed -n 's/.*"handoff_directory":"\([^"]*\)".*/\1/p')
file="$directory/nd-source-fake.bin"
printf data > "$file"
chmod 600 "$file"
if printf '%s' "$input" | grep -q 'slow.md'; then
  touch '{}'
  sleep 0.2
fi
body=$(printf '{{"ok":true,"protocol_version":2,"request_id":"%s","value":{{"file_identity":"9:7","handoff_name":"nd-source-fake.bin","modified_at":123,"size":4}}}}' "$id")
length=${{#body}}
printf '%b%b%b%b%s' "\\$(printf '%03o' $(((length >> 24) & 255)))" "\\$(printf '%03o' $(((length >> 16) & 255)))" "\\$(printf '%03o' $(((length >> 8) & 255)))" "\\$(printf '%03o' $((length & 255)))" "$body"
"#,
            marker.display()
        ),
    );
    let client =
        AppleBridgeClient::new(reader, "Ayumi", &handoff, Duration::from_secs(10), 2).unwrap();
    let slow_client = client.clone();
    let slow = std::thread::spawn(move || {
        slow_client.coordinated_read(
            &SourceRef::new(CONNECTOR, "2026/10/29/slow.md").unwrap(),
            &AtomicBool::new(false),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(marker.exists());
    let fast = client
        .coordinated_read(
            &SourceRef::new(CONNECTOR, "2026/10/29/fast.md").unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(fast.bytes, b"data");
    assert_eq!(slow.join().unwrap().unwrap().bytes, b"data");
    assert!(fs::read_dir(&handoff).unwrap().next().is_none());
    let _ = fs::remove_dir_all(scratch);
}

#[test]
fn download_wait_is_capped_to_the_protocol_maximum() {
    let scratch = scratch("download-timeout");
    let helper = script(
        &scratch,
        "download.sh",
        r#"input=$(tail -c +5)
id=$(printf '%s' "$input" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
if ! printf '%s' "$input" | grep -q '"timeout_milliseconds":60000'; then exit 9; fi
body=$(printf '{"ok":true,"protocol_version":2,"request_id":"%s","value":{"status":"resident"}}' "$id")
length=${#body}
printf '%b%b%b%b%s' "\\$(printf '%03o' $(((length >> 24) & 255)))" "\\$(printf '%03o' $(((length >> 16) & 255)))" "\\$(printf '%03o' $(((length >> 8) & 255)))" "\\$(printf '%03o' $((length & 255)))" "$body"
"#,
    );
    let client = AppleBridgeClient::new(
        helper,
        "Ayumi",
        scratch.join("handoff"),
        Duration::from_secs(120),
        1,
    )
    .unwrap();
    client
        .request_download(
            &SourceRef::new(CONNECTOR, "2026/10/29/n.md").unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap();
    let _ = fs::remove_dir_all(scratch);
}

#[test]
fn fake_helper_qualifies_all_apple_assistant_actions_and_cancellation() {
    let scratch = scratch("assistant-actions");
    let helper = script(
        &scratch,
        "assistant.sh",
        r#"input=$(tail -c +5)
id=$(printf '%s' "$input" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
case "$input" in
  *assistant-availability*) value='{"status":"available"}' ;;
  *assistant-extract-memories*) value='{"candidates":[{"text":"walk","source_ordinals":[1]}]}' ;;
  *assistant-draft-entry*) value='{"text":"draft","reflection_ranges":[]}' ;;
  *assistant-review-paragraph*) value='{"replacement":"better","reflection_ranges":[],"explanation":"clearer"}' ;;
  *assistant-refine-source*) value='{"body":"refined source"}' ;;
  *) exit 3 ;;
esac
body=$(printf '{"ok":true,"protocol_version":2,"request_id":"%s","value":%s}' "$id" "$value")
length=${#body}
printf '%b%b%b%b%s' "\\$(printf '%03o' $(((length >> 24) & 255)))" "\\$(printf '%03o' $(((length >> 16) & 255)))" "\\$(printf '%03o' $(((length >> 8) & 255)))" "\\$(printf '%03o' $((length & 255)))" "$body"
"#,
    );
    let bridge = AppleBridgeClient::new(
        helper,
        "Ayumi",
        scratch.join("handoff"),
        Duration::from_secs(10),
        1,
    )
    .unwrap();
    let adapter = AppleFoundationAssistant::new(bridge, true);
    assert!(adapter.descriptor().available);

    let memories = adapter
        .invoke(
            &AssistantRequest::ExtractMemories(
                diary_core::MemoryExtractionRequest::new(vec!["source".to_owned()]).unwrap(),
            ),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(matches!(memories, AssistantResponse::Memories(_)));

    let draft = adapter
        .invoke(
            &AssistantRequest::DraftEntry(
                diary_core::DraftEntryRequest::new(
                    "memory",
                    None,
                    WritingLanguage::KoKr,
                    diary_core::WritingStyle::Reflective,
                )
                .unwrap(),
            ),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(matches!(draft, AssistantResponse::EntryDraft(_)));

    let review = adapter
        .invoke(
            &AssistantRequest::ReviewParagraph(
                diary_core::ParagraphReviewRequest::new(
                    "paragraph",
                    WritingLanguage::EnUs,
                    diary_core::WritingStyle::Standard,
                )
                .unwrap(),
            ),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(matches!(review, AssistantResponse::ParagraphReview(_)));
    let refinement = adapter
        .invoke(
            &AssistantRequest::RefineSource(
                diary_core::SourceRefinementRequest::new("source").unwrap(),
            ),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(matches!(refinement, AssistantResponse::SourceRefinement(_)));
    assert_eq!(
        adapter.invoke(
            &AssistantRequest::ExtractMemories(
                diary_core::MemoryExtractionRequest::new(vec!["source".to_owned()]).unwrap(),
            ),
            &AtomicBool::new(true),
        ),
        Err(DraftAssistantError::Cancelled)
    );
    let _ = fs::remove_dir_all(scratch);
}

fn scratch(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "nd-bridge-{label}-{}-{}",
        std::process::id(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn script(directory: &Path, name: &str, body: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}
