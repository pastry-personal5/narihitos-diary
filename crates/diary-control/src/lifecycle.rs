use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use diary_control_contract::CONTRACT_VERSION;
use diary_control_contract::MAX_COLLECTION_ITEMS;
use fs2::FileExt;
use serde::{Deserialize, Serialize};

/// Secret-bearing Notion settings. Debug output is intentionally redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct NotionRuntimeConfig {
    token: String,
    root_page_id: String,
}

impl NotionRuntimeConfig {
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    #[must_use]
    pub fn root_page_id(&self) -> &str {
        &self.root_page_id
    }
}

impl std::fmt::Debug for NotionRuntimeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NotionRuntimeConfig")
            .field("token", &"[REDACTED]")
            .field("root_page_id", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct LifecycleConfig {
    pub launch_root: PathBuf,
    pub config_path: PathBuf,
    pub application_support: PathBuf,
    pub runtime_dir: PathBuf,
    pub log_dir: PathBuf,
    pub database_path: PathBuf,
    pub port_fallback_count: u16,
    pub ayumi_directory_name: String,
    pub entries_root: PathBuf,
    pub icloud_entries_root: Option<PathBuf>,
    pub notion_configured: bool,
    pub codex_profiles: Vec<CodexProfileConfig>,
    pub expected_uid: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexProfileConfig {
    pub alias: String,
    pub codex_home: PathBuf,
    pub codex_executable: Option<PathBuf>,
}

impl LifecycleConfig {
    pub fn load() -> Result<Self, LifecycleError> {
        let launch_root = std::env::current_dir().map_err(|_| LifecycleError::Io)?;
        let expected_uid = fs::metadata(&launch_root)
            .map_err(|_| LifecycleError::Io)?
            .uid();
        let config_path = std::env::var_os("ND_CONFIG_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| launch_root.join("config/global_config.yaml"));
        let raw = read_private_file(&config_path, expected_uid, 256 * 1024)?;
        let parsed = parse_config(&raw)?;
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(LifecycleError::MissingHome)?;
        let application_support = std::env::var_os("ND_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("Library/Application Support/narihitos-diary"));
        let runtime_dir = application_support.join("runtime");
        let entries_root = resolve_entries_root(&launch_root, &home, parsed.entries_root);
        let icloud_entries_root = parsed
            .icloud_entries_root
            .map(|configured| resolve_entries_root(&launch_root, &home, configured));
        Ok(Self {
            launch_root: launch_root.clone(),
            config_path,
            log_dir: launch_root.join(".local/logs/control-api"),
            database_path: application_support.join("state.sqlite3"),
            application_support,
            runtime_dir,
            port_fallback_count: parsed.port_fallback_count,
            ayumi_directory_name: parsed.ayumi_directory_name,
            entries_root,
            icloud_entries_root,
            notion_configured: parsed.notion_configured,
            codex_profiles: parsed.codex_profiles,
            expected_uid,
        })
    }

    pub fn prepare_private_state(&self) -> Result<(), LifecycleError> {
        ensure_private_directory(&self.application_support, self.expected_uid)?;
        ensure_private_directory(&self.runtime_dir, self.expected_uid)?;
        Ok(())
    }

    pub fn prepare_database(&self) -> Result<(), LifecycleError> {
        ensure_private_regular(&self.database_path, self.expected_uid)?;
        Ok(())
    }

    pub fn prepare_log_directory(&self) -> Result<(), LifecycleError> {
        let mut current = self.launch_root.clone();
        for component in [".local", "logs", "control-api"] {
            current.push(component);
            if current.exists() {
                let metadata = fs::symlink_metadata(&current).map_err(|_| LifecycleError::Io)?;
                if !metadata.file_type().is_dir()
                    || metadata.file_type().is_symlink()
                    || metadata.uid() != self.expected_uid
                {
                    return Err(LifecycleError::UnsafePath);
                }
            } else {
                fs::create_dir(&current).map_err(|_| LifecycleError::Io)?;
                fs::set_permissions(&current, fs::Permissions::from_mode(0o700))
                    .map_err(|_| LifecycleError::Io)?;
            }
        }
        ensure_private_directory(&self.log_dir, self.expected_uid)
    }

    pub fn validate_database_files(&self) -> Result<(), LifecycleError> {
        for path in [
            self.database_path.clone(),
            PathBuf::from(format!("{}-wal", self.database_path.display())),
            PathBuf::from(format!("{}-shm", self.database_path.display())),
            PathBuf::from(format!("{}-journal", self.database_path.display())),
        ] {
            if path.exists() {
                validate_private_regular(&path, self.expected_uid)?;
            }
        }
        Ok(())
    }

    /// Re-reads the owner-only file only when the composition root needs the
    /// secret-bearing connector. Callers must not log or serialize the result.
    pub fn notion_runtime_config(&self) -> Result<Option<NotionRuntimeConfig>, LifecycleError> {
        let raw = read_private_file(&self.config_path, self.expected_uid, 256 * 1024)?;
        Ok(parse_config(&raw)?.notion)
    }
}

fn resolve_entries_root(launch_root: &Path, home: &Path, configured: PathBuf) -> PathBuf {
    if configured.as_os_str().is_empty() {
        return home.join("Documents/narihitos-diary/entries");
    }
    if configured.is_absolute() {
        return configured;
    }
    if configured == Path::new("~") {
        return home.to_path_buf();
    }
    if let Ok(relative) = configured.strip_prefix("~") {
        return home.join(relative);
    }
    launch_root.join(configured)
}

#[derive(Debug)]
pub struct ServerLock {
    file: File,
}

impl ServerLock {
    pub fn acquire(runtime_dir: &Path, expected_uid: u32) -> Result<Self, LifecycleError> {
        ensure_private_directory(runtime_dir, expected_uid)?;
        let path = runtime_dir.join("control-api.lock");
        let file = open_private_regular(&path, expected_uid)?;
        file.try_lock_exclusive()
            .map_err(|_| LifecycleError::AlreadyRunning)?;
        Ok(Self { file })
    }
}

impl Drop for ServerLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAdvertisement {
    pub version: u32,
    pub instance_id: String,
    pub host: String,
    pub port: u16,
    pub control_token: String,
    pub contract_version: u32,
}

/// Validated native-client attachment data. This intentionally has no
/// `Debug` implementation because it contains the bearer token.
#[derive(Clone)]
pub struct RuntimeAttachment {
    instance_id: String,
    host: String,
    port: u16,
    control_token: String,
    contract_version: u32,
}

impl RuntimeAttachment {
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub fn control_token(&self) -> &str {
        &self.control_token
    }

    #[must_use]
    pub const fn contract_version(&self) -> u32 {
        self.contract_version
    }
}

/// Reads an existing runtime advertisement without creating private state or
/// consulting product configuration.
pub fn load_runtime_attachment(
    runtime_dir: &Path,
    expected_uid: u32,
) -> Result<RuntimeAttachment, LifecycleError> {
    let directory_path = match fs::symlink_metadata(runtime_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(LifecycleError::MissingRuntime);
        }
        Err(_) => return Err(LifecycleError::Io),
    };
    if !directory_path.file_type().is_dir()
        || directory_path.file_type().is_symlink()
        || directory_path.uid() != expected_uid
        || directory_path.mode() & 0o077 != 0
    {
        return Err(LifecycleError::UnsafePath);
    }
    let directory = File::open(runtime_dir).map_err(|_| LifecycleError::Io)?;
    let directory_opened = directory.metadata().map_err(|_| LifecycleError::Io)?;
    if !same_identity(&directory_path, &directory_opened)
        || !directory_opened.file_type().is_dir()
        || directory_opened.uid() != expected_uid
        || directory_opened.mode() & 0o077 != 0
    {
        return Err(LifecycleError::UnsafePath);
    }

    let path = runtime_dir.join("control-api.json");
    let path_metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(LifecycleError::MissingRuntime);
        }
        Err(_) => return Err(LifecycleError::Io),
    };
    if !path_metadata.file_type().is_file()
        || path_metadata.file_type().is_symlink()
        || path_metadata.uid() != expected_uid
        || path_metadata.nlink() != 1
        || path_metadata.mode() & 0o077 != 0
        || path_metadata.len() > 64 * 1024
    {
        return Err(LifecycleError::UnsafePath);
    }
    let mut file = File::open(&path).map_err(|_| LifecycleError::Io)?;
    let opened = file.metadata().map_err(|_| LifecycleError::Io)?;
    if !same_identity(&path_metadata, &opened)
        || !opened.file_type().is_file()
        || opened.uid() != expected_uid
        || opened.nlink() != 1
        || opened.mode() & 0o077 != 0
        || opened.len() > 64 * 1024
    {
        return Err(LifecycleError::UnsafePath);
    }
    let mut raw = String::new();
    Read::by_ref(&mut file)
        .take(64 * 1024 + 1)
        .read_to_string(&mut raw)
        .map_err(|_| LifecycleError::InvalidRuntime)?;
    if raw.len() > 64 * 1024 {
        return Err(LifecycleError::InvalidRuntime);
    }

    let opened_after_read = file.metadata().map_err(|_| LifecycleError::Io)?;
    let current_file = fs::symlink_metadata(&path).map_err(|_| LifecycleError::UnsafePath)?;
    let current_directory =
        fs::symlink_metadata(runtime_dir).map_err(|_| LifecycleError::UnsafePath)?;
    if !same_file_state(&opened, &opened_after_read)
        || !same_file_state(&opened_after_read, &current_file)
        || !current_file.file_type().is_file()
        || current_file.uid() != expected_uid
        || current_file.nlink() != 1
        || current_file.mode() & 0o077 != 0
        || current_file.len() > 64 * 1024
        || !same_identity(&directory_opened, &current_directory)
        || !current_directory.file_type().is_dir()
        || current_directory.uid() != expected_uid
        || current_directory.mode() & 0o077 != 0
    {
        return Err(LifecycleError::UnsafePath);
    }
    let advertisement: RuntimeAdvertisement =
        serde_json::from_str(raw.trim()).map_err(|_| LifecycleError::InvalidRuntime)?;
    if advertisement.version != 1
        || advertisement.host != "127.0.0.1"
        || advertisement.port == 0
        || advertisement.contract_version == 0
        || !random_value_valid(&advertisement.instance_id, "instance_")
        || !random_value_valid(&advertisement.control_token, "token_")
    {
        return Err(LifecycleError::InvalidRuntime);
    }
    if advertisement.contract_version != CONTRACT_VERSION {
        return Err(LifecycleError::IncompatibleContract);
    }
    Ok(RuntimeAttachment {
        instance_id: advertisement.instance_id,
        host: advertisement.host,
        port: advertisement.port,
        control_token: advertisement.control_token,
        contract_version: advertisement.contract_version,
    })
}

fn same_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    same_identity(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

fn random_value_valid(value: &str, prefix: &str) -> bool {
    value.len() == prefix.len() + 64
        && value.starts_with(prefix)
        && value[prefix.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

impl RuntimeAdvertisement {
    pub fn publish(
        &self,
        runtime_dir: &Path,
        expected_uid: u32,
    ) -> Result<RuntimeFile, LifecycleError> {
        let path = runtime_dir.join("control-api.json");
        if path.exists() {
            validate_private_regular(&path, expected_uid)?;
        }
        let temp = runtime_dir.join(format!(".control-api.{}.tmp", self.instance_id));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|_| LifecycleError::Io)?;
        let encoded = serde_json::to_vec(self).map_err(|_| LifecycleError::InvalidRuntime)?;
        file.write_all(&encoded).map_err(|_| LifecycleError::Io)?;
        file.write_all(b"\n").map_err(|_| LifecycleError::Io)?;
        file.sync_all().map_err(|_| LifecycleError::Io)?;
        validate_private_regular(&temp, expected_uid)?;
        fs::rename(&temp, &path).map_err(|_| LifecycleError::Io)?;
        File::open(runtime_dir)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| LifecycleError::Io)?;
        validate_private_regular(&path, expected_uid)?;
        Ok(RuntimeFile {
            path,
            instance_id: self.instance_id.clone(),
            expected_uid,
        })
    }
}

#[derive(Debug)]
pub struct RuntimeFile {
    path: PathBuf,
    instance_id: String,
    expected_uid: u32,
}

impl RuntimeFile {
    pub fn remove_if_owned(&self) -> Result<(), LifecycleError> {
        let path_metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(LifecycleError::Io),
        };
        if path_metadata.file_type().is_symlink() {
            return Err(LifecycleError::UnsafePath);
        }
        let mut raw = String::new();
        match File::open(&self.path).and_then(|mut file| {
            let metadata = file.metadata()?;
            if !metadata.file_type().is_file()
                || metadata.uid() != self.expected_uid
                || metadata.nlink() != 1
                || metadata.mode() & 0o077 != 0
                || metadata.dev() != path_metadata.dev()
                || metadata.ino() != path_metadata.ino()
                || metadata.len() > 64 * 1024
            {
                return Err(std::io::Error::other("unsafe runtime file"));
            }
            Read::by_ref(&mut file)
                .take(64 * 1024 + 1)
                .read_to_string(&mut raw)?;
            Ok(())
        }) {
            Ok(_) => {
                if raw.len() > 64 * 1024 {
                    return Err(LifecycleError::InvalidRuntime);
                }
                let advertisement: RuntimeAdvertisement =
                    serde_json::from_str(raw.trim()).map_err(|_| LifecycleError::InvalidRuntime)?;
                if advertisement.instance_id == self.instance_id {
                    let current =
                        fs::symlink_metadata(&self.path).map_err(|_| LifecycleError::Io)?;
                    if current.file_type().is_symlink()
                        || current.dev() != path_metadata.dev()
                        || current.ino() != path_metadata.ino()
                    {
                        return Err(LifecycleError::UnsafePath);
                    }
                    fs::remove_file(&self.path).map_err(|_| LifecycleError::Io)?;
                    File::open(self.path.parent().ok_or(LifecycleError::InvalidRuntime)?)
                        .and_then(|directory| directory.sync_all())
                        .map_err(|_| LifecycleError::Io)?;
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(LifecycleError::Io),
        }
    }
}

pub fn random_id(prefix: &str) -> Result<String, LifecycleError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| LifecycleError::Random)?;
    let mut value = String::with_capacity(prefix.len() + 64);
    value.push_str(prefix);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").map_err(|_| LifecycleError::Random)?;
    }
    Ok(value)
}

fn read_private_file(
    path: &Path,
    expected_uid: u32,
    maximum: usize,
) -> Result<String, LifecycleError> {
    let path_metadata = fs::symlink_metadata(path).map_err(|_| LifecycleError::MissingConfig)?;
    if path_metadata.file_type().is_symlink() {
        return Err(LifecycleError::UnsafeConfig);
    }
    let mut file = File::open(path).map_err(|_| LifecycleError::MissingConfig)?;
    let metadata = file.metadata().map_err(|_| LifecycleError::Io)?;
    if !metadata.file_type().is_file()
        || metadata.uid() != expected_uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
        || metadata.dev() != path_metadata.dev()
        || metadata.ino() != path_metadata.ino()
        || metadata.len() > maximum as u64
    {
        return Err(LifecycleError::UnsafeConfig);
    }
    let mut raw = String::new();
    Read::by_ref(&mut file)
        .take(maximum as u64 + 1)
        .read_to_string(&mut raw)
        .map_err(|_| LifecycleError::UnsafeConfig)?;
    if raw.len() > maximum {
        return Err(LifecycleError::UnsafeConfig);
    }
    Ok(raw)
}

struct ParsedConfig {
    port_fallback_count: u16,
    notion_configured: bool,
    notion: Option<NotionRuntimeConfig>,
    ayumi_directory_name: String,
    entries_root: PathBuf,
    icloud_entries_root: Option<PathBuf>,
    codex_profiles: Vec<CodexProfileConfig>,
}

fn parse_config(raw: &str) -> Result<ParsedConfig, LifecycleError> {
    let mut parsed = ParsedConfig {
        port_fallback_count: 8,
        notion_configured: false,
        notion: None,
        ayumi_directory_name: String::new(),
        entries_root: PathBuf::new(),
        icloud_entries_root: None,
        codex_profiles: Vec::new(),
    };
    let mut section = "";
    let mut sections = std::collections::BTreeSet::new();
    let mut saw_fallback = false;
    let mut saw_sources = false;
    let mut saw_entries_root = false;
    let mut saw_icloud_entries_root = false;
    let mut saw_notion_token = false;
    let mut saw_notion_root = false;
    let mut saw_codex_profiles = false;
    let mut notion_token = None;
    let mut notion_root = None;
    let mut profile = None;
    for original in raw.lines() {
        let line = original.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if original.contains('\t') {
            return Err(LifecycleError::InvalidConfig);
        }
        if !original.starts_with(' ') && line.ends_with(':') {
            finish_profile(&mut parsed.codex_profiles, profile.take())?;
            section = line.trim_end_matches(':');
            if !matches!(
                section,
                "sources" | "paths" | "notion" | "server" | "assistants"
            ) || !sections.insert(section)
            {
                return Err(LifecycleError::InvalidConfig);
            }
            continue;
        }
        if section == "assistants" {
            if original.starts_with("  ") && !original.starts_with("    ") {
                finish_profile(&mut parsed.codex_profiles, profile.take())?;
                if line != "codex_profiles:" || saw_codex_profiles {
                    return Err(LifecycleError::InvalidConfig);
                }
                saw_codex_profiles = true;
                continue;
            }
            if original.starts_with("    - ") && !original.starts_with("      ") {
                if !saw_codex_profiles {
                    return Err(LifecycleError::InvalidConfig);
                }
                finish_profile(&mut parsed.codex_profiles, profile.take())?;
                profile = Some(ProfileFields::default());
                parse_profile_field(
                    profile.as_mut().expect("profile was just created"),
                    line.strip_prefix("- ")
                        .ok_or(LifecycleError::InvalidConfig)?,
                )?;
                continue;
            }
            if original.starts_with("      ") && !original.starts_with("       ") {
                parse_profile_field(profile.as_mut().ok_or(LifecycleError::InvalidConfig)?, line)?;
                continue;
            }
            return Err(LifecycleError::InvalidConfig);
        }
        if !original.starts_with("  ") || original.starts_with("   ") {
            return Err(LifecycleError::InvalidConfig);
        }
        let (key, value) = line.split_once(':').ok_or(LifecycleError::InvalidConfig)?;
        let value = parse_scalar(value)?;
        match (section, key.trim()) {
            ("server", "port_fallback_count") => {
                if saw_fallback {
                    return Err(LifecycleError::InvalidConfig);
                }
                parsed.port_fallback_count =
                    value.parse().map_err(|_| LifecycleError::InvalidConfig)?;
                if parsed.port_fallback_count > 16 {
                    return Err(LifecycleError::InvalidConfig);
                }
                saw_fallback = true;
            }
            ("sources", "ayumi_directory_name") => {
                if saw_sources || value.is_empty() {
                    return Err(LifecycleError::InvalidConfig);
                }
                parsed.ayumi_directory_name = value;
                saw_sources = true;
            }
            ("paths", "entries_root") => {
                if saw_entries_root {
                    return Err(LifecycleError::InvalidConfig);
                }
                parsed.entries_root = PathBuf::from(value);
                saw_entries_root = true;
            }
            ("paths", "icloud_entries_root") => {
                if saw_icloud_entries_root || value.is_empty() {
                    return Err(LifecycleError::InvalidConfig);
                }
                parsed.icloud_entries_root = Some(PathBuf::from(value));
                saw_icloud_entries_root = true;
            }
            ("notion", "token") => {
                if saw_notion_token {
                    return Err(LifecycleError::InvalidConfig);
                }
                if !value.is_empty() && !value.starts_with("REPLACE_") {
                    notion_token = Some(value);
                }
                saw_notion_token = true;
            }
            ("notion", "root_page_id") => {
                if saw_notion_root {
                    return Err(LifecycleError::InvalidConfig);
                }
                if !value.is_empty() && !value.starts_with("REPLACE_") {
                    notion_root = Some(normalize_notion_page_id(&value)?);
                }
                saw_notion_root = true;
            }
            _ => return Err(LifecycleError::InvalidConfig),
        }
    }
    finish_profile(&mut parsed.codex_profiles, profile)?;
    if sections.contains("assistants") && !saw_codex_profiles {
        return Err(LifecycleError::InvalidConfig);
    }
    parsed.notion =
        notion_token
            .zip(notion_root)
            .map(|(token, root_page_id)| NotionRuntimeConfig {
                token,
                root_page_id,
            });
    parsed.notion_configured = parsed.notion.is_some();
    if parsed.ayumi_directory_name.is_empty() {
        return Err(LifecycleError::InvalidConfig);
    }
    Ok(parsed)
}

fn normalize_notion_page_id(value: &str) -> Result<String, LifecycleError> {
    let compact: String = value
        .chars()
        .filter(|character| *character != '-')
        .collect();
    if compact.len() != 32 || !compact.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(LifecycleError::InvalidConfig);
    }
    Ok(compact.to_ascii_lowercase())
}

#[derive(Default)]
struct ProfileFields {
    alias: Option<String>,
    codex_home: Option<String>,
    codex_executable: Option<String>,
}

fn parse_profile_field(fields: &mut ProfileFields, line: &str) -> Result<(), LifecycleError> {
    let (key, value) = line.split_once(':').ok_or(LifecycleError::InvalidConfig)?;
    let value = parse_scalar(value)?;
    match key.trim() {
        "alias" if fields.alias.is_none() => fields.alias = Some(value),
        "codex_home" if fields.codex_home.is_none() => fields.codex_home = Some(value),
        "codex_executable" if fields.codex_executable.is_none() => {
            fields.codex_executable = Some(value);
        }
        _ => return Err(LifecycleError::InvalidConfig),
    }
    Ok(())
}

fn finish_profile(
    profiles: &mut Vec<CodexProfileConfig>,
    fields: Option<ProfileFields>,
) -> Result<(), LifecycleError> {
    let Some(fields) = fields else {
        return Ok(());
    };
    if profiles.len() >= MAX_COLLECTION_ITEMS {
        return Err(LifecycleError::InvalidConfig);
    }
    let alias = fields.alias.ok_or(LifecycleError::InvalidConfig)?;
    if alias.is_empty()
        || alias.len() > 32
        || !alias.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
        || profiles.iter().any(|profile| profile.alias == alias)
    {
        return Err(LifecycleError::InvalidConfig);
    }
    let codex_home =
        validated_absolute_path(fields.codex_home.ok_or(LifecycleError::InvalidConfig)?)?;
    let codex_executable = fields
        .codex_executable
        .map(validated_absolute_path)
        .transpose()?;
    profiles.push(CodexProfileConfig {
        alias,
        codex_home,
        codex_executable,
    });
    Ok(())
}

fn parse_scalar(raw: &str) -> Result<String, LifecycleError> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    let value = if value.starts_with('"') {
        value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .ok_or(LifecycleError::InvalidConfig)?
    } else if value.contains('"') {
        return Err(LifecycleError::InvalidConfig);
    } else {
        value
    };
    if value.chars().any(char::is_control) {
        return Err(LifecycleError::InvalidConfig);
    }
    Ok(value.to_owned())
}

fn validated_absolute_path(value: String) -> Result<PathBuf, LifecycleError> {
    if value.is_empty() || value.len() > 4 * 1024 || value.chars().any(char::is_control) {
        return Err(LifecycleError::InvalidConfig);
    }
    let path = PathBuf::from(value);
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(LifecycleError::InvalidConfig);
    }
    Ok(path)
}

/// Loads the persistent 32-byte key behind diagnostic correlation tags,
/// creating it (random, mode 0600) on first use. The key stays beside the
/// Application Support state, never under the agent-readable log directory,
/// so a tag in the log cannot be reversed to the value it stands for.
pub fn load_or_create_tag_key(path: &Path, expected_uid: u32) -> Result<[u8; 32], LifecycleError> {
    let mut file = open_private_regular(path, expected_uid)?;
    let mut raw = String::new();
    Read::by_ref(&mut file)
        .take(65)
        .read_to_string(&mut raw)
        .map_err(|_| LifecycleError::Io)?;
    if raw.is_empty() {
        let mut key = [0_u8; 32];
        getrandom::fill(&mut key).map_err(|_| LifecycleError::Random)?;
        let mut hex = String::with_capacity(64);
        for byte in key {
            use std::fmt::Write as _;
            write!(&mut hex, "{byte:02x}").map_err(|_| LifecycleError::Random)?;
        }
        file.write_all(hex.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|_| LifecycleError::Io)?;
        return Ok(key);
    }
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(LifecycleError::UnsafePath);
    }
    let mut key = [0_u8; 32];
    let (pairs, _) = raw.as_bytes().as_chunks::<2>();
    for (slot, pair) in key.iter_mut().zip(pairs) {
        *slot = u8::from_str_radix(
            std::str::from_utf8(pair).map_err(|_| LifecycleError::Io)?,
            16,
        )
        .map_err(|_| LifecycleError::UnsafePath)?;
    }
    Ok(key)
}

fn ensure_private_directory(path: &Path, expected_uid: u32) -> Result<(), LifecycleError> {
    if !path.exists() {
        fs::create_dir(path).map_err(|_| LifecycleError::Io)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| LifecycleError::Io)?;
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| LifecycleError::Io)?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != expected_uid
        || metadata.mode() & 0o077 != 0
    {
        return Err(LifecycleError::UnsafePath);
    }
    Ok(())
}

fn ensure_private_regular(path: &Path, expected_uid: u32) -> Result<(), LifecycleError> {
    if !path.exists() {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|_| LifecycleError::Io)?;
    }
    validate_private_regular(path, expected_uid)
}

fn open_private_regular(path: &Path, expected_uid: u32) -> Result<File, LifecycleError> {
    ensure_private_regular(path, expected_uid)?;
    let path_metadata = fs::symlink_metadata(path).map_err(|_| LifecycleError::Io)?;
    if path_metadata.file_type().is_symlink() {
        return Err(LifecycleError::UnsafePath);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| LifecycleError::Io)?;
    let metadata = file.metadata().map_err(|_| LifecycleError::Io)?;
    if !metadata.file_type().is_file()
        || metadata.uid() != expected_uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
        || metadata.dev() != path_metadata.dev()
        || metadata.ino() != path_metadata.ino()
    {
        return Err(LifecycleError::UnsafePath);
    }
    Ok(file)
}

fn validate_private_regular(path: &Path, expected_uid: u32) -> Result<(), LifecycleError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| LifecycleError::Io)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != expected_uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
    {
        return Err(LifecycleError::UnsafePath);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleError {
    MissingHome,
    MissingConfig,
    MissingRuntime,
    UnsafeConfig,
    InvalidConfig,
    UnsafePath,
    AlreadyRunning,
    InvalidRuntime,
    IncompatibleContract,
    Random,
    Io,
}

#[cfg(test)]
mod tests {
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
            parse_config("sources:\n  ayumi_directory_name: Ayumi\nunknown:\n  value: 1\n")
                .is_err()
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
}
