//! Destination safety: the mandatory gate before any recovery job runs.
//!
//! A source is evidence and must never be mutated. Writing recovered files
//! onto the source would overwrite the very data being recovered, so this
//! check blocks the job rather than warning about it.

use std::path::{Path, PathBuf};

use recovery_core::RecoveryError;

/// Why a destination was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DestinationRejection {
    /// The destination is the source image file itself.
    SameAsSourceFile,
    /// The destination lies inside the source, or the source inside it.
    OverlapsSource,
    /// The destination path does not exist and could not be created.
    Unusable(String),
    /// The destination exists but is not a directory.
    NotADirectory,
}

impl DestinationRejection {
    fn message(&self) -> String {
        match self {
            Self::SameAsSourceFile => "destination is the source itself".into(),
            Self::OverlapsSource => "destination overlaps the source".into(),
            Self::NotADirectory => "destination is not a directory".into(),
            Self::Unusable(detail) => format!("destination is unusable: {detail}"),
        }
    }
}

impl From<DestinationRejection> for RecoveryError {
    fn from(value: DestinationRejection) -> Self {
        RecoveryError::IoFailure(value.message())
    }
}

/// A destination directory that has passed the safety gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SafeDestination {
    root: PathBuf,
}

impl SafeDestination {
    pub fn path(&self) -> &Path {
        &self.root
    }
}

/// Normalises a path for comparison without requiring it to exist.
///
/// `canonicalize` only works on existing paths, and a destination directory may
/// not exist yet, so lexical normalisation is applied first. Symlinks are
/// resolved where the path does exist, since a symlinked destination could
/// otherwise point back at the source.
fn normalize(path: &Path) -> PathBuf {
    if let Ok(resolved) = path.canonicalize() {
        return resolved;
    }
    // Resolve the deepest existing ancestor, then re-append the rest, so a
    // not-yet-created directory still compares correctly.
    let mut prefix = path.to_path_buf();
    // Owned components, so the prefix can be truncated while walking up.
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !prefix.exists() {
        let Some(component) = prefix
            .components()
            .next_back()
            .map(|c| c.as_os_str().to_owned())
        else {
            break;
        };
        tail.push(component);
        if !prefix.pop() {
            break;
        }
    }
    let mut resolved = prefix.canonicalize().unwrap_or(prefix);
    for component in tail.into_iter().rev() {
        // Drop "." and resolve ".." against what we have so far.
        if component == std::ffi::OsStr::new(".") {
            continue;
        }
        if component == std::ffi::OsStr::new("..") {
            resolved.pop();
            continue;
        }
        resolved.push(component);
    }
    resolved
}

/// Whether `inner` is `outer` or lies beneath it.
fn contains(outer: &Path, inner: &Path) -> bool {
    inner == outer || inner.starts_with(outer)
}

/// Validates a destination against the source backing object.
///
/// `source_path` is the file or device backing the source. The destination is
/// refused when it is the source, contains the source, or lies inside it: any
/// of those would write recovered data over the evidence.
pub fn validate_destination(
    destination: &Path,
    source_path: Option<&Path>,
) -> Result<SafeDestination, DestinationRejection> {
    let destination_norm = normalize(destination);

    if let Some(source) = source_path {
        let source_norm = normalize(source);

        if destination_norm == source_norm {
            return Err(DestinationRejection::SameAsSourceFile);
        }
        // Writing inside the source, or around it, both endanger the evidence.
        if contains(&source_norm, &destination_norm) || contains(&destination_norm, &source_norm) {
            return Err(DestinationRejection::OverlapsSource);
        }
        // A destination sharing the source's parent directory is fine; only
        // containment is dangerous.
    }

    if destination_norm.exists() && !destination_norm.is_dir() {
        return Err(DestinationRejection::NotADirectory);
    }

    Ok(SafeDestination {
        root: destination_norm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "data-recovery-dest-{}-{}",
            std::process::id(),
            name
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn accepts_a_separate_directory() {
        let root = temp_dir("ok");
        let source = root.join("disk.img");
        fs::write(&source, b"image").unwrap();
        let destination = root.join("recovered");

        let safe = validate_destination(&destination, Some(&source)).unwrap();
        assert!(safe.path().ends_with("recovered"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn blocks_writing_onto_the_source_file() {
        let root = temp_dir("same");
        let source = root.join("disk.img");
        fs::write(&source, b"image").unwrap();

        assert_eq!(
            validate_destination(&source, Some(&source)),
            Err(DestinationRejection::SameAsSourceFile)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn blocks_a_destination_containing_the_source() {
        let root = temp_dir("outer");
        let inner = root.join("images");
        fs::create_dir_all(&inner).unwrap();
        let source = inner.join("disk.img");
        fs::write(&source, b"image").unwrap();

        // Recovering into a directory that holds the source risks the evidence.
        assert_eq!(
            validate_destination(&root, Some(&source)),
            Err(DestinationRejection::OverlapsSource)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn blocks_a_destination_inside_a_source_directory() {
        let root = temp_dir("inside");
        let source = root.join("mounted-source");
        fs::create_dir_all(&source).unwrap();
        let destination = source.join("recovered");

        assert_eq!(
            validate_destination(&destination, Some(&source)),
            Err(DestinationRejection::OverlapsSource)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolves_traversal_before_comparing() {
        let root = temp_dir("traverse");
        let inner = root.join("images");
        fs::create_dir_all(&inner).unwrap();
        let source = inner.join("disk.img");
        fs::write(&source, b"image").unwrap();

        // A path that walks back into the source directory must still be caught.
        let sneaky = inner.join("elsewhere").join("..").join("disk.img");
        assert_eq!(
            validate_destination(&sneaky, Some(&source)),
            Err(DestinationRejection::SameAsSourceFile)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_a_destination_that_is_a_file() {
        let root = temp_dir("file");
        let source = root.join("disk.img");
        fs::write(&source, b"image").unwrap();
        let destination = root.join("notes.txt");
        fs::write(&destination, b"text").unwrap();

        assert_eq!(
            validate_destination(&destination, Some(&source)),
            Err(DestinationRejection::NotADirectory)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_sibling_of_the_source_is_allowed() {
        let root = temp_dir("sibling");
        let source = root.join("disk.img");
        fs::write(&source, b"image").unwrap();
        // Sharing a parent is not containment.
        assert!(validate_destination(&root.join("out"), Some(&source)).is_ok());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn accepts_a_destination_when_the_source_is_a_device() {
        // A raw device has no meaningful path overlap with a directory.
        let root = temp_dir("device");
        assert!(validate_destination(&root.join("out"), None).is_ok());
        fs::remove_dir_all(&root).unwrap();
    }
}
