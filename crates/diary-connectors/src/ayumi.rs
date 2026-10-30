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
    /// The file's current content no longer matches what the caller last
    /// saw: either the readable body's digest no longer matches
    /// `expected_content_digest`, or the file now contains content the
    /// parser could not safely read (an ambiguous attachment line, or
    /// unclosed front matter). Nothing was written.
    Stale,
    /// A filesystem operation failed. Carries the paths, if any, that were
    /// already deleted before the failure (always empty for
    /// `overwrite_body`, which deletes nothing).
    Io(Vec<PathBuf>),
    /// A path (the source itself or a linked audio file) failed the same
    /// symlink/traversal/containment checks used for reads, or an audio
    /// link destination contained a `?`/`#` that could let it resolve to a
    /// different literal file than the one the audio-extension check saw;
    /// nothing was written or deleted.
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
    /// text (including `created_at`) — after the same BOM-strip and
    /// CRLF-to-LF normalization [`parse_markdown`] applies, so this is not a
    /// byte-for-byte reproduction of a source file that used a BOM or CRLF
    /// line endings, only of the header text itself. Any audio links the
    /// parser withheld from the readable body are re-appended after
    /// `new_body` (each as its own `[audio](<destination>)` line, using the
    /// bracket form only when the destination contains whitespace, since a
    /// bare destination is cut at the first space on re-parse) so a later
    /// re-parse, and therefore [`Self::delete_source`], still finds them.
    /// Rejects the write with `SourceWriteError::Stale`, without modifying
    /// anything, if the file's current content matches neither the
    /// caller-supplied `expected_content_digest` as a readable-body digest
    /// nor as a whole-raw-file digest (a caller may reasonably hold either:
    /// a body digest they computed themselves over a `SourceSnapshot.body`,
    /// or the raw `content_digest` this method — or `read_selected` —
    /// previously returned), or if the file now contains any content the
    /// parser could not safely read (an ambiguous attachment line, or
    /// unclosed front matter) that neither digest could have accounted for.
    pub fn overwrite_body(
        &self,
        source: &SourceRef,
        new_body: &str,
        expected_content_digest: &Digest,
    ) -> Result<FreshnessStamp, SourceWriteError> {
        let path = self
            .resolve(source)
            .map_err(|_| SourceWriteError::UnsafePath)?;
        let metadata = fs::symlink_metadata(&path).map_err(|_| SourceWriteError::Io(Vec::new()))?;
        if metadata.len() > MAX_SOURCE_BYTES {
            return Err(SourceWriteError::Io(Vec::new()));
        }
        let bytes = fs::read(&path).map_err(|_| SourceWriteError::Io(Vec::new()))?;
        let parsed = parse_markdown(&bytes).ok_or(SourceWriteError::Io(Vec::new()))?;
        if parsed.ambiguous_attachment {
            return Err(SourceWriteError::Stale);
        }
        let body_digest_matches =
            Digest::of_bytes(parsed.body.as_bytes()) == *expected_content_digest;
        let raw_digest_matches = Digest::of_bytes(&bytes) == *expected_content_digest;
        if !body_digest_matches && !raw_digest_matches {
            return Err(SourceWriteError::Stale);
        }

        // Trim any trailing newlines from the caller's body before adding
        // the separator: without this, a body that itself already ends in
        // a blank line (for example, one previously round-tripped through
        // this same re-attachment) would accumulate an extra blank line on
        // every edit instead of keeping exactly one separator.
        let mut new_body_section = new_body.trim_end_matches('\n').to_owned();
        if !parsed.audio_links.is_empty() {
            new_body_section.push_str("\n\n");
            for (index, link) in parsed.audio_links.iter().enumerate() {
                if index > 0 {
                    new_body_section.push('\n');
                }
                new_body_section.push_str("[audio](");
                // A bracket-sourced destination can never contain '>', and a
                // bare one can never contain whitespace (Task 2's parser
                // only accepted one or the other on the way in), so this
                // round-trips both shapes without ambiguity:
                // `link_destination` only preserves a destination's interior
                // whitespace when it is wrapped in `<...>`.
                if link.contains(|character: char| character.is_ascii_whitespace()) {
                    new_body_section.push('<');
                    new_body_section.push_str(link);
                    new_body_section.push('>');
                } else {
                    new_body_section.push_str(link);
                }
                new_body_section.push(')');
            }
        }

        let header = front_matter_header(&bytes);
        let new_contents = match header {
            Some(header) => format!("---\n{header}\n---\n{new_body_section}"),
            None => new_body_section,
        };

        // Re-check immediately before writing: the path must still be a
        // non-symlink regular file inside the catalog base. This guards
        // against the file being swapped for a symlink between the read
        // above and this write.
        let metadata = fs::symlink_metadata(&path).map_err(|_| SourceWriteError::Io(Vec::new()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(SourceWriteError::UnsafePath);
        }

        fs::write(&path, new_contents.as_bytes()).map_err(|_| SourceWriteError::Io(Vec::new()))?;

        // Derive the returned stamp from an actual re-read, not from the
        // in-memory `new_contents` we believe we just wrote: the returned
        // value must truthfully describe whatever is really on disk now.
        let written_metadata =
            fs::symlink_metadata(&path).map_err(|_| SourceWriteError::Io(Vec::new()))?;
        if written_metadata.file_type().is_symlink() || !written_metadata.is_file() {
            return Err(SourceWriteError::UnsafePath);
        }
        let written_bytes = fs::read(&path).map_err(|_| SourceWriteError::Io(Vec::new()))?;
        stamp(&written_metadata, &written_bytes).map_err(|_| SourceWriteError::Io(Vec::new()))
    }

    /// Deletes `source` and every locally-resolvable audio file linked from
    /// its body. Each linked audio destination is resolved against the
    /// note's own parent directory and must pass the same
    /// symlink/traversal/containment checks as [`Self::resolve`]; a
    /// destination containing a literal `?` or `#` is rejected outright
    /// (genuine Ayumi audio links never contain one, and such a character
    /// would let the literal file deleted differ from the file the
    /// audio-extension check inspected). All audio links are validated
    /// before anything is deleted: if any one of them is unsafe, nothing is
    /// deleted and `SourceWriteError::UnsafePath` is returned. On success,
    /// returns the paths actually deleted, audio files first and the note
    /// file last. On an I/O failure partway through deleting, returns
    /// `SourceWriteError::Io` carrying the paths already deleted.
    pub fn delete_source(&self, source: &SourceRef) -> Result<Vec<PathBuf>, SourceWriteError> {
        let path = self
            .resolve(source)
            .map_err(|_| SourceWriteError::UnsafePath)?;
        let parent = path.parent().ok_or(SourceWriteError::UnsafePath)?;
        let canonical_parent =
            fs::canonicalize(parent).map_err(|_| SourceWriteError::Io(Vec::new()))?;

        let metadata = fs::symlink_metadata(&path).map_err(|_| SourceWriteError::Io(Vec::new()))?;
        if metadata.len() > MAX_SOURCE_BYTES {
            return Err(SourceWriteError::Io(Vec::new()));
        }
        let bytes = fs::read(&path).map_err(|_| SourceWriteError::Io(Vec::new()))?;
        let parsed = parse_markdown(&bytes).ok_or(SourceWriteError::Io(Vec::new()))?;

        let mut audio_paths: Vec<PathBuf> = Vec::with_capacity(parsed.audio_links.len());
        let mut seen = std::collections::HashSet::new();
        for link in &parsed.audio_links {
            if link.contains('?') || link.contains('#') {
                return Err(SourceWriteError::UnsafePath);
            }
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
            if seen.insert(canonical.clone()) {
                audio_paths.push(canonical);
            }
        }

        let mut deleted = Vec::with_capacity(audio_paths.len() + 1);
        for audio_path in &audio_paths {
            fs::remove_file(audio_path).map_err(|_| SourceWriteError::Io(deleted.clone()))?;
            deleted.push(audio_path.clone());
        }
        fs::remove_file(&path).map_err(|_| SourceWriteError::Io(deleted.clone()))?;
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
            // The requested day is Mac-local while directories are UTC, so
            // scan the neighbours; the caller keeps notes on the local day.
            CatalogRequest::ExplicitDate(day) => {
                let mut days = vec![shift_day(&day, -1), day.clone(), shift_day(&day, 1)];
                self.catalog.discover_days(&mut days, &mut probe)
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

impl diary_core::SourceWriter for AyumiSource {
    fn overwrite_body(
        &self,
        source: &SourceRef,
        new_body: &str,
        expected_digest: &Digest,
    ) -> Result<FreshnessStamp, diary_core::SourceWriteError> {
        self.catalog
            .overwrite_body(source, new_body, expected_digest)
            .map_err(map_write_error)
    }

    fn delete_source(&self, source: &SourceRef) -> Result<(), diary_core::SourceWriteError> {
        self.catalog
            .delete_source(source)
            .map(|_| ())
            .map_err(map_write_error)
    }
}

fn map_write_error(error: SourceWriteError) -> diary_core::SourceWriteError {
    match error {
        SourceWriteError::Stale => diary_core::SourceWriteError::Stale,
        SourceWriteError::UnsafePath => diary_core::SourceWriteError::UnsafePath,
        SourceWriteError::Io(deleted) if deleted.is_empty() => {
            diary_core::SourceWriteError::Unavailable
        }
        SourceWriteError::Io(_) => diary_core::SourceWriteError::PartialFailure,
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
mod tests;
