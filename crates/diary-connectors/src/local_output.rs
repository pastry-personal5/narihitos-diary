//! Exact local Markdown rendering and no-clobber publication.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use diary_core::{DiaryDay, Digest, LocalOutputPort, WritingLanguage, render_local_markdown};
pub use diary_core::{LocalOutputError as LocalError, LocalOutputState as LocalState};

#[derive(Clone, Debug)]
pub struct LocalOutput {
    root: PathBuf,
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
        Ok(Self { root })
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
        self.validate_ancestors(&target)?;
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
}
