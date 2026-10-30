//! Exact local Markdown rendering and no-clobber publication.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use diary_core::{DiaryDay, Digest, LocalOutputPort, WritingLanguage, render_local_markdown};
pub use diary_core::{
    LocalOutputError as LocalError, LocalOutputState as LocalState, LocalReplaceOutcome,
};

#[derive(Clone, Debug)]
pub struct LocalOutput {
    root: PathBuf,
    /// Canonical `~/Library/Mobile Documents` for an iCloud mirror root,
    /// re-checked before every inspect and write.
    icloud_base: Option<PathBuf>,
}

impl LocalOutput {
    pub fn new(root: PathBuf) -> Result<Self, LocalError> {
        if !root.is_absolute()
            || root
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(LocalError::UnsafePath);
        }
        Ok(Self {
            root,
            icloud_base: None,
        })
    }

    /// An output whose root must stay under `home/Library/Mobile Documents`.
    /// The root itself may not exist yet, but it must exist when used.
    pub fn new_icloud(root: PathBuf, home: &Path) -> Result<Self, LocalError> {
        let mut output = Self::new(root)?;
        let base = fs::canonicalize(home.join("Library").join("Mobile Documents"))
            .map_err(|_| LocalError::UnsafePath)?;
        output.icloud_base = Some(base);
        output.check_icloud_root()?;
        Ok(output)
    }

    fn check_icloud_root(&self) -> Result<(), LocalError> {
        let Some(base) = &self.icloud_base else {
            return Ok(());
        };
        let existing = self
            .root
            .ancestors()
            .find(|path| fs::symlink_metadata(path).is_ok())
            .ok_or(LocalError::UnsafePath)?;
        let canonical = fs::canonicalize(existing).map_err(|_| LocalError::Unavailable)?;
        if !canonical.starts_with(base) {
            return Err(LocalError::UnsafePath);
        }
        Ok(())
    }

    fn check_mirror_ready(&self) -> Result<(), LocalError> {
        if self.icloud_base.is_none() {
            return Ok(());
        }
        self.check_icloud_root()?;
        match fs::metadata(&self.root) {
            Ok(metadata) if metadata.is_dir() => Ok(()),
            _ => Err(LocalError::Unavailable),
        }
    }

    fn has_icloud_placeholder(target: &Path) -> bool {
        let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
            return false;
        };
        let mut placeholder = std::ffi::OsString::from(".");
        placeholder.push(name);
        placeholder.push(".icloud");
        fs::symlink_metadata(parent.join(placeholder)).is_ok()
    }

    pub fn target(&self, day: &DiaryDay) -> PathBuf {
        self.root
            .join(&day.as_str()[..4])
            .join(&day.as_str()[5..7])
            .join(format!("{}.md", day.as_str()))
    }

    pub fn render(day: &DiaryDay, language: WritingLanguage, body: &str) -> String {
        render_local_markdown(day, language, body)
    }

    pub fn inspect(&self, day: &DiaryDay, payload: &str) -> Result<LocalState, LocalError> {
        let target = self.target(day);
        self.check_mirror_ready()?;
        self.validate_ancestors(&target)?;
        if Self::has_icloud_placeholder(&target) {
            return Err(LocalError::Unavailable);
        }
        match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(LocalState::Absent),
            Err(_) => Err(LocalError::Unavailable),
            Ok(metadata) if !metadata.file_type().is_file() => Err(LocalError::UnsafePath),
            Ok(metadata) if metadata.len() > 2 * 1024 * 1024 => Err(LocalError::Unavailable),
            Ok(metadata) => {
                let mut bytes = Vec::new();
                let file = File::open(&target).map_err(|_| LocalError::Unavailable)?;
                let opened = file.metadata().map_err(|_| LocalError::Unavailable)?;
                if opened.dev() != metadata.dev()
                    || opened.ino() != metadata.ino()
                    || !opened.file_type().is_file()
                {
                    return Err(LocalError::UnsafePath);
                }
                file.take(2 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| LocalError::Unavailable)?;
                if bytes.len() > 2 * 1024 * 1024 {
                    return Err(LocalError::Unavailable);
                }
                if bytes == payload.as_bytes() {
                    Ok(LocalState::Identical)
                } else {
                    Ok(LocalState::Different(
                        String::from_utf8(bytes).map_err(|_| LocalError::Unavailable)?,
                    ))
                }
            }
        }
    }

    pub fn write(&self, day: &DiaryDay, payload: &str) -> Result<LocalState, LocalError> {
        match self.inspect(day, payload)? {
            LocalState::Absent => {}
            other => return Ok(other),
        }
        let target = self.target(day);
        let parent = target.parent().ok_or(LocalError::UnsafePath)?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|_| LocalError::Unavailable)?;
        self.validate_ancestors(&target)?;
        self.check_mirror_ready()?;
        let mut temp = None;
        for attempt in 0..32 {
            let name = format!(".nd-{}-{}-{attempt}.tmp", std::process::id(), day);
            let path = parent.join(name);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(file) => {
                    temp = Some((path, file));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(LocalError::Unavailable),
            }
        }
        let (temp_path, mut file) = temp.ok_or(LocalError::Unavailable)?;
        let outcome = (|| {
            file.write_all(payload.as_bytes())
                .map_err(|_| LocalError::Unavailable)?;
            file.sync_all().map_err(|_| LocalError::Unavailable)?;
            // A hard link atomically creates the final name only if absent.
            // It never replaces another file created after the preflight.
            match fs::hard_link(&temp_path, &target) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return Ok(());
                }
                Err(_) => return Err(LocalError::Unavailable),
            }
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| LocalError::Unavailable)?;
            Ok(())
        })();
        drop(file);
        let cleaned = fs::remove_file(temp_path);
        outcome?;
        cleaned.map_err(|_| LocalError::Unavailable)?;
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| LocalError::Unavailable)?;
        self.inspect(day, payload)
    }

    /// Atomically replaces the day's file while its bytes still hash to
    /// `expected`. See [`Self::replace_with_hook`].
    pub fn replace(
        &self,
        day: &DiaryDay,
        expected: &Digest,
        payload: &str,
    ) -> Result<LocalReplaceOutcome, LocalError> {
        self.replace_with_hook(day, expected, payload, || {})
    }

    /// `before_rename` runs after the temp file is durable and before the
    /// final digest recheck; tests use it to model a racing editor.
    fn replace_with_hook(
        &self,
        day: &DiaryDay,
        expected: &Digest,
        payload: &str,
        before_rename: impl FnOnce(),
    ) -> Result<LocalReplaceOutcome, LocalError> {
        let target = self.target(day);
        match self.current_digest(day)? {
            None => return Ok(LocalReplaceOutcome::Changed),
            Some(current) if current == Self::payload_digest(payload) => {
                return Ok(LocalReplaceOutcome::AlreadyReplaced);
            }
            Some(current) if &current != expected => return Ok(LocalReplaceOutcome::Changed),
            Some(_) => {}
        }
        let parent = target.parent().ok_or(LocalError::UnsafePath)?;
        Self::sweep_orphan_temps(parent, day);
        let mut temp = None;
        for attempt in 0..32 {
            let path = parent.join(format!(".nd-{}-{}-{attempt}.tmp", std::process::id(), day));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(file) => {
                    temp = Some((path, file));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(LocalError::Unavailable),
            }
        }
        let (temp_path, mut file) = temp.ok_or(LocalError::Unavailable)?;
        let outcome = (|| {
            file.write_all(payload.as_bytes())
                .map_err(|_| LocalError::Unavailable)?;
            file.sync_all().map_err(|_| LocalError::Unavailable)?;
            before_rename();
            // The immediate pre-rename guard: any change since the seed wins.
            if self.current_digest(day)?.as_ref() != Some(expected) {
                return Ok(LocalReplaceOutcome::Changed);
            }
            fs::rename(&temp_path, &target).map_err(|_| LocalError::Unavailable)?;
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| LocalError::Unavailable)?;
            Ok(LocalReplaceOutcome::Replaced)
        })();
        drop(file);
        if outcome != Ok(LocalReplaceOutcome::Replaced) {
            let _ = fs::remove_file(&temp_path);
        }
        outcome
    }

    /// Removes temp files a crashed earlier process left for this day. Only
    /// the exact `.nd-<pid>-<day>-<n>.tmp` pattern of another process is
    /// matched; this process's own temp files and all other names are kept.
    fn sweep_orphan_temps(parent: &Path, day: &DiaryDay) {
        let Ok(entries) = fs::read_dir(parent) else {
            return;
        };
        let own_pid = std::process::id().to_string();
        let day_marker = format!("-{day}-");
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(rest) = name
                .to_str()
                .and_then(|name| name.strip_prefix(".nd-"))
                .and_then(|name| name.strip_suffix(".tmp"))
            else {
                continue;
            };
            let Some((pid, attempt)) = rest.split_once(&day_marker) else {
                continue;
            };
            let numeric = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
            if numeric(pid) && numeric(attempt) && pid != own_pid {
                let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
                if is_file {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    fn current_digest(&self, day: &DiaryDay) -> Result<Option<Digest>, LocalError> {
        Ok(match self.inspect(day, "")? {
            LocalState::Absent => None,
            LocalState::Identical => Some(Self::payload_digest("")),
            LocalState::Different(text) => Some(Self::payload_digest(&text)),
        })
    }

    fn validate_ancestors(&self, target: &Path) -> Result<(), LocalError> {
        for ancestor in target
            .ancestors()
            .skip(1)
            .take_while(|path| *path != Path::new("/"))
        {
            match fs::symlink_metadata(ancestor) {
                Ok(metadata)
                    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => return Err(LocalError::UnsafePath),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(_) => return Err(LocalError::Unavailable),
            }
        }
        Ok(())
    }

    pub fn payload_digest(payload: &str) -> Digest {
        Digest::of_bytes(payload.as_bytes())
    }
}

impl LocalOutputPort for LocalOutput {
    fn target(&self, day: &DiaryDay) -> String {
        self.target(day).to_string_lossy().into_owned()
    }
    fn inspect(&self, day: &DiaryDay, payload: &str) -> Result<LocalState, LocalError> {
        self.inspect(day, payload)
    }
    fn write(&self, day: &DiaryDay, payload: &str) -> Result<LocalState, LocalError> {
        self.write(day, payload)
    }
    fn replace(
        &self,
        day: &DiaryDay,
        expected: &Digest,
        payload: &str,
    ) -> Result<LocalReplaceOutcome, LocalError> {
        self.replace(day, expected, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("nd-local-output-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        fs::canonicalize(path).unwrap()
    }

    #[test]
    fn exact_render_and_no_clobber_resume() {
        let root = scratch("write");
        let day = DiaryDay::parse("2026-10-30").unwrap();
        let output = LocalOutput::new(root.join("entries")).unwrap();
        let payload = LocalOutput::render(&day, WritingLanguage::KoKr, "A day\n");
        assert_eq!(
            payload,
            "---\ndate: \"2026-10-30\"\nlanguage: \"ko-KR\"\n---\n\nA day\n"
        );
        assert_eq!(output.inspect(&day, &payload), Ok(LocalState::Absent));
        assert_eq!(output.write(&day, &payload), Ok(LocalState::Identical));
        assert_eq!(output.write(&day, &payload), Ok(LocalState::Identical));
        assert_eq!(fs::read_to_string(output.target(&day)).unwrap(), payload);
        assert!(matches!(
            output.write(&day, "different"),
            Ok(LocalState::Different(_))
        ));
        assert_eq!(fs::read_to_string(output.target(&day)).unwrap(), payload);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replace_swaps_atomically_only_when_the_seed_digest_still_matches() {
        let root = scratch("replace");
        let day = DiaryDay::parse("2026-10-30").unwrap();
        let output = LocalOutput::new(root.join("entries")).unwrap();
        let old = LocalOutput::render(&day, WritingLanguage::EnUs, "Old day\n");
        let new = LocalOutput::render(&day, WritingLanguage::EnUs, "New day\n");
        let old_digest = LocalOutput::payload_digest(&old);
        output.write(&day, &old).unwrap();

        assert_eq!(
            output.replace(&day, &old_digest, &new),
            Ok(LocalReplaceOutcome::Replaced)
        );
        assert_eq!(fs::read_to_string(output.target(&day)).unwrap(), new);
        // Replaying after success is a no-op, not a second replacement.
        assert_eq!(
            output.replace(&day, &old_digest, &new),
            Ok(LocalReplaceOutcome::AlreadyReplaced)
        );
        let leftovers: Vec<_> = fs::read_dir(output.target(&day).parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "no temp file may remain: {leftovers:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replace_refuses_a_hand_edit_and_a_race_before_the_rename() {
        let root = scratch("replace-race");
        let day = DiaryDay::parse("2026-10-30").unwrap();
        let output = LocalOutput::new(root.join("entries")).unwrap();
        let old = LocalOutput::render(&day, WritingLanguage::EnUs, "Old day\n");
        let new = LocalOutput::render(&day, WritingLanguage::EnUs, "New day\n");
        let old_digest = LocalOutput::payload_digest(&old);
        output.write(&day, &old).unwrap();

        // Edited before the call.
        fs::write(output.target(&day), "hand edit").unwrap();
        assert_eq!(
            output.replace(&day, &old_digest, &new),
            Ok(LocalReplaceOutcome::Changed)
        );
        assert_eq!(
            fs::read_to_string(output.target(&day)).unwrap(),
            "hand edit"
        );

        // Edited after the first check but before the rename.
        fs::write(output.target(&day), &old).unwrap();
        let target = output.target(&day);
        let outcome = output.replace_with_hook(&day, &old_digest, &new, || {
            fs::write(&target, "raced edit").unwrap();
        });
        assert_eq!(outcome, Ok(LocalReplaceOutcome::Changed));
        assert_eq!(fs::read_to_string(&target).unwrap(), "raced edit");
        let names: Vec<_> = fs::read_dir(target.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "temp file must be cleaned up: {names:?}");

        // A missing file is not a replacement target.
        fs::remove_file(&target).unwrap();
        assert_eq!(
            output.replace(&day, &old_digest, &new),
            Ok(LocalReplaceOutcome::Changed)
        );
        assert!(!target.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_symlink_target_and_parent() {
        let root = scratch("symlink");
        let outside = root.join("outside");
        fs::write(&outside, "untouched").unwrap();
        let day = DiaryDay::parse("2026-10-30").unwrap();
        let output = LocalOutput::new(root.join("entries")).unwrap();
        fs::create_dir_all(output.target(&day).parent().unwrap()).unwrap();
        symlink(&outside, output.target(&day)).unwrap();
        assert_eq!(output.write(&day, "entry"), Err(LocalError::UnsafePath));
        assert_eq!(fs::read_to_string(&outside).unwrap(), "untouched");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn concurrent_writers_never_replace_the_winner() {
        let root = scratch("race");
        let output = LocalOutput::new(root.join("entries")).unwrap();
        let day = DiaryDay::parse("2026-10-30").unwrap();
        let left = output.clone();
        let right = output.clone();
        let left_day = day.clone();
        let right_day = day.clone();
        let first = std::thread::spawn(move || left.write(&left_day, "first"));
        let second = std::thread::spawn(move || right.write(&right_day, "second"));
        let left_result = first.join().unwrap().unwrap();
        let right_result = second.join().unwrap().unwrap();
        let final_text = fs::read_to_string(output.target(&day)).unwrap();
        assert!(matches!(final_text.as_str(), "first" | "second"));
        assert_eq!(
            matches!(left_result, LocalState::Identical),
            final_text == "first"
        );
        assert_eq!(
            matches!(right_result, LocalState::Identical),
            final_text == "second"
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn icloud_home(name: &str) -> (PathBuf, PathBuf) {
        let home = scratch(name);
        let root = home
            .join("Library")
            .join("Mobile Documents")
            .join("com~apple~CloudDocs")
            .join("entries");
        fs::create_dir_all(&root).unwrap();
        (home, root)
    }

    #[test]
    fn new_icloud_accepts_root_under_mobile_documents() {
        let (home, root) = icloud_home("icloud-ok");
        let day = DiaryDay::parse("2026-10-30").unwrap();
        let output = LocalOutput::new_icloud(root, &home).unwrap();
        assert_eq!(output.write(&day, "entry"), Ok(LocalState::Identical));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn new_icloud_rejects_root_outside_mobile_documents() {
        let (home, _) = icloud_home("icloud-outside");
        let outside = home.join("Documents").join("entries");
        fs::create_dir_all(&outside).unwrap();
        assert!(matches!(
            LocalOutput::new_icloud(outside, &home),
            Err(LocalError::UnsafePath)
        ));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn new_icloud_rejects_parent_dir_component() {
        let (home, root) = icloud_home("icloud-dotdot");
        let sneaky = root
            .join("..")
            .join("..")
            .join("..")
            .join("..")
            .join("Documents");
        assert!(matches!(
            LocalOutput::new_icloud(sneaky, &home),
            Err(LocalError::UnsafePath)
        ));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn new_icloud_rejects_symlink_escaping_mobile_documents() {
        let (home, root) = icloud_home("icloud-symlink");
        let elsewhere = home.join("Elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        let link = root.join("link");
        symlink(&elsewhere, &link).unwrap();
        assert!(matches!(
            LocalOutput::new_icloud(link.join("entries"), &home),
            Err(LocalError::UnsafePath)
        ));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn icloud_root_that_vanishes_is_unavailable_and_not_recreated() {
        let (home, root) = icloud_home("icloud-missing");
        let day = DiaryDay::parse("2026-10-30").unwrap();
        let output = LocalOutput::new_icloud(root.clone(), &home).unwrap();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(output.inspect(&day, "entry"), Err(LocalError::Unavailable));
        assert_eq!(output.write(&day, "entry"), Err(LocalError::Unavailable));
        assert!(!root.exists());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn icloud_placeholder_makes_inspect_and_write_unavailable() {
        let (home, root) = icloud_home("icloud-placeholder");
        let day = DiaryDay::parse("2026-10-30").unwrap();
        let output = LocalOutput::new_icloud(root, &home).unwrap();
        let target = output.target(&day);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target.parent().unwrap().join(".2026-10-30.md.icloud"), "").unwrap();
        assert_eq!(output.inspect(&day, "entry"), Err(LocalError::Unavailable));
        assert_eq!(output.write(&day, "entry"), Err(LocalError::Unavailable));
        assert!(!target.exists());
        fs::remove_dir_all(home).unwrap();
    }
}
