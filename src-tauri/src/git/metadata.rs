//! Bounded reads of the metadata files a repository pointer makes Zenith read.

use crate::safety::symlink::SymlinkGuard;
use std::path::{Path, PathBuf};

/// Byte cap for the files a repository pointer makes Zenith read: `<root>/.git`
/// when it is a pointer, the `HEAD` it resolves to, and the attribute file that
/// is inspected before any invocation. The size is stated before the read,
/// following the stat-then-cap convention the tree already uses for hashed and
/// audited files.
pub(crate) const MAX_GIT_METADATA_BYTES: u64 = 4_096;

/// Outcome of a capped read.
pub(crate) enum CappedRead {
    Read(String),
    Absent,
    /// The path exists, or could not safely be classified as absent, but its
    /// contents cannot be trusted as a complete UTF-8 metadata file.
    Unreadable,
    OverCap,
}

/// Reads one Git metadata file under [`MAX_GIT_METADATA_BYTES`], without
/// following a link at the path itself.
///
/// The limit is enforced on the open handle rather than on an earlier `stat`, so
/// a file that grows between the check and the read cannot exceed the cap: the
/// read stops one byte past it and reports the oversized case.
pub(crate) fn read_capped(path: &Path) -> CappedRead {
    match SymlinkGuard::is_symlink_metadata(path) {
        Ok(true) => {
            // A link or Windows reparse point is not read: its content would be
            // whatever it names rather than what is recorded at this path.
            return CappedRead::Unreadable;
        }
        Ok(false) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CappedRead::Absent;
        }
        Err(_) => return CappedRead::Unreadable,
    }
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CappedRead::Absent;
        }
        Err(_) => return CappedRead::Unreadable,
    };
    let Ok(metadata) = file.metadata() else {
        return CappedRead::Unreadable;
    };
    if !metadata.is_file() {
        return CappedRead::Unreadable;
    }
    let mut contents = String::new();
    let mut bounded = std::io::Read::take(file, MAX_GIT_METADATA_BYTES + 1);
    match std::io::Read::read_to_string(&mut bounded, &mut contents) {
        Ok(read) if read as u64 <= MAX_GIT_METADATA_BYTES => CappedRead::Read(contents),
        Ok(_) => CappedRead::OverCap,
        Err(_) => CappedRead::Unreadable,
    }
}

/// A path recorded in a Git metadata file, resolved against `base`.
pub(crate) fn recorded_path(base: &Path, raw: &str) -> Option<PathBuf> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    let path = Path::new(value);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    })
}

/// The path a `gitdir:` pointer file names, resolved against `root`.
pub(crate) fn gitfile_target(root: &Path, contents: &str) -> Option<PathBuf> {
    recorded_path(root, contents.trim().strip_prefix("gitdir:")?)
}

/// The first `key = value` entry in the `[core]` section of a Git configuration
/// file.
///
/// The file is repository content, so the parse is deliberately small and
/// section-aware: a value a different section carries is not one Git reads.
pub(crate) fn core_config_value(contents: &str, key: &str) -> Option<String> {
    let mut in_core = false;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[') {
            in_core = section
                .split(']')
                .next()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case("core"));
            continue;
        }
        if !in_core || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(key) {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capped_read_reports_absent_and_oversized_files() {
        let directory = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_capped(&directory.path().join("missing")),
            CappedRead::Absent
        ));

        let undecodable = directory.path().join("undecodable");
        std::fs::write(&undecodable, b"valid = line\n\xff").unwrap();
        assert!(matches!(read_capped(&undecodable), CappedRead::Unreadable));

        let small = directory.path().join("small");
        std::fs::write(&small, "gitdir: /tmp/other\n").unwrap();
        let CappedRead::Read(contents) = read_capped(&small) else {
            panic!("a small file is read");
        };
        assert_eq!(
            gitfile_target(Path::new("/workspace"), &contents),
            Some(PathBuf::from("/tmp/other"))
        );

        let oversized = directory.path().join("oversized");
        std::fs::write(&oversized, "x".repeat(MAX_GIT_METADATA_BYTES as usize + 1)).unwrap();
        assert!(matches!(read_capped(&oversized), CappedRead::OverCap));
    }

    #[test]
    fn a_core_value_is_read_only_from_the_core_section() {
        let contents = "[submodule \"x\"]\n\tworktree = /elsewhere\n[core]\n\tworktree = /here\n";
        assert_eq!(
            core_config_value(contents, "worktree").as_deref(),
            Some("/here")
        );
        assert_eq!(
            core_config_value("[core]\n\tbare = true\n", "worktree"),
            None
        );
    }
}
