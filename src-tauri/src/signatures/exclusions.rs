//! One exclusion contract for discovery, measurement, and execution.
use super::SignatureLoader;
use std::path::Path;
use zenith_platform::path_algebra::{self, PathFlavor};
use zenith_platform::selector::PathSelector;
use zenith_platform::PlatformEnvironment;

pub fn is_excluded(path: &Path, exclusions: &[String], environment: &PlatformEnvironment) -> bool {
    exclusions.iter().any(|exclusion| {
        if let Some(expanded) = SignatureLoader::expand_exclusion(exclusion, environment) {
            return path_algebra::contains(
                &expanded.to_string_lossy(),
                &path.to_string_lossy(),
                environment.flavor(),
            );
        }
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| path_algebra::equal(name, exclusion, environment.flavor()))
    })
}

/// Whether a concrete path lies at or below a root selected by a pattern.
/// This uses the same selector parser as traversal, without touching the disk.
pub(crate) fn reachable_from(pattern: &str, path: &str, flavor: PathFlavor) -> bool {
    if !PathSelector::is_pattern(pattern) {
        return path_algebra::contains(pattern, path, flavor);
    }
    let Ok(selector) = PathSelector::parse(pattern, flavor) else {
        return false;
    };
    let mut candidate = path_algebra::split_path(path, flavor);
    loop {
        if selector.matches(&path_algebra::join_parts(&candidate, flavor), flavor) {
            return true;
        }
        if candidate.components.pop().is_none() {
            return false;
        }
    }
}

/// Keep placeholders symbolic at authoring time: no host filesystem or profile
/// may decide whether a portable catalog loads. Equivalent spellings share a
/// symbol; different roots must be named consistently by the author.
fn symbolic(value: &str) -> String {
    let mut value = SignatureLoader::normalize_pattern(value).replace('\\', "/");
    if value == "~" || value.starts_with("~/") {
        value = format!("${{USER_HOME}}{}", &value[1..]);
    }
    if value == "$TMPDIR" || value.starts_with("$TMPDIR/") {
        value = value.replacen("$TMPDIR", "${TEMP}", 1);
    }
    if let Some(rest) = value.strip_prefix("${") {
        if let Some((name, tail)) = rest.split_once('}') {
            return format!("/__zenith_placeholder/{name}{tail}");
        }
    }
    value
}

pub(crate) fn validate_reachability(
    signature: &crate::models::Signature,
) -> Result<(), crate::models::ZenithError> {
    for exclusion in &signature.exclusions {
        let path_shaped = exclusion.starts_with(['~', '$', '/', '\\'])
            || path_algebra::is_absolute(exclusion, PathFlavor::Windows);
        if !path_shaped {
            if exclusion.is_empty() || exclusion.contains(['/', '\\']) {
                return Err(crate::models::ZenithError::InvalidPlan(format!(
                    "Signature `{}` exclusion `{exclusion}` must be a bare entry name or rooted path",
                    signature.id,
                )));
            }
            continue;
        }
        let excluded = symbolic(exclusion);
        let reachable = signature.paths.iter().any(|root| {
            let root = symbolic(root);
            let flavor = if signature.platforms == [crate::models::PlatformKind::Windows]
                || (path_algebra::is_absolute(&root, PathFlavor::Windows) && !root.starts_with('/'))
            {
                PathFlavor::Windows
            } else {
                PathFlavor::Posix
            };
            reachable_from(&root, &excluded, flavor)
        });
        if !reachable {
            return Err(crate::models::ZenithError::InvalidPlan(format!(
                "Signature `{}` has unreachable exclusion `{exclusion}`",
                signature.id,
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_reachability_obeys_components_and_alternatives() {
        let root = symbolic("${LOCAL_APP_DATA}/Packages/*/{TempState,Temp}");
        assert!(reachable_from(
            &root,
            &symbolic("${LOCAL_APP_DATA}/Packages/app/TempState/keep"),
            PathFlavor::Posix
        ));
        assert!(!reachable_from(
            &root,
            &symbolic("${LOCAL_APP_DATA}/Packages/app/LocalState/keep"),
            PathFlavor::Posix
        ));
        assert!(!reachable_from(
            &symbolic("~/cache"),
            &symbolic("~/cache-neighbor/file"),
            PathFlavor::Posix
        ));
        assert!(reachable_from(
            &symbolic("~/cache"),
            &symbolic("${USER_HOME}/cache/file"),
            PathFlavor::Posix
        ));
        assert!(reachable_from(
            &symbolic("$TMPDIR"),
            &symbolic("${TEMP}/keep"),
            PathFlavor::Posix
        ));
    }

    #[test]
    fn filename_exclusions_are_exact_and_windows_paths_fold_case() {
        let posix = PlatformEnvironment::simulated(PathFlavor::Posix);
        let exclusions = vec!["onboarding.json".into()];
        assert!(is_excluded(
            Path::new("/cache/onboarding.json"),
            &exclusions,
            &posix
        ));
        assert!(!is_excluded(
            Path::new("/cache/onboarding.json.bak"),
            &exclusions,
            &posix
        ));
        let windows = PlatformEnvironment::simulated(PathFlavor::Windows);
        assert!(is_excluded(
            Path::new("C:/cache/ONBOARDING.JSON"),
            &exclusions,
            &windows
        ));
        assert!(is_excluded(
            Path::new("C:/CACHE/keep/file"),
            &["c:/cache/keep".into()],
            &windows
        ));
        assert!(!is_excluded(
            Path::new("C:/CACHE/keep-other/file"),
            &["c:/cache/keep".into()],
            &windows
        ));
    }
}
