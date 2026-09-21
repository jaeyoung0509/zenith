use crate::models::{Signature, SignatureManifest, ZenithError};
use std::path::PathBuf;
use zenith_platform::path_algebra::PathFlavor;
use zenith_platform::{KnownFolder, PlatformEnvironment};

pub struct SignatureLoader;

impl SignatureLoader {
    /// Rewrites a Windows `%VAR%` spelling into the placeholder the expander
    /// resolves.
    ///
    /// Every Windows path list in the wild is written with `%VAR%`, while the
    /// expander accepts `${VAR}`. Normalizing here means the catalog has one
    /// spelling, the lint and the platform classification see that spelling,
    /// and a manifest cannot silently resolve to nothing because it was copied
    /// from documentation. A `%VAR%` this table does not know is left alone and
    /// refused by `Signature::validate`, so an unsupported spelling is an
    /// error rather than an empty expansion.
    pub fn normalize_pattern(value: &str) -> String {
        // Only spellings whose placeholder resolves to the same directory are
        // listed. `%PROGRAMFILES(X86)%` is deliberately absent: on a 64-bit
        // machine it is a different tree from `%PROGRAMFILES%`, so mapping it
        // onto `${PROGRAM_FILES}` would quietly point a signature at the wrong
        // root. A manifest that uses it fails the load and names the supported
        // spellings instead.
        const KNOWN: [(&str, &str); 9] = [
            ("USERPROFILE", "${USER_HOME}"),
            ("LOCALAPPDATA", "${LOCAL_APP_DATA}"),
            ("APPDATA", "${ROAMING_APP_DATA}"),
            ("PROGRAMDATA", "${PROGRAM_DATA}"),
            ("PROGRAMFILES", "${PROGRAM_FILES}"),
            ("SYSTEMROOT", "${SYSTEM_ROOT}"),
            ("WINDIR", "${SYSTEM_ROOT}"),
            ("TEMP", "${TEMP}"),
            // `%TMP%` is the short spelling of the same variable.
            ("TMP", "${TEMP}"),
        ];

        let mut normalized = value.to_string();
        for (name, placeholder) in KNOWN {
            let mut search_from = 0;
            while let Some(open) = normalized[search_from..]
                .find('%')
                .map(|offset| search_from + offset)
            {
                let Some(close) = normalized[open + 1..]
                    .find('%')
                    .map(|offset| open + 1 + offset)
                else {
                    break;
                };
                let inner = normalized[open + 1..close].to_string();
                if inner.eq_ignore_ascii_case(name) {
                    normalized.replace_range(open..=close, placeholder);
                    search_from = open + placeholder.len();
                } else {
                    search_from = close + 1;
                }
            }
        }
        normalized
    }

    /// Loads signatures from a TOML string.
    ///
    /// A signature that contradicts itself is a load failure rather than a
    /// dropped entry: the catalog is the only statement of what may be cleaned,
    /// and a manifest that half-loads is a catalog nobody can reason about. An
    /// id that is missing entirely is the one exception — the entry names
    /// nothing to report and nothing to register.
    pub fn load_str(content: &str) -> Result<Vec<Signature>, ZenithError> {
        let manifest: SignatureManifest = toml::from_str(content)
            .map_err(|e| ZenithError::Io(format!("Failed to parse TOML signature: {}", e)))?;

        let mut valid_signatures = Vec::new();
        for mut sig in manifest.signatures {
            if sig.id.trim().is_empty() {
                continue;
            }
            for pattern in sig.paths.iter_mut() {
                *pattern = Self::normalize_pattern(pattern);
            }
            for exclusion in sig.exclusions.iter_mut() {
                *exclusion = Self::normalize_pattern(exclusion);
            }
            sig.validate()?;
            super::exclusions::validate_reachability(&sig)?;
            valid_signatures.push(sig);
        }

        Ok(valid_signatures)
    }

    /// Expands platform placeholders (`${USER_HOME}`, `${LOCAL_APP_DATA}`, `~`, `$TMPDIR`)
    /// safely against the described environment.
    ///
    /// A user-content folder is owned by the platform, not by its literal
    /// profile spelling: Known Folder Move and administrator redirection move
    /// the real folder without leaving a trace in `$HOME`, so a resolved known
    /// folder (`~/Downloads`, `${USER_HOME}\Documents`) is expanded through the
    /// environment's own resolution instead of `home.join(name)`.
    ///
    /// This is the single resolution boundary for signature paths: the scanner,
    /// the plan verifier, and the cache providers all expand through it, so a
    /// simulated environment resolves exactly the paths it states.
    pub fn expand_path(pattern: &str, environment: &PlatformEnvironment) -> Option<PathBuf> {
        match Self::substitute_known_folder(pattern, environment) {
            Some(substituted) => environment.expand_placeholder(&substituted),
            None => environment.expand_placeholder(pattern),
        }
    }

    /// Expands an exclusion the walkers treat as a path.
    ///
    /// An exclusion is path-shaped when it names the profile (`~`), uses a
    /// placeholder (`${...}`), or is absolute in the described environment's
    /// own spelling. Every one of those goes through the same resolution as a
    /// signature path, so the manifest lint, the size calculator, and the tree
    /// deleter agree on which exclusions protect a file. Anything else is a
    /// bare file name matched against the entry's own name.
    pub fn expand_exclusion(exclusion: &str, environment: &PlatformEnvironment) -> Option<PathBuf> {
        let trimmed = exclusion.trim();
        if trimmed.is_empty() {
            return None;
        }
        let path_shaped = trimmed.starts_with('~')
            || trimmed.starts_with('$')
            || zenith_platform::path_algebra::is_absolute(trimmed, environment.flavor());
        if !path_shaped {
            return None;
        }
        Self::expand_path(trimmed, environment)
    }

    /// Rewrites a user-content folder pattern (`~/Documents/x`) to the
    /// environment's resolved folder when it states one. `None` when the
    /// pattern does not name a known folder or nothing is stated for it, so the
    /// caller falls back to the placeholder expansion.
    fn substitute_known_folder(pattern: &str, environment: &PlatformEnvironment) -> Option<String> {
        let (folder, tail) = Self::split_known_folder(pattern, environment.flavor())?;
        let root = environment.known_folder(folder)?;
        if tail.is_empty() {
            return Some(root.to_string_lossy().into_owned());
        }
        Some(zenith_platform::path_algebra::join(
            &root.to_string_lossy(),
            &tail,
            environment.flavor(),
        ))
    }

    fn split_known_folder(pattern: &str, flavor: PathFlavor) -> Option<(KnownFolder, String)> {
        let trimmed = pattern.trim();
        let rest = trimmed
            .strip_prefix("~/")
            .or_else(|| trimmed.strip_prefix("${USER_HOME}/"))
            .or_else(|| trimmed.strip_prefix("${USER_HOME}\\"))?;
        let (first, tail) = match rest.split_once(['/', '\\']) {
            Some((first, tail)) => (first, Some(tail)),
            None => (rest, None),
        };
        let folder = known_folder_component(first, flavor)?;
        Some((folder, tail.unwrap_or_default().replace('\\', "/")))
    }
}

/// Maps a profile-relative component to a known folder. POSIX folder names are
/// case sensitive (`~/downloads` is not `~/Downloads`); Windows resolves the
/// shell folder case-insensitively.
fn known_folder_component(component: &str, flavor: PathFlavor) -> Option<KnownFolder> {
    KnownFolder::ALL.into_iter().find(|folder| {
        let token = folder.token();
        if flavor.is_windows() {
            return component.eq_ignore_ascii_case(token);
        }
        let mut name = token.to_string();
        if let Some(first) = name.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        component == name
    })
}

#[cfg(test)]
mod tests {
    use super::SignatureLoader;
    use std::path::PathBuf;
    use std::sync::Arc;
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::paths::SimulatedPaths;
    use zenith_platform::{KnownFolder, PlatformEnvironment};

    #[test]
    fn catalog_requires_an_explicit_strategy_and_reachable_exclusions() {
        let valid = r#"
[[signatures]]
id = "test.exclusions"
name = "Exclusions"
category = "system"
risk = "safe"
strategy = "delete_contents"
paths = ["~/.cache/tool"]
exclusions = ["~/.cache/tool/keep.bin"]
"#;
        assert_eq!(SignatureLoader::load_str(valid).unwrap().len(), 1);
        let missing = valid.replace("strategy = \"delete_contents\"", "");
        assert!(SignatureLoader::load_str(&missing)
            .unwrap_err()
            .to_string()
            .contains("strategy"));
        let unreachable = valid.replace("~/.cache/tool/keep.bin", "~/.cache/sibling/keep.bin");
        assert!(SignatureLoader::load_str(&unreachable)
            .unwrap_err()
            .to_string()
            .contains("unreachable exclusion"));
        let relative = valid.replace("~/.cache/tool/keep.bin", "relative/keep.bin");
        assert!(SignatureLoader::load_str(&relative)
            .unwrap_err()
            .to_string()
            .contains("bare entry name"));
        let selector = valid
            .replace("~/.cache/tool\"]", "~/.cache/*/{Cache,Temp}\"]")
            .replace("~/.cache/tool/keep.bin", "~/.cache/editor/Cache/keep.bin");
        assert_eq!(SignatureLoader::load_str(&selector).unwrap().len(), 1);
    }

    /// A manifest written the way Windows documents paths resolves, and a
    /// spelling this build does not know fails the load instead of quietly
    /// matching nothing.
    #[test]
    fn a_windows_variable_spelling_is_normalized_or_refused() {
        let manifest = r#"
[[signatures]]
id = "test.normalized"
name = "Normalized"
category = "system"
risk = "safe"
strategy = "delete_directory"
paths = [
    "%LOCALAPPDATA%\\Temp",
    "%TMP%",
    "%SystemRoot%\\Temp",
    "%USERPROFILE%\\AppData\\Roaming",
    "%APPDATA%\\tool",
]
exclusions = ["%APPDATA%\\tool\\settings.json"]
"#;
        let signatures = SignatureLoader::load_str(manifest).expect("the manifest loads");
        let signature = &signatures[0];
        assert_eq!(signature.paths[0], "${LOCAL_APP_DATA}\\Temp");
        assert_eq!(signature.paths[1], "${TEMP}");
        assert_eq!(signature.paths[2], "${SYSTEM_ROOT}\\Temp");
        assert_eq!(signature.paths[3], "${USER_HOME}\\AppData\\Roaming");
        assert_eq!(
            signature.exclusions[0],
            "${ROAMING_APP_DATA}\\tool\\settings.json"
        );

        // An unknown variable is refused rather than left to resolve to
        // nothing: a manifest that names a root this build cannot resolve would
        // otherwise be a scan that silently covers less.
        let unknown = manifest.replace("%LOCALAPPDATA%", "%SOMEWHERE_ELSE%");
        let error =
            SignatureLoader::load_str(&unknown).expect_err("an unknown variable is refused");
        assert!(
            error.to_string().contains("environment spelling"),
            "the refusal names the problem: {error}"
        );

        // `%PROGRAMFILES(X86)%` is a different directory on a 64-bit machine,
        // so it is refused instead of being mapped onto the 64-bit root.
        let x86 = manifest.replace("%LOCALAPPDATA%", "%PROGRAMFILES(X86)%");
        let error =
            SignatureLoader::load_str(&x86).expect_err("the x86 root is not the 64-bit one");
        assert!(
            error.to_string().contains("environment spelling"),
            "the refusal names the problem: {error}"
        );
    }

    #[test]
    fn expand_path_preserves_absolute_paths_without_home_lookup() {
        let abs = if cfg!(windows) {
            r"C:\test\folder"
        } else {
            "/tmp/test_folder"
        };
        let environment = PlatformEnvironment::simulated(PathFlavor::current());
        let expanded = SignatureLoader::expand_path(abs, &environment);
        assert_eq!(expanded, Some(PathBuf::from(abs)));
    }

    #[test]
    fn expand_path_expands_the_stated_temp_directory() {
        let temp = if cfg!(windows) {
            PathBuf::from(r"D:\Temp\zenith")
        } else {
            PathBuf::from("/tmp/zenith-stated")
        };
        let flavor = PathFlavor::current();
        let environment = PlatformEnvironment::simulated(flavor).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(flavor)
                .with_temp_dir(&temp),
        ));

        assert_eq!(
            SignatureLoader::expand_path("$TMPDIR", &environment),
            Some(temp.clone())
        );
        assert_eq!(
            SignatureLoader::expand_path("${TEMP}", &environment),
            Some(temp.clone())
        );
        assert_eq!(
            SignatureLoader::expand_path("${TEMP}/nested", &environment),
            Some(temp.join("nested"))
        );
    }

    #[test]
    fn expand_path_prefers_the_redirected_known_folder_over_the_profile_spelling() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows)
            .with_home(r"C:\Users\me")
            .with_known_folder(KnownFolder::Downloads, r"D:\Cache\Downloads")
            .with_known_folder(KnownFolder::Documents, r"\\fileserver\home\docs");

        assert_eq!(
            SignatureLoader::expand_path("~/Downloads/uv", &environment),
            Some(PathBuf::from(r"D:\Cache\Downloads\uv"))
        );
        assert_eq!(
            SignatureLoader::expand_path("${USER_HOME}/Downloads", &environment),
            Some(PathBuf::from(r"D:\Cache\Downloads"))
        );
        assert_eq!(
            SignatureLoader::expand_path(r"${USER_HOME}\Documents\cache", &environment),
            Some(PathBuf::from(r"\\fileserver\home\docs\cache"))
        );

        // A folder the environment does not state still resolves through the
        // stated profile: the redirect is only authoritative when it is known.
        assert_eq!(
            SignatureLoader::expand_path("~/Movies", &environment),
            Some(PathBuf::from(r"C:\Users\me\Movies"))
        );
    }

    #[test]
    fn exclusion_expansion_is_shared_by_every_consumer() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Windows)
                .with_home(r"D:\Users\me")
                .with_local_app_data(r"E:\Profiles\me\AppData\Local"),
        ));

        // Every path-shaped spelling expands, including the placeholder form
        // the size calculator and the tree deleter used to ignore.
        for (exclusion, expected) in [
            (r"~\Documents\keep", r"D:\Users\me\Documents\keep"),
            (
                "${LOCAL_APP_DATA}/Vendor/config.json",
                r"E:\Profiles\me\AppData\Local\Vendor\config.json",
            ),
            (r"D:\Documents\keep", r"D:\Documents\keep"),
            (r"\\fileserver\share\keep", r"\\fileserver\share\keep"),
        ] {
            assert_eq!(
                SignatureLoader::expand_exclusion(exclusion, &environment),
                Some(PathBuf::from(expected)),
                "exclusion {exclusion}"
            );
        }

        // A bare file name is not a path and is matched by name instead.
        assert_eq!(
            SignatureLoader::expand_exclusion("settings.json", &environment),
            None
        );
        assert_eq!(SignatureLoader::expand_exclusion("", &environment), None);
    }

    #[test]
    fn a_posix_known_folder_component_stays_case_sensitive() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix)
            .with_home("/home/tester")
            .with_known_folder(KnownFolder::Downloads, "/mnt/data/downloads");

        assert_eq!(
            SignatureLoader::expand_path("~/Downloads/cache", &environment),
            Some(PathBuf::from("/mnt/data/downloads/cache"))
        );
        // `~/downloads` names a different directory on POSIX, so it is not the
        // known folder and resolves through the literal profile spelling.
        assert_eq!(
            SignatureLoader::expand_path("~/downloads/cache", &environment),
            Some(PathBuf::from("/home/tester/downloads/cache"))
        );
    }
}
