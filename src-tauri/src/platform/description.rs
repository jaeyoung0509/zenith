//! Environment description: the platform facts a module is allowed to depend on.
//!
//! A module that reads `std::env` or a Win32 API directly cannot be exercised
//! against an environment other than the one it happens to run on, which is why
//! Windows behavior has historically only been provable on a Windows machine.
//! [`PlatformEnvironment`] is the injectable replacement: the real process
//! builds one with [`PlatformEnvironment::native`], tests build one with
//! [`PlatformEnvironment::simulated`] and state the facts they want to present —
//! a redirected known folder, a non-`C:` system drive, a UNC profile, a volume
//! without a stable identifier, or a missing external tool.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::path_algebra::PathFlavor;
use super::paths::{NativePlatformPaths, PlatformPathsProvider};

/// User-content folders whose real location is owned by the platform, not by
/// the literal `~/Downloads` spelling. Known Folder Move and administrator
/// redirection both move these without leaving a trace in the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KnownFolder {
    Downloads,
    Desktop,
    Documents,
    Movies,
}

impl KnownFolder {
    pub const ALL: [KnownFolder; 4] = [
        KnownFolder::Downloads,
        KnownFolder::Desktop,
        KnownFolder::Documents,
        KnownFolder::Movies,
    ];

    pub const fn token(self) -> &'static str {
        match self {
            KnownFolder::Downloads => "downloads",
            KnownFolder::Desktop => "desktop",
            KnownFolder::Documents => "documents",
            KnownFolder::Movies => "movies",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|folder| folder.token() == token)
    }
}

/// Whether an external tool the application may execute exists.
///
/// `NotFound` is a real answer, not an error: the container adapter reports
/// `unavailable` when its CLI is absent instead of returning an empty success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolResolution {
    Found(PathBuf),
    NotFound,
}

impl ToolResolution {
    pub fn path(&self) -> Option<&Path> {
        match self {
            ToolResolution::Found(path) => Some(path),
            ToolResolution::NotFound => None,
        }
    }

    pub fn is_found(&self) -> bool {
        matches!(self, ToolResolution::Found(_))
    }
}

/// Identity facts for one mounted volume.
///
/// `id` is `None` when the filesystem exposes no stable identifier (a network
/// share, some FUSE and virtual filesystems). Callers must not synthesize one
/// from the mount point: the identifier exists to detect that a mount was
/// replaced, and a derived value would silently claim the disk is the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeIdentity {
    pub name: String,
    pub mount_point: PathBuf,
    pub filesystem: Option<String>,
    pub id: Option<String>,
}

/// The platform facts available to the code under test.
#[derive(Clone)]
pub struct PlatformEnvironment {
    flavor: PathFlavor,
    roots: Arc<dyn PlatformPathsProvider>,
    known_folders: BTreeMap<KnownFolder, PathBuf>,
    path_entries: Vec<PathBuf>,
    volumes: Option<Vec<VolumeIdentity>>,
    tools: BTreeMap<String, ToolResolution>,
}

impl std::fmt::Debug for PlatformEnvironment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlatformEnvironment")
            .field("flavor", &self.flavor)
            .field("known_folders", &self.known_folders)
            .field("path_entries", &self.path_entries)
            .field("volumes", &self.volumes)
            .field("tools", &self.tools)
            .finish_non_exhaustive()
    }
}

impl PlatformEnvironment {
    /// The environment of the running process.
    pub fn native() -> Self {
        let native = NativePlatformPaths::new();
        let mut known_folders = BTreeMap::new();
        for folder in KnownFolder::ALL {
            if let Some(path) = native.content_dir(folder.token()) {
                known_folders.insert(folder, path);
            }
        }
        Self {
            flavor: PathFlavor::current(),
            roots: Arc::new(native),
            known_folders,
            path_entries: std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .collect(),
            // `None` means "ask the operating system" rather than "no volumes".
            volumes: None,
            // `None` entries mean "resolve through the platform tool search".
            tools: BTreeMap::new(),
        }
    }

    /// An empty description for a stated path flavor. Tests add only the facts
    /// they intend the code to observe.
    pub fn simulated(flavor: PathFlavor) -> Self {
        Self {
            flavor,
            roots: Arc::new(super::paths::SimulatedPaths::new().with_flavor(flavor)),
            known_folders: BTreeMap::new(),
            path_entries: Vec::new(),
            volumes: None,
            tools: BTreeMap::new(),
        }
    }

    pub fn flavor(&self) -> PathFlavor {
        self.flavor
    }

    pub fn roots(&self) -> &dyn PlatformPathsProvider {
        self.roots.as_ref()
    }

    pub fn with_roots(mut self, roots: Arc<dyn PlatformPathsProvider>) -> Self {
        self.roots = roots;
        self
    }

    pub fn with_home(self, home: impl Into<PathBuf>) -> Self {
        self.with_roots(Arc::new(
            super::paths::SimulatedPaths::default().with_home(home),
        ))
    }

    pub fn with_known_folder(mut self, folder: KnownFolder, path: impl Into<PathBuf>) -> Self {
        self.known_folders.insert(folder, path.into());
        self
    }

    pub fn with_path_entry(mut self, entry: impl Into<PathBuf>) -> Self {
        self.path_entries.push(entry.into());
        self
    }

    pub fn with_volumes(mut self, volumes: Vec<VolumeIdentity>) -> Self {
        self.volumes = Some(volumes);
        self
    }

    pub fn with_tool(mut self, name: &str, path: impl Into<PathBuf>) -> Self {
        self.tools
            .insert(name.to_string(), ToolResolution::Found(path.into()));
        self
    }

    pub fn with_missing_tool(mut self, name: &str) -> Self {
        self.tools
            .insert(name.to_string(), ToolResolution::NotFound);
        self
    }

    /// Resolved user-content folder, or `None` when the platform does not
    /// expose it. A folder resolved here wins over a literal profile path.
    pub fn known_folder(&self, folder: KnownFolder) -> Option<&PathBuf> {
        self.known_folders.get(&folder)
    }

    pub fn known_folders(&self) -> &BTreeMap<KnownFolder, PathBuf> {
        &self.known_folders
    }

    pub fn path_entries(&self) -> &[PathBuf] {
        &self.path_entries
    }

    /// Volumes when the description states them, otherwise `None` so the
    /// caller asks the operating system.
    pub fn volumes(&self) -> Option<&[VolumeIdentity]> {
        self.volumes.as_deref()
    }

    /// Tool resolution when the description states it, otherwise `None` so the
    /// caller uses the platform tool search.
    pub fn tool(&self, name: &str) -> Option<&ToolResolution> {
        self.tools.get(name)
    }
}

impl PlatformPathsProvider for PlatformEnvironment {
    fn user_home(&self) -> Option<PathBuf> {
        self.roots.user_home()
    }

    fn local_app_data(&self) -> Option<PathBuf> {
        self.roots.local_app_data()
    }

    fn roaming_app_data(&self) -> Option<PathBuf> {
        self.roots.roaming_app_data()
    }

    fn temp_dir(&self) -> PathBuf {
        self.roots.temp_dir()
    }

    fn program_files(&self) -> Option<PathBuf> {
        self.roots.program_files()
    }

    fn program_data(&self) -> Option<PathBuf> {
        self.roots.program_data()
    }

    fn content_dir(&self, token: &str) -> Option<PathBuf> {
        KnownFolder::parse(token).and_then(|folder| self.known_folders.get(&folder).cloned())
    }

    fn flavor(&self) -> PathFlavor {
        self.flavor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::path_algebra::{protected_root, PathFlavor, ProtectedRoot};

    #[test]
    fn simulated_profile_can_live_on_a_non_system_drive() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows)
            .with_home(r"D:\Users\홍 길동")
            .with_known_folder(KnownFolder::Documents, r"D:\Users\홍 길동\Documents");

        let home = environment.user_home().expect("simulated home");
        assert_eq!(home, PathBuf::from(r"D:\Users\홍 길동"));
        assert_eq!(
            environment.content_dir("documents"),
            Some(PathBuf::from(r"D:\Users\홍 길동\Documents"))
        );
        assert!(environment.content_dir("secrets").is_none());
    }

    #[test]
    fn redirected_known_folder_wins_over_the_profile_spelling() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows)
            .with_home(r"C:\Users\me")
            .with_known_folder(KnownFolder::Documents, r"C:\Users\me\OneDrive\Documents");

        assert_eq!(
            environment.known_folder(KnownFolder::Documents),
            Some(&PathBuf::from(r"C:\Users\me\OneDrive\Documents"))
        );
        // The redirected folder is the authority; the literal profile path is
        // not present at all, so a caller cannot accidentally trust it.
        assert_ne!(
            environment.content_dir("documents"),
            Some(PathBuf::from(r"C:\Users\me\Documents"))
        );
    }

    #[test]
    fn unc_profile_is_representable() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows)
            .with_home(r"\\fileserver\profiles\me");
        let home = environment.user_home().expect("simulated UNC home");
        assert_eq!(home, PathBuf::from(r"\\fileserver\profiles\me"));
        assert_eq!(
            protected_root(r"\\fileserver\profiles\me", PathFlavor::Windows),
            None
        );
    }

    #[test]
    fn a_volume_without_file_identity_is_representable() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix).with_volumes(vec![
            VolumeIdentity {
                name: "network".to_string(),
                mount_point: PathBuf::from("/Volumes/work"),
                filesystem: Some("smbfs".to_string()),
                id: None,
            },
            VolumeIdentity {
                name: "root".to_string(),
                mount_point: PathBuf::from("/"),
                filesystem: Some("apfs".to_string()),
                id: Some("disk1s5".to_string()),
            },
        ]);

        let volumes = environment.volumes().expect("stated volumes");
        assert_eq!(volumes.len(), 2);
        assert!(volumes[0].id.is_none());
    }

    #[test]
    fn a_missing_tool_is_an_answer_not_a_fallback() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix)
            .with_missing_tool("docker")
            .with_tool("podman", "/usr/local/bin/podman");

        assert_eq!(environment.tool("docker"), Some(&ToolResolution::NotFound));
        assert!(!environment.tool("docker").expect("stated").is_found());
        assert_eq!(
            environment.tool("podman").and_then(ToolResolution::path),
            Some(Path::new("/usr/local/bin/podman"))
        );
        assert_eq!(environment.tool("kubectl"), None);
    }

    #[test]
    fn native_environment_matches_the_compiled_flavor() {
        let environment = PlatformEnvironment::native();
        assert_eq!(environment.flavor(), PathFlavor::current());
        // Volumes and tools are unresolved so the caller keeps its OS path.
        assert!(environment.volumes().is_none());
        assert!(environment.tool("docker").is_none());
    }

    #[test]
    fn windows_roots_are_protected_even_when_the_description_is_not_native() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows)
            .with_home(r"D:\Users\me")
            .with_known_folder(KnownFolder::Desktop, r"D:\Users\me\Desktop");

        assert_eq!(
            protected_root(r"D:\Windows\System32", environment.flavor()),
            Some(ProtectedRoot::WindowsDirectory)
        );
        let desktop = environment
            .known_folder(KnownFolder::Desktop)
            .expect("redirected desktop");
        assert!(crate::platform::path_algebra::contains(
            r"D:\Users\me\Desktop",
            &desktop.to_string_lossy(),
            environment.flavor()
        ));
    }
}
