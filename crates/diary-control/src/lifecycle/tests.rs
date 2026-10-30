use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn entries_root_expands_the_documented_home_prefix() {
    let launch_root = Path::new("/workspace/narihitos-diary");
    let home = Path::new("/Users/diary-owner");
    assert_eq!(
        resolve_entries_root(
            launch_root,
            home,
            PathBuf::from("~/Documents/narihitos-diary/entries"),
        ),
        home.join("Documents/narihitos-diary/entries")
    );
    assert_eq!(
        resolve_entries_root(launch_root, home, PathBuf::from("entries")),
        launch_root.join("entries")
    );
}

#[test]
fn parses_optional_icloud_entries_root() {
    let parsed = parse_config(
            "sources:\n  ayumi_directory_name: Ayumi\npaths:\n  entries_root: \"entries\"\n  icloud_entries_root: \"~/Library/Mobile Documents/com~apple~CloudDocs/d\"\n",
        )
        .unwrap();
    let home = Path::new("/Users/diary-owner");
    let resolved = resolve_entries_root(
        Path::new("/workspace"),
        home,
        parsed.icloud_entries_root.unwrap(),
    );
    assert_eq!(
        resolved,
        home.join("Library/Mobile Documents/com~apple~CloudDocs/d")
    );
}

#[test]
fn absent_icloud_root_is_none() {
    let parsed =
        parse_config("sources:\n  ayumi_directory_name: Ayumi\npaths:\n  entries_root: x\n")
            .unwrap();
    assert!(parsed.icloud_entries_root.is_none());
}

#[test]
fn duplicate_icloud_root_is_invalid() {
    assert!(
            parse_config(
                "sources:\n  ayumi_directory_name: Ayumi\npaths:\n  icloud_entries_root: a\n  icloud_entries_root: b\n"
            )
            .is_err()
        );
}

#[test]
fn tag_key_is_created_private_and_stable_across_loads() {
    let scratch = std::env::temp_dir().join(format!(
        "nd-tag-key-{}-{}",
        std::process::id(),
        SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&scratch).unwrap();
    fs::set_permissions(&scratch, fs::Permissions::from_mode(0o700)).unwrap();
    let uid = fs::metadata(&scratch).unwrap().uid();
    let path = scratch.join("diagnostic-tag-key");
    let first = load_or_create_tag_key(&path, uid).unwrap();
    assert_ne!(first, [0; 32]);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(load_or_create_tag_key(&path, uid).unwrap(), first);
    fs::write(&path, "not-a-key").unwrap();
    assert!(load_or_create_tag_key(&path, uid).is_err());
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn strict_config_bounds_server_fallback_and_detects_notion_placeholders() {
    let parsed = parse_config(
            "sources:\n  ayumi_directory_name: \"Ayumi\"\npaths:\n  entries_root: \"ignored\"\nnotion:\n  token: \"REPLACE_TOKEN\"\n  root_page_id: \"REPLACE_ROOT\"\nserver:\n  port_fallback_count: 16\n",
        )
        .unwrap();
    assert_eq!(parsed.port_fallback_count, 16);
    assert_eq!(parsed.ayumi_directory_name, "Ayumi");
    assert!(!parsed.notion_configured);
    let configured = parse_config(
            "sources:\n  ayumi_directory_name: Ayumi\nnotion:\n  token: secret\n  root_page_id: 01234567-89AB-CDEF-0123-456789ABCDEF\n",
        )
        .unwrap();
    let notion = configured.notion.unwrap();
    assert_eq!(notion.root_page_id(), "0123456789abcdef0123456789abcdef");
    assert!(!format!("{notion:?}").contains("secret"));
    assert!(
            parse_config(
                "sources:\n  ayumi_directory_name: Ayumi\nnotion:\n  token: secret\n  root_page_id: not-a-page-id\n"
            )
            .is_err()
        );
    assert!(
        parse_config(
            "sources:\n  ayumi_directory_name: Ayumi\nserver:\n  port_fallback_count: 17\n"
        )
        .is_err()
    );
    assert!(
        parse_config("sources:\n  ayumi_directory_name: Ayumi\nunknown:\n  value: 1\n").is_err()
    );
}

#[test]
fn strict_config_parses_zero_one_and_multiple_codex_profiles() {
    let zero = parse_config("sources:\n  ayumi_directory_name: Ayumi\n").unwrap();
    assert!(zero.codex_profiles.is_empty());

    let one = parse_config(
            "sources:\n  ayumi_directory_name: Ayumi\nassistants:\n  codex_profiles:\n    - alias: pp7\n      codex_home: \"/tmp/codex-pp7\"\n",
        )
        .unwrap();
    assert_eq!(one.codex_profiles.len(), 1);
    assert_eq!(one.codex_profiles[0].alias, "pp7");
    assert_eq!(
        one.codex_profiles[0].codex_home,
        Path::new("/tmp/codex-pp7")
    );
    assert_eq!(one.codex_profiles[0].codex_executable, None);

    let multiple = parse_config(
            "sources:\n  ayumi_directory_name: Ayumi\nassistants:\n  codex_profiles:\n    - alias: first\n      codex_home: /tmp/first\n      codex_executable: /opt/codex/bin/codex\n    - alias: second_2\n      codex_home: /tmp/second\n",
        )
        .unwrap();
    assert_eq!(multiple.codex_profiles.len(), 2);
    assert_eq!(
        multiple.codex_profiles[0].codex_executable.as_deref(),
        Some(Path::new("/opt/codex/bin/codex"))
    );
}

#[test]
fn strict_config_rejects_malformed_codex_profile_collections() {
    let invalid_blocks = [
        "assistants:\n  other: []\n",
        "assistants:\n  codex_profiles:\n    - alias: Upper\n      codex_home: /tmp/home\n",
        "assistants:\n  codex_profiles:\n    - alias: same\n      codex_home: /tmp/one\n    - alias: same\n      codex_home: /tmp/two\n",
        "assistants:\n  codex_profiles:\n    - alias: missing_home\n",
        "assistants:\n  codex_profiles:\n    - alias: relative\n      codex_home: relative/home\n",
        "assistants:\n  codex_profiles:\n    - alias: traversal\n      codex_home: /tmp/../private\n",
        "assistants:\n  codex_profiles:\n    - alias: relative_exec\n      codex_home: /tmp/home\n      codex_executable: bin/codex\n",
        "assistants:\n  codex_profiles:\n    - alias: duplicate_key\n      alias: twice\n      codex_home: /tmp/home\n",
        "assistants:\n  codex_profiles:\n    - alias: unknown\n      codex_home: /tmp/home\n      command: codex\n",
        "assistants:\n  codex_profiles:\n    - codex_home: /tmp/home\n      alias: wrong_first_is_allowed_but_too_long_here\n",
    ];
    for block in invalid_blocks {
        let raw = format!("sources:\n  ayumi_directory_name: Ayumi\n{block}");
        assert_eq!(
            parse_config(&raw).err(),
            Some(LifecycleError::InvalidConfig)
        );
    }
}

#[test]
fn strict_config_bounds_codex_profile_count() {
    let mut raw =
        "sources:\n  ayumi_directory_name: Ayumi\nassistants:\n  codex_profiles:\n".to_owned();
    for index in 0..=MAX_COLLECTION_ITEMS {
        use std::fmt::Write as _;
        write!(
            &mut raw,
            "    - alias: p{index}\n      codex_home: /tmp/profile-{index}\n"
        )
        .unwrap();
    }
    assert_eq!(
        parse_config(&raw).err(),
        Some(LifecycleError::InvalidConfig)
    );
}

#[test]
fn lock_runtime_publication_and_cleanup_are_instance_scoped() {
    let scratch = Scratch::new("runtime");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let runtime = scratch.path().join("runtime");
    ensure_private_directory(&runtime, uid).unwrap();
    let first = ServerLock::acquire(&runtime, uid).unwrap();
    assert_eq!(
        ServerLock::acquire(&runtime, uid).unwrap_err(),
        LifecycleError::AlreadyRunning
    );
    let advertisement = RuntimeAdvertisement {
        version: 1,
        instance_id: format!("instance_{}", "11".repeat(32)),
        host: "127.0.0.1".to_owned(),
        port: 8771,
        control_token: format!("token_{}", "22".repeat(32)),
        contract_version: 1,
    };
    let runtime_file = advertisement.publish(&runtime, uid).unwrap();
    let metadata = fs::metadata(runtime.join("control-api.json")).unwrap();
    assert_eq!(metadata.mode() & 0o777, 0o600);
    runtime_file.remove_if_owned().unwrap();
    assert!(!runtime.join("control-api.json").exists());
    drop(first);
    ServerLock::acquire(&runtime, uid).unwrap();
}

#[test]
fn runtime_attachment_loader_is_read_only_strict_and_token_safe() {
    let scratch = Scratch::new("runtime-reader");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let runtime = scratch.path().join("runtime");
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::MissingRuntime)
    );
    assert!(!runtime.exists());
    ensure_private_directory(&runtime, uid).unwrap();
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::MissingRuntime)
    );

    let advertisement = RuntimeAdvertisement {
        version: 1,
        instance_id: format!("instance_{}", "11".repeat(32)),
        host: "127.0.0.1".to_owned(),
        port: 8771,
        control_token: format!("token_{}", "22".repeat(32)),
        contract_version: CONTRACT_VERSION,
    };
    let runtime_file = advertisement.publish(&runtime, uid).unwrap();
    let attachment = load_runtime_attachment(&runtime, uid).unwrap();
    assert_eq!(attachment.host(), "127.0.0.1");
    assert_eq!(attachment.port(), 8771);
    assert_eq!(
        attachment.instance_id(),
        format!("instance_{}", "11".repeat(32))
    );
    assert_eq!(
        attachment.control_token(),
        format!("token_{}", "22".repeat(32))
    );
    assert_eq!(attachment.contract_version(), CONTRACT_VERSION);
    assert_eq!(
        load_runtime_attachment(&runtime, uid.saturating_add(1)).err(),
        Some(LifecycleError::UnsafePath)
    );
    runtime_file.remove_if_owned().unwrap();

    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o750)).unwrap();
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::UnsafePath)
    );
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();

    let path = runtime.join("control-api.json");
    fs::write(&path, b"not-json\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::InvalidRuntime)
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::UnsafePath)
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let linked = runtime.join("runtime-copy.json");
    fs::hard_link(&path, &linked).unwrap();
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::UnsafePath)
    );
    fs::remove_file(linked).unwrap();

    fs::remove_file(&path).unwrap();
    let symlink_target = scratch.path().join("advertisement-target.json");
    fs::write(&symlink_target, b"not-json\n").unwrap();
    fs::set_permissions(&symlink_target, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&symlink_target, &path).unwrap();
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::UnsafePath)
    );
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::UnsafePath)
    );
    fs::remove_dir(&path).unwrap();
    fs::write(&path, b"not-json\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    let oversized = OpenOptions::new().write(true).open(&path).unwrap();
    oversized.set_len(64 * 1024 + 1).unwrap();
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::UnsafePath)
    );
}

#[test]
fn runtime_attachment_loader_rejects_unsafe_directories_and_contract_mismatch() {
    let scratch = Scratch::new("runtime-reader-directory");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let runtime = scratch.path().join("runtime");

    fs::write(&runtime, b"not a directory").unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::UnsafePath)
    );
    fs::remove_file(&runtime).unwrap();

    let target = scratch.path().join("runtime-target");
    ensure_private_directory(&target, uid).unwrap();
    symlink(&target, &runtime).unwrap();
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::UnsafePath)
    );
    fs::remove_file(&runtime).unwrap();
    ensure_private_directory(&runtime, uid).unwrap();

    let advertisement = RuntimeAdvertisement {
        version: 1,
        instance_id: format!("instance_{}", "11".repeat(32)),
        host: "127.0.0.1".to_owned(),
        port: 8771,
        control_token: format!("token_{}", "22".repeat(32)),
        contract_version: CONTRACT_VERSION.saturating_add(1),
    };
    let runtime_file = advertisement.publish(&runtime, uid).unwrap();
    assert_eq!(
        load_runtime_attachment(&runtime, uid).err(),
        Some(LifecycleError::IncompatibleContract)
    );
    runtime_file.remove_if_owned().unwrap();
}

#[test]
fn runtime_attachment_loader_rejects_malformed_advertisement_fields() {
    let scratch = Scratch::new("runtime-reader-fields");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let runtime = scratch.path().join("runtime");
    ensure_private_directory(&runtime, uid).unwrap();
    let path = runtime.join("control-api.json");
    let valid = serde_json::json!({
        "version": 1,
        "instance_id": format!("instance_{}", "11".repeat(32)),
        "host": "127.0.0.1",
        "port": 8771,
        "control_token": format!("token_{}", "22".repeat(32)),
        "contract_version": CONTRACT_VERSION,
    });
    let mut invalid = Vec::new();
    for (field, value) in [
        ("version", serde_json::json!(2)),
        ("host", serde_json::json!("localhost")),
        ("port", serde_json::json!(0)),
        ("contract_version", serde_json::json!(0)),
        ("instance_id", serde_json::json!("instance_short")),
        ("control_token", serde_json::json!("token_NOT_HEX")),
    ] {
        let mut advertisement = valid.clone();
        advertisement[field] = value;
        invalid.push(advertisement);
    }
    let mut unknown_field = valid;
    unknown_field["unexpected"] = serde_json::json!(true);
    invalid.push(unknown_field);
    for advertisement in invalid {
        fs::write(&path, serde_json::to_vec(&advertisement).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            load_runtime_attachment(&runtime, uid).err(),
            Some(LifecycleError::InvalidRuntime)
        );
    }
}

#[test]
fn private_file_checks_reject_links_and_group_access() {
    let scratch = Scratch::new("private");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let real = scratch.path().join("real.yaml");
    fs::write(&real, "sources:\n").unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(
        read_private_file(&real, uid, 1024).unwrap_err(),
        LifecycleError::UnsafeConfig
    );
    fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
    let linked = scratch.path().join("linked.yaml");
    symlink(&real, &linked).unwrap();
    assert_eq!(
        read_private_file(&linked, uid, 1024).unwrap_err(),
        LifecycleError::UnsafeConfig
    );
}

#[test]
fn repository_log_path_rejects_redirecting_parent_symlinks() {
    let scratch = Scratch::new("log-parent");
    let outside = Scratch::new("log-outside");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    symlink(outside.path(), scratch.path().join(".local")).unwrap();
    let config = LifecycleConfig {
        launch_root: scratch.path().to_path_buf(),
        config_path: scratch.path().join("config.yaml"),
        application_support: scratch.path().join("state"),
        runtime_dir: scratch.path().join("state/runtime"),
        log_dir: scratch.path().join(".local/logs/control-api"),
        database_path: scratch.path().join("state/state.sqlite3"),
        port_fallback_count: 0,
        ayumi_directory_name: "Synthetic".to_owned(),
        entries_root: scratch.path().join("entries"),
        icloud_entries_root: None,
        notion_configured: false,
        codex_profiles: Vec::new(),
        expected_uid: uid,
    };
    assert_eq!(
        config.prepare_log_directory(),
        Err(LifecycleError::UnsafePath)
    );
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "nd-lifecycle-{label}-{}-{}",
            std::process::id(),
            SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
