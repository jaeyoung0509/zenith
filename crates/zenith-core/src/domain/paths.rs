//! Path primitives whose constructors enforce what their name claims.
//!
//! A `PathBuf` says nothing about whether it is rooted, whether it has been
//! resolved through symlinks, or whether it is a pseudo-URI that only looks
//! like a path. Every module that needs one of those properties has to
//! re-derive it, and a check that is written twice eventually gets written
//! differently. These wrappers move the check into the constructor and carry
//! the answer in the type.
//!
//! Neither wrapper is execution authority. A path that is absolute and
//! canonical is still untrusted: blacklist, signature-scope, symlink, and
//! TOCTOU validation all run on the value, not on its type.

use std::fmt;
use std::path::{Path, PathBuf};

/// A [`PathBuf`] that failed to satisfy a path invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathViolation {
    NotAbsolute(PathBuf),
    Unresolvable {
        path: PathBuf,
        kind: std::io::ErrorKind,
        reason: String,
    },
}

impl fmt::Display for PathViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathViolation::NotAbsolute(path) => {
                write!(f, "Expected an absolute path, received: {}", path.display())
            }
            PathViolation::Unresolvable { path, reason, .. } => write!(
                f,
                "Could not resolve the canonical location of {}: {}",
                path.display(),
                reason
            ),
        }
    }
}

impl std::error::Error for PathViolation {}

/// A path that is rooted in its own spelling.
///
/// The rootedness is recorded, not implied. A value of this type is safe to
/// compare against an approved root, because a relative path can never be
/// inside one — and a relative path compared against an approved root is the
/// failure this type exists to make unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AbsolutePath(PathBuf);

impl AbsolutePath {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, PathViolation> {
        let path = path.into();
        if path.is_absolute() {
            Ok(Self(path))
        } else {
            Err(PathViolation::NotAbsolute(path))
        }
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }

    pub fn join(&self, segment: impl AsRef<Path>) -> Self {
        // Joining a relative segment onto a rooted path stays rooted; joining
        // an absolute one replaces it, which is still rooted. Both outcomes
        // satisfy the invariant, so no fallible step is needed.
        Self(self.0.join(segment))
    }
}

impl AsRef<Path> for AbsolutePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl fmt::Display for AbsolutePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

/// A path that the filesystem resolved through every symlink in it.
///
/// Two different spellings of the same file canonicalize to the same value, so
/// a blacklist or scope check performed on a `CanonicalPath` cannot be dodged
/// by naming the target a second way. The resolution happened at construction:
/// a `CanonicalPath` never describes a path that has not been resolved, which
/// is why the fallible step is here rather than at each use site.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalPath(PathBuf);

impl CanonicalPath {
    /// Resolves `path` through the filesystem, following symlinks.
    pub fn resolve(path: impl AsRef<Path>) -> Result<Self, PathViolation> {
        let path = path.as_ref();
        match std::fs::canonicalize(path) {
            Ok(resolved) => Ok(Self(resolved)),
            Err(error) => Err(PathViolation::Unresolvable {
                path: path.to_path_buf(),
                kind: error.kind(),
                reason: error.to_string(),
            }),
        }
    }

    /// Reinterprets a value that is already the output of a canonicalization.
    ///
    /// Callers that resolve through an injected inspector rather than
    /// `std::fs` use this to keep the type without resolving twice. The
    /// resolved argument is the caller's assertion; it is named so that the
    /// assertion is visible at the call site.
    pub fn from_resolved(resolved: PathBuf) -> Self {
        Self(resolved)
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }

    pub fn join(&self, segment: impl AsRef<Path>) -> Self {
        Self(self.0.join(segment))
    }
}

impl AsRef<Path> for CanonicalPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl fmt::Display for CanonicalPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_is_not_an_absolute_path() {
        let violation = AbsolutePath::new("caches/tool").unwrap_err();
        assert_eq!(
            violation,
            PathViolation::NotAbsolute(PathBuf::from("caches/tool"))
        );
        assert!(violation.to_string().contains("Expected an absolute path"));
    }

    #[test]
    fn joining_keeps_an_absolute_path_absolute() {
        let root = AbsolutePath::new("/tmp").unwrap();
        assert!(root.join("child").as_path().is_absolute());
        assert!(root.join("/elsewhere").as_path().is_absolute());
    }

    #[test]
    fn resolving_a_missing_path_reports_why_rather_than_returning_it() {
        let missing = std::env::temp_dir().join("zenith-core-absent-path-fixture");
        let violation = CanonicalPath::resolve(&missing).unwrap_err();
        match violation {
            PathViolation::Unresolvable {
                path, kind, reason, ..
            } => {
                assert_eq!(path, missing);
                assert_eq!(kind, std::io::ErrorKind::NotFound);
                assert!(!reason.is_empty());
            }
            other => panic!("expected PathViolation::Unresolvable, got {other:?}"),
        }
    }

    /// Symlink creation needs a platform-specific call; the resolution rule it
    /// exercises is the same one on every platform.
    #[cfg(unix)]
    #[test]
    fn resolving_a_symlink_yields_the_link_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let resolved = CanonicalPath::resolve(&link).unwrap();
        assert_eq!(resolved.as_path(), real.canonicalize().unwrap());
        assert_ne!(resolved.as_path(), link);
    }
}
