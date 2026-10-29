use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use diary_control_contract::CONTRACT_VERSION;
use fs2::FileExt;
use serde::{Deserialize, Serialize};

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
    pub notion_configured: bool,
    pub expected_uid: u32,
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
        Ok(Self {
            launch_root: launch_root.clone(),
            config_path,
            log_dir: launch_root.join(".local/logs/control-api"),
            database_path: application_support.join("state.sqlite3"),
            application_support,
            runtime_dir,
            port_fallback_count: parsed.port_fallback_count,
            ayumi_directory_name: parsed.ayumi_directory_name,
            notion_configured: parsed.notion_configured,
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
    ayumi_directory_name: String,
}

fn parse_config(raw: &str) -> Result<ParsedConfig, LifecycleError> {
    let mut parsed = ParsedConfig {
        port_fallback_count: 8,
        notion_configured: false,
        ayumi_directory_name: String::new(),
    };
    let mut section = "";
    let mut saw_fallback = false;
    let mut notion_token = false;
    let mut notion_root = false;
    for original in raw.lines() {
        let line = original.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !original.starts_with(' ') && line.ends_with(':') {
            section = line.trim_end_matches(':');
            if !matches!(section, "sources" | "paths" | "notion" | "server") {
                return Err(LifecycleError::InvalidConfig);
            }
            continue;
        }
        let (key, value) = line.split_once(':').ok_or(LifecycleError::InvalidConfig)?;
        let value = value.trim().trim_matches('"');
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
                if !parsed.ayumi_directory_name.is_empty() || value.is_empty() {
                    return Err(LifecycleError::InvalidConfig);
                }
                parsed.ayumi_directory_name = value.to_owned();
            }
            ("paths", "entries_root") => {}
            ("notion", "token") => {
                notion_token = !value.is_empty() && !value.starts_with("REPLACE_")
            }
            ("notion", "root_page_id") => {
                notion_root = !value.is_empty() && !value.starts_with("REPLACE_")
            }
            _ => return Err(LifecycleError::InvalidConfig),
        }
    }
    parsed.notion_configured = notion_token && notion_root;
    if parsed.ayumi_directory_name.is_empty() {
        return Err(LifecycleError::InvalidConfig);
    }
    Ok(parsed)
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
    fn strict_config_bounds_server_fallback_and_detects_notion_placeholders() {
        let parsed = parse_config(
            "sources:\n  ayumi_directory_name: \"Ayumi\"\npaths:\n  entries_root: \"ignored\"\nnotion:\n  token: \"REPLACE_TOKEN\"\n  root_page_id: \"REPLACE_ROOT\"\nserver:\n  port_fallback_count: 16\n",
        )
        .unwrap();
        assert_eq!(parsed.port_fallback_count, 16);
        assert_eq!(parsed.ayumi_directory_name, "Ayumi");
        assert!(!parsed.notion_configured);
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
            notion_configured: false,
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
