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
mod tests;
