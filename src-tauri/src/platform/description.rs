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

use serde::{Deserialize, Serialize};
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
    /// Stated temporary directory. `None` defers to the roots provider.
    temp_dir: Option<PathBuf>,
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
            temp_dir: None,
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
            temp_dir: None,
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

    /// States the user profile. The replacement roots keep this environment's
    /// flavor, so a Windows simulation stays Windows on a macOS host instead of
    /// silently becoming POSIX.
    pub fn with_home(self, home: impl Into<PathBuf>) -> Self {
        let roots = super::paths::SimulatedPaths::new()
            .with_flavor(self.flavor)
            .with_home(home);
        self.with_roots(Arc::new(roots))
    }

    pub fn with_known_folder(mut self, folder: KnownFolder, path: impl Into<PathBuf>) -> Self {
        self.known_folders.insert(folder, path.into());
        self
    }

    /// States the temporary directory. Without it the roots provider answers.
    pub fn with_temp_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.temp_dir = Some(path.into());
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

    // Accessors for callers that already hold the concrete environment, so
    // they do not need the provider trait in scope. `PlatformPathsProvider`
    // remains the injection seam for code that accepts any provider.
    pub fn user_home(&self) -> Option<PathBuf> {
        PlatformPathsProvider::user_home(self)
    }

    pub fn local_app_data(&self) -> Option<PathBuf> {
        PlatformPathsProvider::local_app_data(self)
    }

    pub fn roaming_app_data(&self) -> Option<PathBuf> {
        PlatformPathsProvider::roaming_app_data(self)
    }

    pub fn temp_dir(&self) -> PathBuf {
        PlatformPathsProvider::temp_dir(self)
    }

    pub fn program_files(&self) -> Option<PathBuf> {
        PlatformPathsProvider::program_files(self)
    }

    pub fn program_data(&self) -> Option<PathBuf> {
        PlatformPathsProvider::program_data(self)
    }

    pub fn content_dir(&self, token: &str) -> Option<PathBuf> {
        PlatformPathsProvider::content_dir(self, token)
    }

    pub fn expand_placeholder(&self, pattern: &str) -> Option<PathBuf> {
        PlatformPathsProvider::expand_placeholder(self, pattern)
    }
}

/// How the user profile is rooted. Only the shape travels, never the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileShape {
    /// `<drive>:\Users\<name>`
    DriveRooted,
    /// `\\server\share\<name>`
    Unc,
    /// `/home/<name>` or `/Users/<name>`
    PosixHome,
    /// The profile could not be resolved at all.
    Unresolved,
}

/// De-identified description of an environment.
///
/// This is what `--doctor` prints: drive letters, shapes, booleans and tool
/// names only — never a user name, machine name, company share, or profile
/// path. The same value is what a pasted report contributes back to the tree,
/// so a user's unusual machine becomes a committed fixture instead of a
/// machine nobody owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentShape {
    pub flavor: String,
    /// Drive letter of the system drive, when the platform has one.
    pub system_drive: Option<String>,
    pub profile_shape: ProfileShape,
    /// True when a known folder does not sit under the profile (Known Folder
    /// Move, redirected Documents/Desktop, or an administrator policy).
    pub known_folder_redirected: bool,
    /// True when the temporary directory lives inside the profile.
    pub temp_inside_profile: bool,
    /// True when every volume reports a stable identifier; false when at least
    /// one volume (a network share, for example) reports none.
    pub volume_identity: bool,
    /// Names of tools that were searched for and not found.
    pub missing_tools: Vec<String>,
}

/// A committed environment shape plus a name, picked up by the table-driven
/// fixture test. Fixtures state the environment; they never state a real path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentFixture {
    pub name: String,
    #[serde(flatten)]
    pub shape: EnvironmentShape,
}

impl EnvironmentFixture {
    pub fn new(name: impl Into<String>, shape: EnvironmentShape) -> Self {
        Self {
            name: name.into(),
            shape,
        }
    }

    /// Builds the environment this fixture describes, with synthetic but
    /// shape-faithful roots.
    pub fn environment(&self) -> PlatformEnvironment {
        let flavor = match self.shape.flavor.as_str() {
            "windows" => PathFlavor::Windows,
            _ => PathFlavor::Posix,
        };
        let environment = PlatformEnvironment::simulated(flavor);

        if flavor.is_windows() {
            let drive = self
                .shape
                .system_drive
                .clone()
                .unwrap_or_else(|| "C:".to_string());
            let home = match self.shape.profile_shape {
                ProfileShape::Unc => r"\\fixture-host\profiles\fixture".to_string(),
                _ => format!(r"{drive}\Users\fixture"),
            };
            let documents = if self.shape.known_folder_redirected {
                format!(r"{drive}\Redirected\Documents")
            } else {
                format!(r"{home}\Documents")
            };
            let temp = if self.shape.temp_inside_profile {
                format!(r"{home}\AppData\Local\Temp")
            } else {
                format!(r"{drive}\Temp")
            };
            let mut roots = super::paths::SimulatedPaths::new()
                .with_flavor(flavor)
                .with_home(home.clone())
                .with_temp_dir(temp);
            // App-data and program roots are stated only when the profile
            // names a drive: a UNC profile does not, and inventing a drive
            // there would misreport the machine's system drive.
            if matches!(
                self.shape.profile_shape,
                ProfileShape::DriveRooted | ProfileShape::Unresolved
            ) {
                roots = roots
                    .with_local_app_data(format!(r"{home}\AppData\Local"))
                    .with_roaming_app_data(format!(r"{home}\AppData\Roaming"))
                    .with_program_files(format!(r"{drive}\Program Files"))
                    .with_program_data(format!(r"{drive}\ProgramData"));
            }
            let mut environment = super::description::PlatformEnvironment::simulated(flavor)
                .with_roots(Arc::new(roots))
                .with_known_folder(KnownFolder::Documents, documents)
                .with_path_entry(format!(r"{drive}\Windows\System32"));
            environment = self.apply_volumes(
                environment,
                VolumeIdentity {
                    name: "system".to_string(),
                    mount_point: PathBuf::from(format!(r"{drive}\")),
                    filesystem: Some("NTFS".to_string()),
                    id: Some("fixture-volume".to_string()),
                },
                VolumeIdentity {
                    name: "mapped".to_string(),
                    mount_point: PathBuf::from(r"Z:\"),
                    filesystem: Some("SMB".to_string()),
                    id: None,
                },
            );
            return self.apply_tools(environment);
        }

        let home = "/home/fixture".to_string();
        let documents = if self.shape.known_folder_redirected {
            "/srv/redirected/Documents".to_string()
        } else {
            format!("{home}/Documents")
        };
        let temp = if self.shape.temp_inside_profile {
            format!("{home}/.cache/tmp")
        } else {
            "/var/tmp".to_string()
        };
        let roots = super::paths::SimulatedPaths::new()
            .with_flavor(flavor)
            .with_home(home.clone())
            .with_temp_dir(temp)
            .with_local_app_data(format!("{home}/.local/share"))
            .with_roaming_app_data(format!("{home}/.config"));
        let environment = environment
            .with_roots(Arc::new(roots))
            .with_known_folder(KnownFolder::Documents, documents)
            .with_path_entry("/usr/local/bin");
        let environment = self.apply_volumes(
            environment,
            VolumeIdentity {
                name: "root".to_string(),
                mount_point: PathBuf::from("/"),
                filesystem: Some("apfs".to_string()),
                id: Some("fixture-volume".to_string()),
            },
            VolumeIdentity {
                name: "network".to_string(),
                mount_point: PathBuf::from("/Volumes/fixture"),
                filesystem: Some("smbfs".to_string()),
                id: None,
            },
        );
        self.apply_tools(environment)
    }

    fn apply_volumes(
        &self,
        environment: PlatformEnvironment,
        with_identity: VolumeIdentity,
        without_identity: VolumeIdentity,
    ) -> PlatformEnvironment {
        if self.shape.volume_identity {
            environment.with_volumes(vec![with_identity])
        } else {
            environment.with_volumes(vec![without_identity])
        }
    }

    fn apply_tools(&self, environment: PlatformEnvironment) -> PlatformEnvironment {
        let mut environment = environment;
        for tool in &self.shape.missing_tools {
            environment = environment.with_missing_tool(tool);
        }
        environment
    }
}

impl PlatformEnvironment {
    /// De-identified shape of this environment.
    pub fn shape(&self) -> EnvironmentShape {
        let home_text = self
            .roots
            .user_home()
            .map(|home| home.to_string_lossy().to_string());
        let flavor = self.flavor;
        let profile_shape = match home_text.as_deref() {
            None => ProfileShape::Unresolved,
            Some(home) if crate::platform::path_algebra::is_unc(home, flavor) => ProfileShape::Unc,
            Some(_) if flavor.is_windows() => ProfileShape::DriveRooted,
            Some(_) => ProfileShape::PosixHome,
        };
        // A UNC profile does not name a drive even though the machine has one,
        // so fall back to the roots that do.
        let system_drive = profile_shape_drive(home_text.as_deref(), flavor).or_else(|| {
            let roots = [self.roots.program_data(), self.roots.program_files()];
            roots
                .into_iter()
                .flatten()
                .find_map(|root| profile_shape_drive(root.to_str(), flavor))
        });
        let known_folder_redirected = self.known_folders.iter().any(|(_, folder)| match home_text
            .as_deref()
        {
            Some(home) => {
                !crate::platform::path_algebra::contains(home, &folder.to_string_lossy(), flavor)
            }
            None => true,
        });
        let temp_inside_profile = match (self.temp_dir().to_str(), home_text.as_deref()) {
            (Some(temp), Some(home)) => crate::platform::path_algebra::contains(home, temp, flavor),
            _ => false,
        };
        let volume_identity = self
            .volumes
            .as_deref()
            .map(|volumes| volumes.iter().all(|volume| volume.id.is_some()))
            .unwrap_or(false);
        let mut missing_tools = self
            .tools
            .iter()
            .filter(|(_, resolution)| !resolution.is_found())
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        missing_tools.sort();

        EnvironmentShape {
            flavor: flavor.name().to_string(),
            system_drive,
            profile_shape,
            known_folder_redirected,
            temp_inside_profile,
            volume_identity,
            missing_tools,
        }
    }
}

fn profile_shape_drive(home: Option<&str>, flavor: PathFlavor) -> Option<String> {
    if !flavor.is_windows() {
        return None;
    }
    let home = home?;
    let mut chars = home.chars();
    let first = chars.next()?;
    let second = chars.next()?;
    if second == ':' && first.is_ascii_alphabetic() {
        return Some(format!("{}:", first.to_ascii_uppercase()));
    }
    None
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
        self.temp_dir
            .clone()
            .unwrap_or_else(|| self.roots.temp_dir())
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
    fn a_stated_home_keeps_the_environments_flavor() {
        // Replacing the roots must not silently re-flavor the environment:
        // that turns a Windows simulation into a POSIX one on a macOS host.
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows)
            .with_home(r"D:\Users\me")
            .with_known_folder(KnownFolder::Downloads, r"D:\Users\me\Downloads");

        assert_eq!(environment.flavor(), PathFlavor::Windows);
        assert_eq!(
            environment.expand_placeholder("~/.npm"),
            Some(PathBuf::from(r"D:\Users\me\.npm"))
        );
        assert_eq!(
            environment.content_dir("downloads"),
            Some(PathBuf::from(r"D:\Users\me\Downloads"))
        );
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

    fn shape_fixture(
        flavor: &str,
        system_drive: Option<&str>,
        profile_shape: ProfileShape,
        known_folder_redirected: bool,
        volume_identity: bool,
    ) -> EnvironmentFixture {
        EnvironmentFixture::new(
            "fixture",
            EnvironmentShape {
                flavor: flavor.to_string(),
                system_drive: system_drive.map(str::to_string),
                profile_shape,
                known_folder_redirected,
                temp_inside_profile: true,
                volume_identity,
                missing_tools: vec!["docker".to_string()],
            },
        )
    }

    #[test]
    fn a_committed_fixture_round_trips_through_its_shape() {
        for fixture in [
            shape_fixture("windows", Some("D:"), ProfileShape::DriveRooted, true, true),
            // A UNC profile names no drive, and the fixture states none.
            shape_fixture("windows", None, ProfileShape::Unc, false, false),
            shape_fixture("posix", None, ProfileShape::PosixHome, false, true),
        ] {
            let environment = fixture.environment();
            let shape = environment.shape();
            assert_eq!(
                shape, fixture.shape,
                "fixture {} did not round-trip",
                fixture.name
            );
        }
    }

    #[test]
    fn a_shape_carries_no_path_or_account_name() {
        let fixture = shape_fixture(
            "windows",
            Some("D:"),
            ProfileShape::DriveRooted,
            true,
            false,
        );
        let environment = fixture.environment();
        let serialized = serde_json::to_string(&environment.shape()).expect("serialize shape");

        assert!(!serialized.contains("Users\\"));
        assert!(!serialized.contains("fixture-host"));
        assert!(!serialized.contains("Documents"));
        // The shape names the drive letter, which is the fact under test.
        assert!(serialized.contains("D:"));
        assert!(serialized.contains("docker"));
        // And the environment the fixture built really does carry the facts.
        assert_eq!(environment.tool("docker"), Some(&ToolResolution::NotFound));
        assert!(environment
            .volumes()
            .is_some_and(|volumes| volumes.iter().all(|volume| volume.id.is_none())));
    }

    #[test]
    fn a_drive_rooted_fixture_protects_its_system_roots() {
        let environment = shape_fixture(
            "windows",
            Some("D:"),
            ProfileShape::DriveRooted,
            false,
            true,
        )
        .environment();

        assert_eq!(
            protected_root(r"D:\Windows\System32", environment.flavor()),
            Some(ProtectedRoot::WindowsDirectory)
        );
        let home = environment.user_home().expect("fixture home");
        assert!(crate::platform::path_algebra::contains(
            r"D:\Users\fixture",
            &home.to_string_lossy(),
            environment.flavor()
        ));
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
