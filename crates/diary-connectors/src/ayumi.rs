//! Bounded, privacy-preserving Ayumi Markdown source adapter.

use std::{
    fs::{self, File},
    io::{ErrorKind, Read},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    sync::Arc,
    sync::atomic::AtomicBool,
    time::UNIX_EPOCH,
};

use diary_core::{
    CatalogRequest, DateEvidence, DiaryDay, Digest, FreshnessOutcome, FreshnessStamp,
    SelectedSource, SourceAccessError, SourceAvailability, SourceCandidate, SourceCatalog,
    SourceDiscovery, SourceReadOutcome, SourceReader, SourceRef, SourceSnapshot, Timestamp,
};

use crate::{AppleBridgeClient, BridgeAvailability, BridgeRead};

const CONNECTOR: &str = "ayumi";
/// Deliberately below the helper's handoff bound.  Oversize input is never
/// mistaken for an empty, readable note.
pub const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Discovery {
    pub candidates: Vec<SourceCandidate>,
    /// Content-free diagnostics suitable for a UI.
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadOutcome {
    Read(Box<SelectedSource>),
    NeedsDownload,
    PermissionDenied,
    Unreadable,
    Changed,
    Unverifiable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceReadError {
    InvalidDirectoryName,
    RootUnavailable,
    UnknownSource,
    Access(SourceAccessError),
}

/// Errors from a write-back or delete operation on an Ayumi source. Mirrors
/// the naming style of [`SourceReadError`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceWriteError {
    /// The file's current content no longer matches the caller's expected
    /// digest; nothing was written.
    Stale,
    /// A filesystem operation failed.
    Io,
    /// A path (the source itself or a linked audio file) failed the same
    /// symlink/traversal/containment checks used for reads; nothing was
    /// written or deleted.
    UnsafePath,
}

/// Catalog rooted at the already-resolved iCloud Drive location.  The
/// configured Ayumi value is strictly one directory component beneath it.
#[derive(Clone, Debug)]
pub struct AyumiCatalog {
    root: PathBuf,
    directory_name: String,
}

impl AyumiCatalog {
    pub fn new(
        root: impl Into<PathBuf>,
        directory_name: impl Into<String>,
    ) -> Result<Self, SourceReadError> {
        let directory_name = directory_name.into();
        if directory_name.is_empty()
            || directory_name == "."
            || directory_name == ".."
            || directory_name.contains(['/', '\\', '\0'])
            || Path::new(&directory_name).components().count() != 1
            || Path::new(&directory_name).is_absolute()
        {
            return Err(SourceReadError::InvalidDirectoryName);
        }
        Ok(Self {
            root: root.into(),
            directory_name,
        })
    }

    pub fn discover(&self, day: &DiaryDay) -> Result<Discovery, SourceReadError> {
        let mut days = vec![shift_day(day, -1), day.clone(), shift_day(day, 1)];
        self.discover_days(&mut days, &mut |_| Ok(SourceAvailability::Resident))
    }

    /// Queries precisely one requested extra calendar day; it never expands
    /// into an archive scan.
    pub fn discover_extra_day(&self, day: &DiaryDay) -> Result<Discovery, SourceReadError> {
        self.discover_days(&mut vec![day.clone()], &mut |_| {
            Ok(SourceAvailability::Resident)
        })
    }

    pub fn read_selected(&self, source: &SourceRef) -> Result<ReadOutcome, SourceReadError> {
        let path = self.resolve(source)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
            Ok(_) => return Ok(ReadOutcome::Unverifiable),
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                return Ok(ReadOutcome::PermissionDenied);
            }
            Err(_) => return Ok(ReadOutcome::Unverifiable),
        };
        if metadata.len() > MAX_SOURCE_BYTES {
            return Ok(ReadOutcome::Unreadable);
        }
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                return Ok(ReadOutcome::PermissionDenied);
            }
            Err(_) => return Ok(ReadOutcome::Unreadable),
        };
        let opened = match file.metadata() {
            Ok(value) if value.is_file() => value,
            Ok(_) => return Ok(ReadOutcome::Unverifiable),
            Err(_) => return Ok(ReadOutcome::Unverifiable),
        };
        if metadata_identity(&metadata) != metadata_identity(&opened) {
            return Ok(ReadOutcome::Changed);
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        if Read::by_ref(&mut file)
            .take(MAX_SOURCE_BYTES + 1)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return Ok(ReadOutcome::Unreadable);
        }
        let after_opened = match file.metadata() {
            Ok(value) => value,
            Err(_) => return Ok(ReadOutcome::Unverifiable),
        };
        let after_path = match fs::symlink_metadata(&path) {
            Ok(value) if value.is_file() && !value.file_type().is_symlink() => value,
            Ok(_) => return Ok(ReadOutcome::Changed),
            Err(_) => return Ok(ReadOutcome::Unverifiable),
        };
        if bytes.len() as u64 > MAX_SOURCE_BYTES
            || bytes.len() as u64 != metadata.len()
            || metadata_identity(&metadata) != metadata_identity(&after_opened)
            || metadata_identity(&metadata) != metadata_identity(&after_path)
        {
            return Ok(ReadOutcome::Changed);
        }
        let parsed = match parse_markdown(&bytes) {
            Some(value) => value,
            None => return Ok(ReadOutcome::Unreadable),
        };
        if parsed.ambiguous_attachment {
            return Ok(ReadOutcome::Unreadable);
        }
        let stamp = stamp(&after_opened, &bytes)?;
        Ok(ReadOutcome::Read(Box::new(selected_source(
            source, parsed, stamp,
        )?)))
    }

    /// Re-reads exactly the selected source and compares every stamp field,
    /// including the original-byte digest (so same-size/same-mtime changes win).
    pub fn revalidate(&self, snapshot: &SourceSnapshot) -> Result<ReadOutcome, SourceReadError> {
        let outcome = match self.read_selected(&snapshot.source) {
            Ok(outcome) => outcome,
            Err(SourceReadError::UnknownSource | SourceReadError::RootUnavailable) => {
                return Ok(ReadOutcome::Unverifiable);
            }
            Err(error) => return Err(error),
        };
        match outcome {
            ReadOutcome::Read(current) if current.snapshot.stamp == snapshot.stamp => {
                Ok(ReadOutcome::Read(current))
            }
            ReadOutcome::Read(_) => Ok(ReadOutcome::Changed),
            other => Ok(other),
        }
    }

    fn discover_days(
        &self,
        days: &mut Vec<DiaryDay>,
        availability_probe: &mut impl FnMut(&SourceRef) -> Result<SourceAvailability, SourceReadError>,
    ) -> Result<Discovery, SourceReadError> {
        let base = self.base()?;
        days.sort();
        days.dedup();
        let mut candidates = Vec::new();
        let mut warnings = Vec::new();
        for day in days {
            let dir = base.join(day.as_str().replace('-', "/"));
            let directory_metadata = match fs::symlink_metadata(&dir) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(_) => {
                    warnings.push("source date directory is unavailable".into());
                    continue;
                }
            };
            if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
                warnings.push("source date directory is not a regular directory".into());
                continue;
            }
            let canonical_dir = match fs::canonicalize(&dir) {
                Ok(path) if path.starts_with(&base) => path,
                _ => {
                    warnings.push("source date directory failed containment checks".into());
                    continue;
                }
            };
            let directory_entries = match fs::read_dir(&canonical_dir) {
                Ok(entries) => entries,
                Err(_) => {
                    warnings.push("source date directory is unreadable".into());
                    continue;
                }
            };
            let mut entries = Vec::new();
            for entry in directory_entries {
                match entry {
                    Ok(entry) => entries.push(entry),
                    Err(_) => warnings.push("source directory entry is unavailable".into()),
                }
            }
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    warnings.push("source filename is not valid UTF-8".into());
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }
                if !name.ends_with(".md") {
                    warnings.push("source entry is not a Markdown file".into());
                    continue;
                }
                let meta = match fs::symlink_metadata(entry.path()) {
                    Ok(value) => value,
                    Err(_) => {
                        warnings.push("source metadata unavailable".into());
                        continue;
                    }
                };
                if meta.file_type().is_symlink() || !meta.is_file() {
                    warnings.push("source entry is not a regular file".into());
                    continue;
                }
                let relative = format!(
                    "{}/{}/{}/{}",
                    &day.as_str()[0..4],
                    &day.as_str()[5..7],
                    &day.as_str()[8..10],
                    name
                );
                let source_note_name = relative.clone();
                let source = SourceRef::new(CONNECTOR, relative)
                    .map_err(|_| SourceReadError::UnknownSource)?;
                let availability = availability_probe(&source)?;
                let (header_day, warning) = if availability == SourceAvailability::Resident {
                    if meta.len() > MAX_SOURCE_BYTES {
                        (None, Some("source exceeds the readable size limit".into()))
                    } else {
                        match read_bounded(&entry.path(), &meta) {
                            Ok(Some(bytes)) => match parse_markdown(&bytes) {
                                Some(parsed) => {
                                    let attachment_warning = parsed
                                        .ambiguous_attachment
                                        .then(|| "source attachment syntax is unsafe".into());
                                    (
                                        parsed.header_day,
                                        merge_warnings(parsed.warning, attachment_warning),
                                    )
                                }
                                None => (None, Some("source is not valid UTF-8".into())),
                            },
                            Ok(None) => {
                                (None, Some("source exceeds the readable size limit".into()))
                            }
                            Err(_) => (None, Some("source body is unavailable".into())),
                        }
                    }
                } else {
                    (
                        None,
                        Some(match availability {
                            SourceAvailability::CloudOnly => "source needs download".into(),
                            SourceAvailability::PermissionDenied => {
                                "source permission was denied".into()
                            }
                            SourceAvailability::Unknown => "source availability is unknown".into(),
                            SourceAvailability::Resident => unreachable!(),
                        }),
                    )
                };
                let warning =
                    merge_warnings(disagreement_warning(day, header_day.as_ref()), warning);
                candidates.push(SourceCandidate {
                    source,
                    source_note_name,
                    date_evidence: DateEvidence {
                        directory_day: Some(day.clone()),
                        header_day,
                        warning,
                    },
                    availability,
                });
            }
        }
        Ok(Discovery {
            candidates,
            warnings,
        })
    }

    fn base(&self) -> Result<PathBuf, SourceReadError> {
        let root = fs::canonicalize(&self.root).map_err(|_| SourceReadError::RootUnavailable)?;
        let base = root.join(&self.directory_name);
        let metadata = fs::symlink_metadata(&base).map_err(|_| SourceReadError::RootUnavailable)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(SourceReadError::RootUnavailable);
        }
        let canonical = fs::canonicalize(base).map_err(|_| SourceReadError::RootUnavailable)?;
        if !canonical.starts_with(&root) {
            return Err(SourceReadError::RootUnavailable);
        }
        Ok(canonical)
    }

    fn resolve(&self, source: &SourceRef) -> Result<PathBuf, SourceReadError> {
        if source.connector != CONNECTOR {
            return Err(SourceReadError::UnknownSource);
        }
        let relative = Path::new(&source.source_id);
        if !valid_source_path(relative) {
            return Err(SourceReadError::UnknownSource);
        }
        let base = self.base()?;
        let path = base.join(relative);
        let metadata = fs::symlink_metadata(&path).map_err(|_| SourceReadError::UnknownSource)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(SourceReadError::UnknownSource);
        }
        let canonical = fs::canonicalize(&path).map_err(|_| SourceReadError::UnknownSource)?;
        if !canonical.starts_with(&base) {
            return Err(SourceReadError::UnknownSource);
        }
        Ok(canonical)
    }

    /// Overwrites the Markdown body of `source`, preserving its front matter
    /// (including `created_at`) byte-for-byte. Rejects the write with
    /// `SourceWriteError::Stale` if the file's current readable body does not
    /// match `expected_content_digest`, without modifying anything.
    pub fn overwrite_body(
        &self,
        source: &SourceRef,
        new_body: &str,
        expected_content_digest: &Digest,
    ) -> Result<FreshnessStamp, SourceWriteError> {
        let path = self
            .resolve(source)
            .map_err(|_| SourceWriteError::UnsafePath)?;
        let bytes = fs::read(&path).map_err(|_| SourceWriteError::Io)?;
        let parsed = parse_markdown(&bytes).ok_or(SourceWriteError::Io)?;
        if Digest::of_bytes(parsed.body.as_bytes()) != *expected_content_digest {
            return Err(SourceWriteError::Stale);
        }

        let header = front_matter_header(&bytes);
        let new_contents = match header {
            Some(header) => format!("---\n{header}\n---\n{new_body}"),
            None => new_body.to_owned(),
        };

        // Re-check immediately before writing: the path must still be a
        // non-symlink regular file inside the catalog base. This guards
        // against the file being swapped for a symlink between the read
        // above and this write.
        let metadata = fs::symlink_metadata(&path).map_err(|_| SourceWriteError::Io)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(SourceWriteError::UnsafePath);
        }

        fs::write(&path, new_contents.as_bytes()).map_err(|_| SourceWriteError::Io)?;
        let metadata = fs::symlink_metadata(&path).map_err(|_| SourceWriteError::Io)?;
        stamp(&metadata, new_contents.as_bytes()).map_err(|_| SourceWriteError::Io)
    }

    /// Deletes `source` and every locally-resolvable audio file linked from
    /// its body. Each linked audio destination is resolved against the
    /// note's own parent directory and must pass the same
    /// symlink/traversal/containment checks as [`Self::resolve`]; if any
    /// resolved audio path escapes that directory, nothing is deleted and
    /// `SourceWriteError::UnsafePath` is returned. On success, returns the
    /// paths actually deleted, audio files first and the note file last.
    pub fn delete_source(&self, source: &SourceRef) -> Result<Vec<PathBuf>, SourceWriteError> {
        let path = self
            .resolve(source)
            .map_err(|_| SourceWriteError::UnsafePath)?;
        let parent = path.parent().ok_or(SourceWriteError::UnsafePath)?;
        let canonical_parent = fs::canonicalize(parent).map_err(|_| SourceWriteError::Io)?;

        let bytes = fs::read(&path).map_err(|_| SourceWriteError::Io)?;
        let parsed = parse_markdown(&bytes).ok_or(SourceWriteError::Io)?;

        let mut audio_paths = Vec::with_capacity(parsed.audio_links.len());
        for link in &parsed.audio_links {
            let relative = Path::new(link);
            if relative.is_absolute() {
                return Err(SourceWriteError::UnsafePath);
            }
            let candidate = canonical_parent.join(relative);
            let metadata =
                fs::symlink_metadata(&candidate).map_err(|_| SourceWriteError::UnsafePath)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(SourceWriteError::UnsafePath);
            }
            let canonical =
                fs::canonicalize(&candidate).map_err(|_| SourceWriteError::UnsafePath)?;
            if !canonical.starts_with(&canonical_parent) {
                return Err(SourceWriteError::UnsafePath);
            }
            audio_paths.push(canonical);
        }

        for audio_path in &audio_paths {
            fs::remove_file(audio_path).map_err(|_| SourceWriteError::Io)?;
        }
        fs::remove_file(&path).map_err(|_| SourceWriteError::Io)?;

        let mut deleted = audio_paths;
        deleted.push(path);
        Ok(deleted)
    }
}

/// Returns the raw front-matter header text (between the opening and closing
/// `---` delimiters), exactly as [`split_front_matter`] splits it for
/// [`parse_markdown`], or `None` if there is no front matter. Used to
/// preserve the header verbatim when overwriting a note's body.
fn front_matter_header(bytes: &[u8]) -> Option<String> {
    let input = std::str::from_utf8(bytes).ok()?;
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let normalized = input.replace("\r\n", "\n");
    let after = normalized.strip_prefix("---\n")?;
    let (header, _) = split_front_matter(after)?;
    Some(header.to_owned())
}

#[derive(Clone, Debug)]
pub struct AyumiSource {
    catalog: AyumiCatalog,
    bridge: AppleBridgeClient,
    cancelled: Arc<AtomicBool>,
}

impl AyumiSource {
    #[must_use]
    pub fn new(catalog: AyumiCatalog, bridge: AppleBridgeClient) -> Self {
        Self {
            catalog,
            bridge,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    #[must_use]
    pub fn cancellation_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn discover_request(&self, request: CatalogRequest) -> Result<Discovery, SourceReadError> {
        let mut probe = |source: &SourceRef| {
            self.bridge
                .availability(source, &self.cancelled)
                .map(|availability| match availability {
                    BridgeAvailability::Resident => SourceAvailability::Resident,
                    BridgeAvailability::CloudOnly => SourceAvailability::CloudOnly,
                    BridgeAvailability::PermissionDenied => SourceAvailability::PermissionDenied,
                    BridgeAvailability::Missing => SourceAvailability::Unknown,
                })
                .map_err(SourceReadError::Access)
        };
        match request {
            CatalogRequest::Automatic(day) => {
                let mut days = vec![shift_day(&day, -1), day.clone(), shift_day(&day, 1)];
                self.catalog.discover_days(&mut days, &mut probe)
            }
            CatalogRequest::ExplicitDate(day) => {
                self.catalog.discover_days(&mut vec![day], &mut probe)
            }
        }
    }

    fn read_bridge(&self, source: &SourceRef) -> Result<ReadOutcome, SourceAccessError> {
        match self.bridge.availability(source, &self.cancelled)? {
            BridgeAvailability::CloudOnly => return Ok(ReadOutcome::NeedsDownload),
            BridgeAvailability::PermissionDenied => return Ok(ReadOutcome::PermissionDenied),
            BridgeAvailability::Missing => return Ok(ReadOutcome::Unverifiable),
            BridgeAvailability::Resident => {}
        }
        let read = match self.bridge.coordinated_read(source, &self.cancelled) {
            Ok(read) => read,
            Err(SourceAccessError::PermissionDenied) => {
                return Ok(ReadOutcome::PermissionDenied);
            }
            Err(SourceAccessError::NeedsDownload) => return Ok(ReadOutcome::NeedsDownload),
            Err(SourceAccessError::NotFound) => return Ok(ReadOutcome::Unverifiable),
            Err(SourceAccessError::Oversized) => return Ok(ReadOutcome::Unreadable),
            Err(error) => return Err(error),
        };
        if read.bytes.len() as u64 != read.size || read.size > MAX_SOURCE_BYTES {
            return Ok(ReadOutcome::Changed);
        }
        let parsed = match parse_markdown(&read.bytes) {
            Some(value) if !value.ambiguous_attachment => value,
            Some(_) | None => return Ok(ReadOutcome::Unreadable),
        };
        let stamp = bridge_stamp(&read)?;
        selected_source(source, parsed, stamp)
            .map(Box::new)
            .map(ReadOutcome::Read)
            .map_err(|_| SourceAccessError::HelperProtocol)
    }
}

impl SourceCatalog for AyumiSource {
    fn discover(&self, request: CatalogRequest) -> Result<SourceDiscovery, SourceAccessError> {
        self.discover_request(request)
            .map(|discovery| SourceDiscovery {
                candidates: discovery.candidates,
                diagnostics: discovery.warnings,
            })
            .map_err(|error| match error {
                SourceReadError::InvalidDirectoryName => SourceAccessError::InvalidConfiguration,
                SourceReadError::Access(error) => error,
                SourceReadError::RootUnavailable | SourceReadError::UnknownSource => {
                    SourceAccessError::Unavailable
                }
            })
    }
}

impl SourceReader for AyumiSource {
    fn read_selected(&self, source: &SourceRef) -> Result<SourceReadOutcome, SourceAccessError> {
        Ok(match self.read_bridge(source)? {
            ReadOutcome::Read(source) => SourceReadOutcome::Read(*source),
            ReadOutcome::NeedsDownload => SourceReadOutcome::NeedsDownload {
                source: source.clone(),
            },
            ReadOutcome::PermissionDenied => SourceReadOutcome::PermissionDenied {
                source: source.clone(),
            },
            ReadOutcome::Unreadable => SourceReadOutcome::Unreadable {
                source: source.clone(),
            },
            ReadOutcome::Changed | ReadOutcome::Unverifiable => SourceReadOutcome::Unverifiable {
                source: source.clone(),
            },
        })
    }

    fn revalidate(&self, snapshot: &SourceSnapshot) -> Result<FreshnessOutcome, SourceAccessError> {
        Ok(match self.read_bridge(&snapshot.source)? {
            ReadOutcome::Read(current) if current.snapshot.stamp == snapshot.stamp => {
                FreshnessOutcome::Fresh
            }
            ReadOutcome::Read(_) | ReadOutcome::Changed => FreshnessOutcome::Changed {
                source: snapshot.source.clone(),
            },
            ReadOutcome::NeedsDownload => FreshnessOutcome::NeedsDownload {
                source: snapshot.source.clone(),
            },
            ReadOutcome::PermissionDenied => FreshnessOutcome::PermissionDenied {
                source: snapshot.source.clone(),
            },
            ReadOutcome::Unreadable => FreshnessOutcome::Unreadable {
                source: snapshot.source.clone(),
            },
            ReadOutcome::Unverifiable => FreshnessOutcome::Unverifiable {
                source: snapshot.source.clone(),
            },
        })
    }

    fn request_download(&self, source: &SourceRef) -> Result<(), SourceAccessError> {
        self.bridge.request_download(source, &self.cancelled)
    }
}

fn selected_source(
    source: &SourceRef,
    parsed: Parsed,
    stamp: FreshnessStamp,
) -> Result<SelectedSource, SourceReadError> {
    let directory_day = source_directory_day(source)?;
    let warning = merge_warnings(
        disagreement_warning(&directory_day, parsed.header_day.as_ref()),
        parsed.warning,
    );
    Ok(SelectedSource {
        snapshot: SourceSnapshot {
            source: source.clone(),
            stamp,
            body: parsed.body,
        },
        date_evidence: DateEvidence {
            directory_day: Some(directory_day),
            header_day: parsed.header_day,
            warning,
        },
    })
}

fn source_directory_day(source: &SourceRef) -> Result<DiaryDay, SourceReadError> {
    let parts: Vec<_> = source.source_id.split('/').collect();
    if parts.len() != 4 {
        return Err(SourceReadError::UnknownSource);
    }
    DiaryDay::parse(format!("{}-{}-{}", parts[0], parts[1], parts[2]))
        .map_err(|_| SourceReadError::UnknownSource)
}

fn bridge_stamp(read: &BridgeRead) -> Result<FreshnessStamp, SourceAccessError> {
    FreshnessStamp::new(
        read.file_identity.clone(),
        read.size,
        read.modified_at,
        Digest::of_bytes(&read.bytes),
    )
    .map_err(|_| SourceAccessError::HelperProtocol)
}

fn valid_source_path(relative: &Path) -> bool {
    let components: Vec<_> = relative.components().collect();
    let [
        Component::Normal(year),
        Component::Normal(month),
        Component::Normal(day),
        Component::Normal(file),
    ] = components.as_slice()
    else {
        return false;
    };
    let (Some(year), Some(month), Some(day), Some(file)) =
        (year.to_str(), month.to_str(), day.to_str(), file.to_str())
    else {
        return false;
    };
    DiaryDay::parse(format!("{year}-{month}-{day}")).is_ok()
        && !file.starts_with('.')
        && file.ends_with(".md")
}

fn read_bounded(path: &Path, expected: &fs::Metadata) -> std::io::Result<Option<Vec<u8>>> {
    let mut file = File::open(path)?;
    let opened = file.metadata()?;
    if metadata_identity(expected) != metadata_identity(&opened) {
        return Err(std::io::Error::other("source changed before read"));
    }
    if opened.len() > MAX_SOURCE_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let after_opened = file.metadata()?;
    let after_path = fs::symlink_metadata(path)?;
    if metadata_identity(&opened) != metadata_identity(&after_opened)
        || metadata_identity(&opened) != metadata_identity(&after_path)
    {
        return Err(std::io::Error::other("source changed during read"));
    }
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        Ok(None)
    } else {
        Ok(Some(bytes))
    }
}

struct Parsed {
    body: String,
    header_day: Option<DiaryDay>,
    warning: Option<String>,
    ambiguous_attachment: bool,
    audio_links: Vec<String>,
}

fn parse_markdown(bytes: &[u8]) -> Option<Parsed> {
    let input = std::str::from_utf8(bytes).ok()?;
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let normalized = input.replace("\r\n", "\n");
    if normalized.contains('\r') {
        return None;
    }
    let (header, body) = if let Some(after) = normalized.strip_prefix("---\n") {
        let Some((header, body)) = split_front_matter(after) else {
            return Some(Parsed {
                body: String::new(),
                header_day: None,
                warning: Some("front matter is unclosed".into()),
                ambiguous_attachment: true,
                audio_links: Vec::new(),
            });
        };
        (Some(header), body)
    } else {
        (None, normalized.as_str())
    };

    let mut created = None;
    let mut created_invalid = false;
    let mut malformed = false;
    if let Some(header) = header {
        for line in header.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, value)) = trimmed.split_once(':') else {
                malformed = true;
                continue;
            };
            if trimmed.len() == line.len() && key.trim() == "created_at" {
                if created.is_some() {
                    malformed = true;
                    created_invalid = true;
                } else {
                    created = Some(unquote(value.trim()));
                }
            }
        }
    }
    let header_day = (!created_invalid)
        .then(|| created.and_then(parse_offset_day))
        .flatten();
    let warning = if malformed {
        Some("front matter is malformed".into())
    } else if created.is_some() && header_day.is_none() {
        Some("created_at is invalid or lacks an offset".into())
    } else {
        None
    };
    let mut output = Vec::new();
    let mut ambiguous_attachment = false;
    let mut audio_links = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        match classify_attachment_line(trimmed) {
            AttachmentLine::StandaloneLocalAudio(destination) => {
                audio_links.push(destination);
                continue;
            }
            AttachmentLine::Ambiguous => {
                ambiguous_attachment = true;
                continue;
            }
            AttachmentLine::Safe => {}
        }
        output.push(line);
    }
    Some(Parsed {
        body: output.join("\n"),
        header_day,
        warning,
        ambiguous_attachment,
        audio_links,
    })
}

fn split_front_matter(after_open: &str) -> Option<(&str, &str)> {
    if let Some(end) = after_open.find("\n---\n") {
        return Some((&after_open[..end], &after_open[end + 5..]));
    }
    after_open.strip_suffix("\n---").map(|header| (header, ""))
}

fn unquote(value: &str) -> &str {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AttachmentLine {
    Safe,
    StandaloneLocalAudio(String),
    Ambiguous,
}

fn classify_attachment_line(line: &str) -> AttachmentLine {
    if line.to_ascii_lowercase().contains("attachment:") {
        return AttachmentLine::Ambiguous;
    }
    let mut offset = 0;
    let mut links = 0;
    let mut local_audio = None;
    while let Some(relative_open) = line[offset..].find("](") {
        let open = offset + relative_open;
        let tail = &line[open + 2..];
        let Some(relative_close) = matching_link_close(tail) else {
            return AttachmentLine::Ambiguous;
        };
        let close = open + 2 + relative_close;
        let raw_destination = tail[..relative_close].trim();
        let destination = link_destination(raw_destination);
        links += 1;
        if is_local_audio_destination(destination) {
            local_audio = Some((open, close, destination));
        }
        offset = close + 1;
    }
    if local_audio.is_none() {
        if let Some((_, destination)) = line.split_once("]:")
            && line.starts_with('[')
            && is_local_audio_destination(link_destination(destination))
        {
            return AttachmentLine::Ambiguous;
        }
        return AttachmentLine::Safe;
    }
    let (open, close, destination) = local_audio.expect("checked above");
    if links == 1
        && line.starts_with('[')
        && !line.starts_with("![")
        && open > 0
        && close + 1 == line.len()
        && !destination.is_empty()
        && destination_is_path_safe(destination)
    {
        return AttachmentLine::StandaloneLocalAudio(destination.to_owned());
    }
    AttachmentLine::Ambiguous
}

/// Returns whether a parsed link destination is safe to treat as a
/// locally-resolvable relative path: no leading `/` (absolute) and no `..`
/// component that could escape the source's directory.
fn destination_is_path_safe(destination: &str) -> bool {
    !destination.starts_with('/')
        && destination
            .split('/')
            .all(|component| !component.is_empty() && component != "..")
}

fn matching_link_close(value: &str) -> Option<usize> {
    let mut depth = 0_u32;
    for (index, byte) in value.bytes().enumerate() {
        match byte {
            b'(' => depth = depth.saturating_add(1),
            b')' if depth == 0 => return Some(index),
            b')' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn link_destination(value: &str) -> &str {
    let value = value.trim();
    if let Some(value) = value.strip_prefix('<') {
        return value.split_once('>').map_or(value, |(path, _)| path);
    }
    value.split_ascii_whitespace().next().unwrap_or("")
}

fn is_local_audio_destination(destination: &str) -> bool {
    let normalized = destination
        .split(['?', '#'])
        .next()
        .unwrap_or(destination)
        .to_ascii_lowercase();
    let is_audio = [".mp3", ".m4a", ".wav", ".aac", ".flac"]
        .iter()
        .any(|extension| normalized.ends_with(extension));
    let remote = destination.contains("://") || destination.starts_with("mailto:");
    is_audio && !remote
}

fn parse_offset_day(value: &str) -> Option<DiaryDay> {
    let (date, time_and_offset) = value.split_once('T')?;
    let day = DiaryDay::parse(date).ok()?;
    let (time, offset) = if let Some(time) = time_and_offset.strip_suffix('Z') {
        (time, "Z")
    } else {
        let offset_index = time_and_offset.rfind(['+', '-'])?;
        (
            &time_and_offset[..offset_index],
            &time_and_offset[offset_index..],
        )
    };
    let base_time = time.split_once('.').map_or(time, |(base, fraction)| {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            ""
        } else {
            base
        }
    });
    let mut parts = base_time.split(':');
    let hour = parse_two(parts.next()?)?;
    let minute = parse_two(parts.next()?)?;
    let second = parse_two(parts.next()?)?;
    if parts.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    if offset != "Z" {
        let bytes = offset.as_bytes();
        if bytes.len() != 6 || bytes[3] != b':' {
            return None;
        }
        let offset_hour = parse_two(&offset[1..3])?;
        let offset_minute = parse_two(&offset[4..6])?;
        if offset_hour > 23 || offset_minute > 59 {
            return None;
        }
    }
    Some(day)
}

fn parse_two(value: &str) -> Option<u8> {
    (value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())?
}

fn disagreement_warning(directory: &DiaryDay, header: Option<&DiaryDay>) -> Option<String> {
    header
        .filter(|day| *day != directory)
        .map(|_| "directory and created_at dates disagree".into())
}

fn merge_warnings(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(format!("{left}; {right}")),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn stamp(metadata: &fs::Metadata, bytes: &[u8]) -> Result<FreshnessStamp, SourceReadError> {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs() as i64);
    FreshnessStamp::new(
        format!("{}:{}", metadata.dev(), metadata.ino()),
        metadata.len(),
        Timestamp(modified),
        Digest::of_bytes(bytes),
    )
    .map_err(|_| SourceReadError::RootUnavailable)
}

fn metadata_identity(metadata: &fs::Metadata) -> (u64, u64, u64, i64, i64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
    )
}
fn shift_day(day: &DiaryDay, offset: i32) -> DiaryDay {
    let mut year: i32 = day.as_str()[0..4].parse().expect("validated");
    let mut month: i32 = day.as_str()[5..7].parse().expect("validated");
    let mut date: i32 = day.as_str()[8..10].parse::<i32>().expect("validated") + offset;
    while date < 1 {
        month -= 1;
        if month == 0 {
            year -= 1;
            month = 12;
        }
        date += days_in_month(year, month);
    }
    while date > days_in_month(year, month) {
        date -= days_in_month(year, month);
        month += 1;
        if month == 13 {
            year += 1;
            month = 1;
        }
    }
    DiaryDay::parse(format!("{year:04}-{month:02}-{date:02}")).expect("calendar arithmetic")
}
fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        _ => 28,
    }
}

#[cfg(test)]
mod tests {
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
        let ReadOutcome::Read(snapshot) =
            catalog.read_selected(&found.candidates[0].source).unwrap()
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

        let invalid =
            parse_markdown(b"---\ncreated_at: 2026-10-28T25:00:00+09:00\n---\nbody").unwrap();
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
            parse_markdown(b"---\nmetadata:\n  created_at: 2026-10-28T00:00:00Z\n---\nbody")
                .unwrap();
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

        let absolute =
            parse_markdown(b"prose\n[audio](/Users/x/recording.m4a)\nmore prose").unwrap();
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
input=$(cat)
id=$(printf '%s' "$input" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
if printf '%s' "$input" | grep -q 'icloud-availability'; then
  if printf '%s' "$input" | grep -q 'cloud.md'; then status=cloud-only; else status=resident; fi
  printf '{"ok":true,"protocol_version":1,"request_id":"%s","value":{"status":"%s"}}\n' "$id" "$status"
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
  printf '{"ok":true,"protocol_version":1,"request_id":"%s","value":{"file_identity":"4:5","handoff_name":"nd-source-selected.bin","modified_at":123,"size":%s}}\n' "$id" "$size"
else
  printf '{"ok":true,"protocol_version":1,"request_id":"%s","value":{"status":"resident"}}\n' "$id"
fi
"#,
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        let bridge = AppleBridgeClient::new(
            &helper,
            "Ayumi",
            root.join("handoff"),
            Duration::from_secs(2),
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
            diary_core::revalidate_workflow_sources(
                &mut workflow,
                &source,
                checkpoint,
                Timestamp(4),
            )
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
    fn delete_source_rejects_traversal_audio_link() {
        let root = temp();
        let outside_target = root.join("outside-passwd.m4a");
        fs::write(&outside_target, b"must not be deleted").unwrap();
        let note_path = root.join("Ayumi/2026/10/28/n.md");
        fs::write(
            &note_path,
            "prose\n[audio](../../../outside-passwd.m4a)\nmore prose",
        )
        .unwrap();
        let catalog = AyumiCatalog::new(&root, "Ayumi").unwrap();
        let source = SourceRef::new(CONNECTOR, "2026/10/28/n.md").unwrap();

        // Confirm the upstream parser already withholds this destination, so
        // it is a defense-in-depth check: either the parser kept it out of
        // `audio_links` entirely (nothing unsafe is ever attempted) or, were
        // it present, our own containment check would reject it. Either way
        // nothing outside the fixture is touched and the outside file survives.
        let result = catalog.delete_source(&source);
        match result {
            Ok(deleted) => {
                assert!(deleted.iter().all(|p| p != &outside_target));
            }
            Err(error) => assert_eq!(error, SourceWriteError::UnsafePath),
        }
        assert!(outside_target.exists(), "traversal target must survive");
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
}
