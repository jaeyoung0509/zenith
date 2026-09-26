use crate::large_files::identity_from_path;
#[cfg(not(target_os = "windows"))]
use crate::models::AppInstallSource;
use crate::models::{
    AppRelatedConfidence, AppRelatedItem, AppRelatedKind, AppUninstallInspection, InstalledApp,
    ObservationQuality, ReviewedFileIdentity,
};
use crate::safety::Blacklist;
#[cfg(not(target_os = "windows"))]
use plist::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(not(target_os = "windows"))]
use sysinfo::{ProcessesToUpdate, System};
use uuid::Uuid;
use zenith_platform::description::PlatformEnvironment;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const MAX_INCOMPLETE_REASONS: usize = 32;

#[derive(Debug, Clone)]
pub struct AppRecord {
    pub app: InstalledApp,
    pub path: PathBuf,
    pub identity: ReviewedFileIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPathMeasurement {
    pub logical_size: u64,
    pub allocated_size: u64,
    pub complete: bool,
    pub incomplete_reason: Option<String>,
    pub skipped_entries: u64,
}

impl AppPathMeasurement {
    pub fn quality(&self) -> ObservationQuality {
        if self.complete && self.skipped_entries == 0 {
            ObservationQuality::Fresh
        } else if self.logical_size > 0 || self.allocated_size > 0 {
            ObservationQuality::Partial
        } else {
            ObservationQuality::Unavailable
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppInventory {
    pub inventory_id: String,
    pub records: HashMap<String, AppRecord>,
    pub created_at: u64,
    pub quality: ObservationQuality,
    pub skipped_entry_count: u64,
    pub incomplete_reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RelatedRecord {
    pub item: AppRelatedItem,
    pub path: PathBuf,
    pub identity: ReviewedFileIdentity,
}

#[derive(Debug, Clone)]
pub struct AppInspectionRecord {
    pub inspection: AppUninstallInspection,
    pub app_path: PathBuf,
    pub app_identity: ReviewedFileIdentity,
    pub related: HashMap<String, RelatedRecord>,
    pub created_at: u64,
}

/// Filesystem probe seam for application discovery and size inspection.
/// Allows deterministic testing of permission and I/O failures without altering host permissions.
pub trait AppFsProbe: Send + Sync {
    fn symlink_metadata(&self, path: &Path) -> std::io::Result<fs::Metadata> {
        fs::symlink_metadata(path)
    }

    fn read_dir(&self, path: &Path) -> std::io::Result<fs::ReadDir> {
        fs::read_dir(path)
    }
}

pub struct NativeAppFsProbe;

impl AppFsProbe for NativeAppFsProbe {}

/// The bundle identifiers of the applications that are running right now.
///
/// A cleanup scan needs to know whether the application that owns a cache
/// namespace is using it: removing a cache out from under a running
/// application is a race the user did not ask for. The answer comes from the
/// process table rather than from the application inventory — a process whose
/// executable lives inside a `.app` bundle is running that bundle, so the
/// probe costs one process-table pass and one `Info.plist` read per running
/// bundle, and it needs no per-application rule and no inventory walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunningApplications {
    bundle_ids: Vec<String>,
    process_names: Vec<String>,
    process_state_known: bool,
}

impl RunningApplications {
    /// Asks the process table which application bundles are in use.
    ///
    /// Bundle identifiers are a macOS fact, and so is this probe: elsewhere no
    /// cache namespace is named after one, so the call costs nothing and
    /// reports nothing.
    pub fn probe() -> Self {
        #[cfg(target_os = "macos")]
        {
            let mut system = System::new();
            system.refresh_processes(ProcessesToUpdate::All, true);
            if system.processes().is_empty() {
                return Self::default();
            }
            let mut bundle_ids = Vec::new();
            let mut process_names = Vec::new();
            for process in system.processes().values() {
                process_names.push(process.name().to_string_lossy().into_owned());
                let Some(executable) = process.exe() else {
                    continue;
                };
                let Some(bundle) = app_bundle_of(executable) else {
                    continue;
                };
                if let Some(identifier) = bundle_identifier(bundle) {
                    bundle_ids.push(identifier);
                }
            }
            let mut observed = Self::from_ids(bundle_ids);
            observed.process_names = process_names;
            observed.process_state_known = true;
            observed
        }

        #[cfg(not(target_os = "macos"))]
        {
            Self::default()
        }
    }

    /// States the running bundles. The production probe is the process table;
    /// a test states what it wants the scan to see.
    pub fn from_ids(ids: impl IntoIterator<Item = String>) -> Self {
        let mut bundle_ids: Vec<String> = ids
            .into_iter()
            .map(|identifier| identifier.to_ascii_lowercase())
            .filter(|identifier| !identifier.is_empty())
            .collect();
        bundle_ids.sort();
        bundle_ids.dedup();
        Self {
            bundle_ids,
            process_names: Vec::new(),
            process_state_known: false,
        }
    }

    /// Matches a guarded cache owner against the same process snapshot used
    /// for app-bundle ownership; an unreadable table never means idle.
    pub fn running_executables(
        &self,
        policy: &zenith_core::domain::cleanup::RunningProcessPolicy,
    ) -> Option<Vec<String>> {
        if !self.process_state_known {
            return None;
        }
        let mut names: Vec<String> = self
            .process_names
            .iter()
            .filter(|name| policy.matches(name))
            .cloned()
            .collect();
        names.sort();
        names.dedup();
        Some(names)
    }

    /// The running application that owns a cache namespace, if any.
    ///
    /// A namespace belongs to a bundle when it is that bundle's identifier or
    /// one of its children (`com.example.app`, `com.example.app.helper`), which
    /// is how applications name their per-process caches.
    pub fn owner_of(&self, namespace: &str) -> Option<&str> {
        let namespace = namespace.to_ascii_lowercase();
        self.bundle_ids
            .iter()
            .find(|identifier| {
                namespace == **identifier
                    || namespace
                        .strip_prefix(identifier.as_str())
                        .is_some_and(|suffix| suffix.starts_with('.'))
            })
            .map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.bundle_ids.is_empty()
    }
}

/// The `.app` bundle a path lives inside, if any.
#[cfg(target_os = "macos")]
fn app_bundle_of(path: &Path) -> Option<&Path> {
    path.ancestors().find(|ancestor| {
        ancestor
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    })
}

#[cfg(target_os = "macos")]
fn bundle_identifier(bundle: &Path) -> Option<String> {
    read_bundle_metadata(bundle).metadata.bundle_id
}

pub struct ApplicationScanner;

impl ApplicationScanner {
    pub fn scan(environment: &PlatformEnvironment) -> AppInventory {
        Self::scan_with(environment, &NativeAppFsProbe)
    }

    pub fn scan_with<P: AppFsProbe>(environment: &PlatformEnvironment, probe: &P) -> AppInventory {
        // Windows has no reviewed application-bundle inventory: bundles,
        // their related data, and the uninstall review that consumes them are
        // macOS concepts. The refusal is stated by the environment's flavor so
        // it is provable on any runner, and a native Windows process states
        // Windows.
        if environment.flavor().is_windows() {
            let _ = probe;
            return empty_inventory();
        }

        #[cfg(target_os = "windows")]
        {
            empty_inventory()
        }

        #[cfg(not(target_os = "windows"))]
        {
            let mut records = HashMap::new();
            let mut skipped_entry_count = 0u64;
            let mut incomplete_reasons = Vec::new();
            let mut observed_roots = 0u64;

            // The reviewed system root and the stated profile's own folder are
            // the only places an application bundle is inventoried from.
            let mut roots = Vec::new();
            if let Some(system_root) = environment.program_files() {
                roots.push(system_root);
            }
            if let Some(home) = environment.user_home() {
                roots.push(home.join("Applications"));
            }
            if roots.is_empty() {
                return AppInventory {
                    inventory_id: Uuid::new_v4().to_string(),
                    records,
                    created_at: unix_timestamp(),
                    quality: ObservationQuality::Unavailable,
                    skipped_entry_count: 1,
                    incomplete_reasons: vec![
                        "Could not resolve any application discovery roots".to_string()
                    ],
                };
            }

            let mut system = System::new_all();
            system.refresh_processes(ProcessesToUpdate::All, true);
            let running_paths = system
                .processes()
                .values()
                .filter_map(|process| process.exe().map(Path::to_path_buf))
                .collect::<Vec<_>>();

            for root in roots {
                let meta = match probe.symlink_metadata(&root) {
                    Ok(m) => m,
                    Err(e) => {
                        if e.kind() != std::io::ErrorKind::NotFound {
                            skipped_entry_count = skipped_entry_count.saturating_add(1);
                            if incomplete_reasons.len() < MAX_INCOMPLETE_REASONS {
                                incomplete_reasons.push(format!(
                                    "Could not read root metadata {}: {e}",
                                    root.display()
                                ));
                            }
                        } else {
                            observed_roots = observed_roots.saturating_add(1);
                        }
                        continue;
                    }
                };
                if meta.file_type().is_symlink() {
                    // Symlink exclusion is an intentional scope boundary, not
                    // a failed observation.
                    observed_roots = observed_roots.saturating_add(1);
                    continue;
                }
                if !meta.is_dir() {
                    skipped_entry_count = skipped_entry_count.saturating_add(1);
                    if incomplete_reasons.len() < MAX_INCOMPLETE_REASONS {
                        incomplete_reasons.push(format!(
                            "Application root is not a directory: {}",
                            root.display()
                        ));
                    }
                    continue;
                }
                let entries = match probe.read_dir(&root) {
                    Ok(e) => e,
                    Err(e) => {
                        skipped_entry_count = skipped_entry_count.saturating_add(1);
                        if incomplete_reasons.len() < MAX_INCOMPLETE_REASONS {
                            incomplete_reasons.push(format!(
                                "Could not read application root directory {}: {e}",
                                root.display()
                            ));
                        }
                        continue;
                    }
                };
                observed_roots = observed_roots.saturating_add(1);
                for entry in entries {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(e) => {
                            skipped_entry_count = skipped_entry_count.saturating_add(1);
                            if incomplete_reasons.len() < MAX_INCOMPLETE_REASONS {
                                incomplete_reasons
                                    .push(format!("Could not read application entry: {e}"));
                            }
                            continue;
                        }
                    };
                    let path = entry.path();
                    if path.extension().and_then(|value| value.to_str()) != Some("app") {
                        continue;
                    }

                    let Some(identity) = identity_from_path(&path) else {
                        skipped_entry_count = skipped_entry_count.saturating_add(1);
                        if incomplete_reasons.len() < MAX_INCOMPLETE_REASONS {
                            incomplete_reasons.push(format!(
                                "Could not determine file identity for {}",
                                path.display()
                            ));
                        }
                        continue;
                    };

                    let bundle_obs = read_bundle_metadata(&path);
                    let metadata = bundle_obs.metadata;
                    let name = metadata
                        .display_name
                        .clone()
                        .or_else(|| {
                            path.file_stem()
                                .and_then(|value| value.to_str())
                                .map(str::to_string)
                        })
                        .unwrap_or_else(|| "Unknown App".to_string());

                    let measurement = measure_path_without_symlinks_with(&path, probe);
                    let size_quality = measurement.quality();
                    let mut app_incomplete_reason = measurement.incomplete_reason.clone();
                    let mut app_skipped_entries = measurement.skipped_entries;

                    let has_bundle_incomplete = bundle_obs.incomplete_reason.is_some();
                    if let Some(reason) = bundle_obs.incomplete_reason {
                        app_skipped_entries = app_skipped_entries.saturating_add(1);
                        if app_incomplete_reason.is_none() {
                            app_incomplete_reason = Some(reason.clone());
                        }
                        if incomplete_reasons.len() < MAX_INCOMPLETE_REASONS {
                            incomplete_reasons.push(reason);
                        }
                    }

                    let quality =
                        if has_bundle_incomplete || size_quality != ObservationQuality::Fresh {
                            ObservationQuality::Partial
                        } else {
                            ObservationQuality::Fresh
                        };

                    if quality != ObservationQuality::Fresh {
                        skipped_entry_count =
                            skipped_entry_count.saturating_add(app_skipped_entries.max(1));
                        if let Some(reason) = &measurement.incomplete_reason {
                            if incomplete_reasons.len() < MAX_INCOMPLETE_REASONS {
                                incomplete_reasons.push(reason.clone());
                            }
                        }
                    }

                    let is_system_protected =
                        is_zenith_identity(&name, metadata.bundle_id.as_deref());
                    let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
                    #[cfg(target_os = "windows")]
                    let norm_canonical =
                        zenith_platform::NativePlatformPaths::normalize_verbatim_path(&canonical);
                    let is_running = running_paths.iter().any(|exe| {
                        #[cfg(target_os = "windows")]
                        {
                            zenith_platform::NativePlatformPaths::windows_path_starts_with(
                                exe,
                                &norm_canonical,
                            )
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
                            exe.starts_with(&canonical)
                        }
                    });
                    let id = Uuid::new_v4().to_string();
                    let app = InstalledApp {
                        id: id.clone(),
                        name,
                        bundle_id: metadata.bundle_id,
                        version: metadata.version,
                        display_path: path.to_string_lossy().to_string(),
                        executable_name: metadata.executable,
                        logical_size: measurement.logical_size,
                        allocated_size: measurement.allocated_size,
                        modified_at: probe
                            .symlink_metadata(&path)
                            .ok()
                            .and_then(|value| value.modified().ok())
                            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                            .map(|value| value.as_secs()),
                        install_source: detect_install_source(&path),
                        is_running,
                        is_system_protected,
                        quality,
                        size_quality,
                        incomplete_reason: app_incomplete_reason,
                        skipped_entries: app_skipped_entries,
                    };
                    records.insert(
                        id,
                        AppRecord {
                            app,
                            path,
                            identity,
                        },
                    );
                }
            }

            let inventory_quality = if skipped_entry_count == 0 {
                ObservationQuality::Fresh
            } else if observed_roots > 0 {
                ObservationQuality::Partial
            } else {
                ObservationQuality::Unavailable
            };

            AppInventory {
                inventory_id: Uuid::new_v4().to_string(),
                records,
                created_at: unix_timestamp(),
                quality: inventory_quality,
                skipped_entry_count,
                incomplete_reasons,
            }
        }
    }

    pub fn inspect(
        environment: &PlatformEnvironment,
        inventory: &AppInventory,
        app_id: &str,
    ) -> Result<AppInspectionRecord, String> {
        Self::inspect_with(environment, inventory, app_id, &NativeAppFsProbe)
    }

    pub fn inspect_with<P: AppFsProbe>(
        environment: &PlatformEnvironment,
        inventory: &AppInventory,
        app_id: &str,
        probe: &P,
    ) -> Result<AppInspectionRecord, String> {
        let record = inventory
            .records
            .get(app_id)
            .ok_or_else(|| "Application inventory is stale. Refresh applications.".to_string())?;
        if is_zenith_app(&record.app) {
            return Err("Zenith cannot uninstall itself.".to_string());
        }
        if record.app.is_running {
            return Err(format!(
                "Quit {} before reviewing uninstall data.",
                record.app.name
            ));
        }
        if identity_from_path(&record.path).as_ref() != Some(&record.identity) {
            return Err(
                "The application changed after inventory. Refresh applications.".to_string(),
            );
        }

        let home = environment
            .user_home()
            .ok_or_else(|| "Could not resolve the user home directory".to_string())?;
        let bundle_id = record.app.bundle_id.clone();
        let normalized_name = record.app.name.trim().to_string();
        let mut related = HashMap::new();
        let mut incomplete = false;
        let mut warnings = Vec::new();

        // Profile-relative data roots. The profile comes from the environment,
        // and Windows uses the stated AppData folders when the platform
        // resolves them instead of guessing them from the profile spelling.
        #[cfg(not(target_os = "windows"))]
        let roots: Vec<(PathBuf, AppRelatedKind)> = [
            (
                "Library/Application Support",
                AppRelatedKind::ApplicationSupport,
            ),
            ("Library/Caches", AppRelatedKind::Cache),
            ("Library/Logs", AppRelatedKind::Log),
            ("Library/Preferences", AppRelatedKind::Preference),
            (
                "Library/Saved Application State",
                AppRelatedKind::SavedState,
            ),
            ("Library/Containers", AppRelatedKind::Container),
            ("Library/Group Containers", AppRelatedKind::GroupContainer),
            (
                "Library/Application Scripts",
                AppRelatedKind::ApplicationScripts,
            ),
            ("Library/HTTPStorages", AppRelatedKind::HttpStorage),
            ("Library/WebKit", AppRelatedKind::WebKit),
        ]
        .into_iter()
        .map(|(relative, kind)| (home.join(relative), kind))
        .collect();

        #[cfg(target_os = "windows")]
        let roots: Vec<(PathBuf, AppRelatedKind)> = vec![
            (
                environment
                    .local_app_data()
                    .unwrap_or_else(|| home.join("AppData/Local")),
                AppRelatedKind::Cache,
            ),
            (
                environment
                    .roaming_app_data()
                    .unwrap_or_else(|| home.join("AppData/Roaming")),
                AppRelatedKind::ApplicationSupport,
            ),
        ];

        for (root, kind) in roots {
            match probe.symlink_metadata(&root) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() || !meta.is_dir() {
                        continue;
                    }
                }
                Err(err) => {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        incomplete = true;
                        if warnings.len() < MAX_INCOMPLETE_REASONS {
                            warnings.push(format!(
                                "Could not access related data directory {}: {err}",
                                root.display()
                            ));
                        }
                    }
                    continue;
                }
            }
            let entries = match probe.read_dir(&root) {
                Ok(entries) => entries,
                Err(err) => {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        incomplete = true;
                        if warnings.len() < MAX_INCOMPLETE_REASONS {
                            warnings.push(format!(
                                "Could not read related data directory {}: {err}",
                                root.display()
                            ));
                        }
                    }
                    continue;
                }
            };
            for entry in entries {
                let Ok(entry) = entry else {
                    incomplete = true;
                    continue;
                };
                let path = entry.path();
                if Blacklist::is_blacklisted_with(&path, environment) {
                    continue;
                }
                match probe.symlink_metadata(&path) {
                    Ok(m) => {
                        if m.file_type().is_symlink() {
                            continue;
                        }
                    }
                    Err(err) => {
                        if err.kind() != std::io::ErrorKind::NotFound {
                            incomplete = true;
                            if warnings.len() < MAX_INCOMPLETE_REASONS {
                                warnings.push(format!(
                                    "Could not read metadata for {}: {err}",
                                    path.display()
                                ));
                            }
                        }
                        continue;
                    }
                }
                let filename = entry.file_name().to_string_lossy().to_string();
                let Some((confidence, evidence)) =
                    match_candidate(kind, &filename, bundle_id.as_deref(), &normalized_name)
                else {
                    continue;
                };
                let Some(identity) = identity_from_path(&path) else {
                    incomplete = true;
                    continue;
                };
                let measurement = measure_path_without_symlinks_with(&path, probe);
                let quality = measurement.quality();
                if quality != ObservationQuality::Fresh {
                    incomplete = true;
                }
                let id = Uuid::new_v4().to_string();
                let selected_by_default = confidence == AppRelatedConfidence::High;
                let item = AppRelatedItem {
                    id: id.clone(),
                    name: filename,
                    display_path: path.to_string_lossy().to_string(),
                    kind,
                    confidence,
                    evidence,
                    logical_size: measurement.logical_size,
                    allocated_size: measurement.allocated_size,
                    selected_by_default,
                    quality,
                    incomplete_reason: measurement.incomplete_reason,
                    skipped_entries: measurement.skipped_entries,
                };
                related.insert(
                    id,
                    RelatedRecord {
                        item,
                        path,
                        identity,
                    },
                );
            }
        }

        if let Some(reason) = &record.app.incomplete_reason {
            incomplete = true;
            if warnings.len() < MAX_INCOMPLETE_REASONS {
                warnings.push(format!(
                    "Application bundle observation was partial: {reason}"
                ));
            }
        }
        if bundle_id.is_none() {
            warnings.push("This app has no readable CFBundleIdentifier. Only exact app-name matches are shown and none are selected automatically.".to_string());
        }
        if incomplete {
            warnings.push(
                "Some protected or unreadable Library locations could not be inspected."
                    .to_string(),
            );
        }

        let inspection_id = Uuid::new_v4().to_string();
        let mut related_items = related
            .values()
            .map(|record| record.item.clone())
            .collect::<Vec<_>>();
        related_items.sort_by_key(|left| std::cmp::Reverse(left.allocated_size));
        let inspection = AppUninstallInspection {
            inspection_id,
            app: record.app.clone(),
            related_items,
            incomplete,
            warnings,
        };
        Ok(AppInspectionRecord {
            inspection,
            app_path: record.path.clone(),
            app_identity: record.identity.clone(),
            related,
            created_at: unix_timestamp(),
        })
    }
}

fn is_zenith_app(app: &InstalledApp) -> bool {
    app.is_system_protected || is_zenith_identity(&app.name, app.bundle_id.as_deref())
}

/// The inventory a platform without bundle support reports: empty, with a
/// fresh id so callers cannot mistake it for a populated one.
fn empty_inventory() -> AppInventory {
    AppInventory {
        inventory_id: Uuid::new_v4().to_string(),
        records: HashMap::new(),
        created_at: unix_timestamp(),
        quality: ObservationQuality::Fresh,
        skipped_entry_count: 0,
        incomplete_reasons: Vec::new(),
    }
}

fn is_zenith_identity(name: &str, bundle_id: Option<&str>) -> bool {
    name == "Zenith" || bundle_id == Some("com.zenith.desktop")
}

const MAX_APP_WALK_DEPTH: usize = 32;

#[cfg(test)]
fn measure_path_without_symlinks(path: &Path) -> AppPathMeasurement {
    measure_path_without_symlinks_with(path, &NativeAppFsProbe)
}

fn measure_path_without_symlinks_with<P: AppFsProbe>(path: &Path, probe: &P) -> AppPathMeasurement {
    let root_metadata = match probe.symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            return AppPathMeasurement {
                logical_size: 0,
                allocated_size: 0,
                complete: false,
                incomplete_reason: Some(format!(
                    "Could not read root metadata for {}: {e}",
                    path.display()
                )),
                skipped_entries: 1,
            };
        }
    };
    if root_metadata.file_type().is_symlink() {
        return AppPathMeasurement {
            logical_size: 0,
            allocated_size: 0,
            complete: false,
            incomplete_reason: Some(format!(
                "Refusing to follow root symlink: {}",
                path.display()
            )),
            skipped_entries: 1,
        };
    }

    #[cfg(unix)]
    let root_device = root_metadata.dev();
    let mut logical_size = 0u64;
    let mut allocated_size = 0u64;
    let mut complete = true;
    let mut incomplete_reason = None;
    let mut skipped_entries = 0u64;
    let mut stack = vec![(path.to_path_buf(), 0usize)];

    while let Some((current, depth)) = stack.pop() {
        let metadata = match probe.symlink_metadata(&current) {
            Ok(m) => m,
            Err(e) => {
                skipped_entries = skipped_entries.saturating_add(1);
                complete = false;
                if incomplete_reason.is_none() {
                    incomplete_reason = Some(format!(
                        "Could not read metadata for {}: {e}",
                        current.display()
                    ));
                }
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        #[cfg(unix)]
        if metadata.dev() != root_device {
            continue;
        }
        if metadata.is_file() {
            logical_size = logical_size.saturating_add(metadata.len());
            #[cfg(unix)]
            {
                allocated_size =
                    allocated_size.saturating_add(metadata.blocks().saturating_mul(512));
            }
            #[cfg(windows)]
            {
                let file_allocated =
                    crate::scanner::get_allocated_size(&current).unwrap_or(metadata.len());
                allocated_size = allocated_size.saturating_add(file_allocated);
            }
            #[cfg(not(any(unix, windows)))]
            {
                allocated_size = allocated_size.saturating_add(metadata.len());
            }
            continue;
        }
        if metadata.is_dir() {
            if depth >= MAX_APP_WALK_DEPTH {
                skipped_entries = skipped_entries.saturating_add(1);
                complete = false;
                if incomplete_reason.is_none() {
                    incomplete_reason = Some(format!(
                        "Directory depth limit exceeded at {}",
                        current.display()
                    ));
                }
                continue;
            }

            match probe.read_dir(&current) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(entry) => {
                                stack.push((entry.path(), depth + 1));
                            }
                            Err(e) => {
                                skipped_entries = skipped_entries.saturating_add(1);
                                complete = false;
                                if incomplete_reason.is_none() {
                                    incomplete_reason = Some(format!(
                                        "Could not read directory entry in {}: {e}",
                                        current.display()
                                    ));
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    skipped_entries = skipped_entries.saturating_add(1);
                    complete = false;
                    if incomplete_reason.is_none() {
                        incomplete_reason = Some(format!(
                            "Could not read directory {}: {e}",
                            current.display()
                        ));
                    }
                }
            }
        }
    }

    AppPathMeasurement {
        logical_size,
        allocated_size,
        complete,
        incomplete_reason,
        skipped_entries,
    }
}

#[cfg(not(target_os = "windows"))]
#[derive(Default)]
struct BundleMetadata {
    display_name: Option<String>,
    bundle_id: Option<String>,
    version: Option<String>,
    executable: Option<String>,
}

#[cfg(not(target_os = "windows"))]
struct BundleMetadataObservation {
    metadata: BundleMetadata,
    incomplete_reason: Option<String>,
}

#[cfg(not(target_os = "windows"))]
fn read_bundle_metadata(path: &Path) -> BundleMetadataObservation {
    let plist_path = path.join("Contents/Info.plist");
    match Value::from_file(&plist_path) {
        Ok(value) => {
            let Some(dict) = value.as_dictionary() else {
                return BundleMetadataObservation {
                    metadata: BundleMetadata::default(),
                    incomplete_reason: Some(format!(
                        "Info.plist in {} is not a valid dictionary",
                        path.display()
                    )),
                };
            };
            let get = |key: &str| dict.get(key).and_then(Value::as_string).map(str::to_string);
            BundleMetadataObservation {
                metadata: BundleMetadata {
                    display_name: get("CFBundleDisplayName").or_else(|| get("CFBundleName")),
                    bundle_id: get("CFBundleIdentifier"),
                    version: get("CFBundleShortVersionString").or_else(|| get("CFBundleVersion")),
                    executable: get("CFBundleExecutable"),
                },
                incomplete_reason: None,
            }
        }
        Err(err) => {
            let reason = match fs::symlink_metadata(&plist_path) {
                Ok(_) => format!("Failed to parse Info.plist in {}: {err}", path.display()),
                Err(metadata_error) if metadata_error.kind() == std::io::ErrorKind::NotFound => {
                    format!("Missing Info.plist in {}", path.display())
                }
                Err(metadata_error) => format!(
                    "Could not inspect Info.plist in {}: {metadata_error}",
                    path.display()
                ),
            };
            BundleMetadataObservation {
                metadata: BundleMetadata::default(),
                incomplete_reason: Some(reason),
            }
        }
    }
}

fn match_candidate(
    kind: AppRelatedKind,
    filename: &str,
    bundle_id: Option<&str>,
    app_name: &str,
) -> Option<(AppRelatedConfidence, String)> {
    if let Some(bundle_id) = bundle_id {
        let exact_bundle_match = filename == bundle_id
            || filename == format!("{bundle_id}.plist")
            || filename == format!("{bundle_id}.savedState");
        if exact_bundle_match {
            let shared = kind == AppRelatedKind::GroupContainer;
            return Some((
                if shared {
                    AppRelatedConfidence::Shared
                } else {
                    AppRelatedConfidence::High
                },
                if shared {
                    "Exact group/container identifier; treated as shared until exclusive ownership is proven"
                } else {
                    "Exact CFBundleIdentifier match"
                }
                .to_string(),
            ));
        }
    }
    if !app_name.is_empty() && filename.eq_ignore_ascii_case(app_name) {
        return Some((
            AppRelatedConfidence::Medium,
            "Exact application display-name match".to_string(),
        ));
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn detect_install_source(path: &Path) -> AppInstallSource {
    let text = path.to_string_lossy();
    if text.contains("/Caskroom/") {
        AppInstallSource::HomebrewCask
    } else {
        AppInstallSource::ApplicationBundle
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AppInstallSource;
    use std::io::Write;
    #[cfg(not(target_os = "windows"))]
    use std::sync::Arc;
    use zenith_core::domain::cleanup::RunningProcessPolicy;
    use zenith_platform::path_algebra::PathFlavor;

    #[test]
    fn one_process_snapshot_distinguishes_running_from_unknown_owner() {
        let guard = RunningProcessPolicy::guarding(vec!["Brave Browser Helper".into()]);
        assert_eq!(
            RunningApplications::default().running_executables(&guard),
            None
        );
        let mut observed = RunningApplications::from_ids(Vec::new());
        observed.process_names = vec!["Brave Browser Helper".into(), "unrelated".into()];
        observed.process_state_known = true;
        assert_eq!(
            observed.running_executables(&guard),
            Some(vec!["Brave Browser Helper".into()])
        );
    }
    #[cfg(not(target_os = "windows"))]
    use zenith_platform::paths::SimulatedPaths;

    #[cfg(not(target_os = "windows"))]
    #[derive(Default)]
    struct FailingAppFsProbe {
        metadata_failure: Option<PathBuf>,
        read_dir_failure: Option<PathBuf>,
    }

    #[cfg(not(target_os = "windows"))]
    impl super::AppFsProbe for FailingAppFsProbe {
        fn symlink_metadata(&self, path: &Path) -> std::io::Result<fs::Metadata> {
            if self.metadata_failure.as_deref() == Some(path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "simulated metadata refusal",
                ));
            }
            fs::symlink_metadata(path)
        }

        fn read_dir(&self, path: &Path) -> std::io::Result<fs::ReadDir> {
            if self.read_dir_failure.as_deref() == Some(path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "simulated directory refusal",
                ));
            }
            fs::read_dir(path)
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn write_test_bundle(bundle: &Path, name: &str) {
        let contents = bundle.join("Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(
            contents.join("Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleName</key><string>{name}</string><key>CFBundleIdentifier</key><string>com.example.{name}</string></dict></plist>"#
            ),
        )
        .unwrap();
        fs::write(contents.join("payload.bin"), b"visible payload").unwrap();
    }

    /// A POSIX environment stating the reviewed system root and the profile.
    #[cfg(not(target_os = "windows"))]
    fn environment_with_roots(system_root: &Path, home: &Path) -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current()).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::current())
                .with_home(home)
                .with_program_files(system_root),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn scan_uses_the_stated_application_roots_and_nothing_else() {
        let temp = tempfile::tempdir().unwrap();
        let system_root = temp.path().join("reviewed-applications");
        let profile = temp.path().join("profile");
        fs::create_dir_all(system_root.join("Reviewed.app")).unwrap();
        fs::create_dir_all(profile.join("Applications/Profile.app")).unwrap();
        fs::create_dir_all(temp.path().join("elsewhere/Unstated.app")).unwrap();

        let environment = environment_with_roots(&system_root, &profile);
        let inventory = ApplicationScanner::scan(&environment);
        let mut names = inventory
            .records
            .values()
            .map(|record| record.app.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["Profile".to_string(), "Reviewed".to_string()]);

        // Without a stated system root, a bundle elsewhere is not inventoried
        // even though the host has one.
        let profile_only =
            PlatformEnvironment::simulated(PathFlavor::current()).with_roots(Arc::new(
                SimulatedPaths::new()
                    .with_flavor(PathFlavor::current())
                    .with_home(&profile),
            ));
        let inventory = ApplicationScanner::scan(&profile_only);
        let names = inventory
            .records
            .values()
            .map(|record| record.app.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["Profile".to_string()]);
    }

    #[test]
    fn windows_flavor_reports_an_empty_inventory() {
        let inventory =
            ApplicationScanner::scan(&PlatformEnvironment::simulated(PathFlavor::Windows));

        assert!(inventory.records.is_empty());
        assert!(!inventory.inventory_id.is_empty());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn unresolved_posix_application_roots_report_unavailable_inventory() {
        let inventory =
            ApplicationScanner::scan(&PlatformEnvironment::simulated(PathFlavor::Posix));

        assert!(inventory.records.is_empty());
        assert_eq!(inventory.quality, ObservationQuality::Unavailable);
        assert_eq!(inventory.skipped_entry_count, 1);
        assert!(inventory.incomplete_reasons[0].contains("resolve"));
    }

    #[test]
    fn exact_bundle_identifier_is_high_confidence() {
        let (confidence, _) = match_candidate(
            AppRelatedKind::Preference,
            "com.example.Editor.plist",
            Some("com.example.Editor"),
            "Editor",
        )
        .unwrap();
        assert_eq!(confidence, AppRelatedConfidence::High);
    }

    #[test]
    fn substring_match_is_rejected() {
        assert!(match_candidate(
            AppRelatedKind::ApplicationSupport,
            "Editor Pro Backup",
            Some("com.example.Editor"),
            "Editor",
        )
        .is_none());
    }

    #[test]
    fn group_container_is_never_high_confidence() {
        let (confidence, _) = match_candidate(
            AppRelatedKind::GroupContainer,
            "group.com.example.Editor",
            Some("group.com.example.Editor"),
            "Editor",
        )
        .unwrap();
        assert_eq!(confidence, AppRelatedConfidence::Shared);
    }

    #[test]
    fn recognizes_the_configured_zenith_bundle_identifier() {
        let app = InstalledApp {
            id: "zenith".to_string(),
            name: "Renamed App".to_string(),
            bundle_id: Some("com.zenith.desktop".to_string()),
            version: None,
            display_path: "/Applications/Renamed App.app".to_string(),
            executable_name: None,
            logical_size: 0,
            allocated_size: 0,
            modified_at: None,
            install_source: AppInstallSource::ApplicationBundle,
            is_running: false,
            is_system_protected: false,
            quality: ObservationQuality::Fresh,
            size_quality: ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entries: 0,
        };
        assert!(is_zenith_app(&app));
    }

    #[test]
    fn dedicated_app_size_measurement_counts_files_without_following_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("Example.app");
        fs::create_dir_all(bundle.join("Contents")).unwrap();
        let mut file = fs::File::create(bundle.join("Contents/payload.bin")).unwrap();
        file.write_all(&[7; 4096]).unwrap();
        drop(file);
        // A second real file proves the walk descends the bundle on every
        // platform, so the assertion below is never vacuous.
        let mut nested = fs::File::create(bundle.join("Contents/nested.bin")).unwrap();
        nested.write_all(&[9; 1024]).unwrap();
        drop(nested);
        // A directory outside the bundle whose contents must never be counted.
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let mut escape = fs::File::create(outside.join("secret.bin")).unwrap();
        escape.write_all(&[1; 8192]).unwrap();
        drop(escape);

        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, bundle.join("Contents/escape")).unwrap();

        let measurement = measure_path_without_symlinks(&bundle);
        assert_eq!(measurement.logical_size, 4096 + 1024);
        assert!(measurement.allocated_size > 0);
        assert!(measurement.complete);
        assert_eq!(measurement.quality(), ObservationQuality::Fresh);
        assert_eq!(measurement.skipped_entries, 0);
    }

    #[test]
    fn unreadable_root_reports_unavailable_measurement() {
        let nonexistent = PathBuf::from("/nonexistent/path/for/test/Example.app");
        let measurement = measure_path_without_symlinks(&nonexistent);
        assert_eq!(measurement.quality(), ObservationQuality::Unavailable);
        assert_eq!(measurement.logical_size, 0);
        assert!(!measurement.complete);
        assert!(measurement.incomplete_reason.is_some());
        assert_eq!(measurement.skipped_entries, 1);
    }

    #[cfg(unix)]
    #[test]
    fn app_scan_with_symlinked_root_treats_it_as_an_intentional_exclusion() {
        let temp = tempfile::tempdir().unwrap();
        let real_apps = temp.path().join("real_apps");
        fs::create_dir_all(&real_apps).unwrap();
        let symlink_root = temp.path().join("Applications");

        std::os::unix::fs::symlink(&real_apps, &symlink_root).unwrap();

        let environment = PlatformEnvironment::simulated(PathFlavor::Posix).with_home(temp.path());
        let inventory = ApplicationScanner::scan(&environment);

        assert_eq!(inventory.skipped_entry_count, 0);
        assert_eq!(inventory.quality, ObservationQuality::Fresh);
        assert!(inventory.incomplete_reasons.is_empty());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn app_scan_retains_readable_records_when_another_root_cannot_be_read() {
        let temp = tempfile::tempdir().unwrap();
        let system_root = temp.path().join("system-applications");
        let profile = temp.path().join("profile");
        let profile_apps = profile.join("Applications");
        fs::create_dir_all(&system_root).unwrap();
        write_test_bundle(&profile_apps.join("Readable.app"), "Readable");

        let environment = environment_with_roots(&system_root, &profile);
        let probe = FailingAppFsProbe {
            read_dir_failure: Some(system_root.clone()),
            ..Default::default()
        };
        let inventory = ApplicationScanner::scan_with(&environment, &probe);

        assert_eq!(inventory.records.len(), 1);
        assert_eq!(inventory.quality, ObservationQuality::Partial);
        assert_eq!(inventory.skipped_entry_count, 1);
        assert!(inventory
            .incomplete_reasons
            .iter()
            .any(|reason| reason.contains(&system_root.display().to_string())));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn app_scan_retains_bundle_with_lower_bound_size_when_child_metadata_fails() {
        let temp = tempfile::tempdir().unwrap();
        let system_root = temp.path().join("system-applications");
        let profile = temp.path().join("profile");
        let bundle = profile.join("Applications/Partial.app");
        fs::create_dir_all(&system_root).unwrap();
        write_test_bundle(&bundle, "Partial");
        let refused_child = bundle.join("Contents/refused.bin");
        fs::write(&refused_child, vec![7; 4_096]).unwrap();

        let environment = environment_with_roots(&system_root, &profile);
        let probe = FailingAppFsProbe {
            metadata_failure: Some(refused_child),
            ..Default::default()
        };
        let inventory = ApplicationScanner::scan_with(&environment, &probe);

        assert_eq!(inventory.records.len(), 1);
        let app = &inventory.records.values().next().unwrap().app;
        assert_eq!(app.size_quality, ObservationQuality::Partial);
        assert!(app.allocated_size > 0);
        assert_eq!(app.skipped_entries, 1);
        assert_eq!(inventory.quality, ObservationQuality::Partial);
    }

    #[cfg(unix)]
    #[test]
    fn app_scan_with_corrupt_info_plist_marks_app_and_inventory_partial() {
        let temp = tempfile::tempdir().unwrap();
        let apps = temp.path().join("Applications");
        let bundle = apps.join("Broken.app");
        let contents = bundle.join("Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("Info.plist"), b"invalid binary or xml plist").unwrap();
        fs::write(contents.join("payload.bin"), b"binary").unwrap();

        let environment = PlatformEnvironment::simulated(PathFlavor::Posix).with_home(temp.path());
        let inventory = ApplicationScanner::scan(&environment);

        assert_eq!(inventory.records.len(), 1);
        let record = inventory.records.values().next().unwrap();
        assert_eq!(record.app.quality, ObservationQuality::Partial);
        assert_eq!(record.app.size_quality, ObservationQuality::Fresh);
        assert!(record
            .app
            .incomplete_reason
            .as_deref()
            .unwrap()
            .contains("Failed to parse Info.plist"));
        assert_eq!(inventory.quality, ObservationQuality::Partial);
        assert!(inventory.skipped_entry_count >= 1);
    }

    #[cfg(unix)]
    #[test]
    fn app_inspect_with_unreadable_related_root_reports_incomplete_and_warning() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let apps = home.join("Applications");
        let bundle = apps.join("Simple.app");
        let contents = bundle.join("Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("payload.bin"), b"binary").unwrap();

        let environment = PlatformEnvironment::simulated(PathFlavor::Posix).with_home(home);
        let inventory = ApplicationScanner::scan(&environment);
        assert_eq!(inventory.records.len(), 1);
        let app_id = inventory.records.keys().next().unwrap();

        let library = home.join("Library");
        let app_support = library.join("Application Support");
        fs::create_dir_all(&app_support).unwrap();

        let probe = FailingAppFsProbe {
            read_dir_failure: Some(app_support),
            ..Default::default()
        };

        let inspection =
            ApplicationScanner::inspect_with(&environment, &inventory, app_id, &probe).unwrap();

        assert!(inspection.inspection.incomplete);
        assert!(inspection
            .inspection
            .warnings
            .iter()
            .any(|w| w.contains("Could not read related data directory")));
    }

    #[test]
    fn a_running_bundle_owns_its_namespaces_and_their_children() {
        let running = RunningApplications::from_ids(vec![
            "com.example.App".to_string(),
            "com.other.Tool".to_string(),
            "com.example.App".to_string(),
        ]);
        assert_eq!(running.owner_of("com.example.app"), Some("com.example.app"));
        assert_eq!(
            running.owner_of("com.example.app.helper"),
            Some("com.example.app"),
            "an application's per-process caches belong to the application"
        );
        assert_eq!(running.owner_of("com.other.Tool"), Some("com.other.tool"));
        assert_eq!(running.owner_of("com.example.application"), None);
        assert_eq!(running.owner_of("com.example.apphelper"), None);
        assert_eq!(running.owner_of("com.someone.else"), None);

        // An empty probe (no application running, or another platform) owns
        // nothing rather than everything.
        let none = RunningApplications::default();
        assert!(none.is_empty());
        assert_eq!(none.owner_of("com.example.app"), None);
    }

    /// Bundle ownership is a macOS fact. On any other platform the probe must
    /// still exist, type-check, and answer "nothing is running" — rather than
    /// make every scan pay for a process-table pass whose answer cannot match
    /// a cache namespace.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_probe_is_inert_where_bundles_do_not_exist() {
        assert!(RunningApplications::probe().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_bundle_of_an_executable_is_the_application_that_runs_it() {
        assert_eq!(
            app_bundle_of(Path::new(
                "/Applications/Example.app/Contents/MacOS/Example"
            )),
            Some(Path::new("/Applications/Example.app"))
        );
        assert_eq!(
            app_bundle_of(Path::new(
                "/Applications/Example.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper"
            )),
            Some(Path::new(
                "/Applications/Example.app/Contents/Frameworks/Helper.app"
            ))
        );
        assert_eq!(app_bundle_of(Path::new("/usr/bin/git")), None);
        assert_eq!(
            app_bundle_of(Path::new("/Applications/Example/bin/tool")),
            None
        );
    }
}
