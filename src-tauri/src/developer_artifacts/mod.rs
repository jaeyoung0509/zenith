use crate::large_files::FileIdentity;
use crate::models::{
    DeveloperArtifact, DeveloperArtifactKind, DeveloperArtifactScanEvent,
    DeveloperArtifactScanResult, DeveloperEcosystem, DeveloperWorkspace,
};
use crate::safety::{Blacklist, SymlinkGuard};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const MAX_WORKSPACES: usize = 16;
const MAX_CANDIDATES: usize = 512;
const MAX_DISCOVERY_DEPTH: usize = 64;
const MAX_MEASUREMENT_DEPTH: usize = 64;
const MAX_DISCOVERY_ENTRIES: u64 = 250_000;
const INVENTORY_TTL_SECS: u64 = 15 * 60;

/// Workspace roots are process-local on purpose for the MVP. They are bounded
/// and the scan revalidates every root before doing any filesystem work.
pub(crate) static WORKSPACES: LazyLock<Mutex<HashMap<String, DeveloperWorkspaceRecord>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
pub struct DeveloperWorkspaceRecord {
    pub workspace: DeveloperWorkspace,
    pub path: PathBuf,
    pub identity: FileIdentity,
    pub created_at: u64,
    pub whole_home: bool,
}

#[derive(Debug, Clone)]
pub struct DeveloperArtifactRecord {
    pub artifact: DeveloperArtifact,
    pub path: PathBuf,
    pub identity: FileIdentity,
    pub workspace_path: PathBuf,
    pub workspace_identity: FileIdentity,
    pub project_root: PathBuf,
    pub project_identity: FileIdentity,
    pub artifact_relative: PathBuf,
    pub marker_identities: Vec<(PathBuf, FileIdentity)>,
}

#[derive(Debug, Clone)]
pub struct DeveloperArtifactInventory {
    pub scan_id: String,
    pub records: HashMap<String, DeveloperArtifactRecord>,
    pub workspace_ids: Vec<String>,
    pub created_at: u64,
    pub discovered_count: u64,
    pub measured_count: u64,
    pub skipped_entries: u64,
    pub cancelled: bool,
    pub truncated: bool,
}

impl DeveloperArtifactInventory {
    pub fn is_fresh(&self) -> bool {
        unix_timestamp().saturating_sub(self.created_at) < INVENTORY_TTL_SECS
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    id: String,
    workspace: DeveloperWorkspaceRecord,
    project_name: String,
    ecosystem: DeveloperEcosystem,
    kind: DeveloperArtifactKind,
    path: PathBuf,
    project_root: PathBuf,
    artifact_relative: PathBuf,
    marker_paths: Vec<PathBuf>,
    evidence: Vec<String>,
    rebuild_hint: Option<String>,
}

#[derive(Debug, Clone)]
struct ArtifactMatch {
    ecosystem: DeveloperEcosystem,
    kind: DeveloperArtifactKind,
    project_root: PathBuf,
    artifact_relative: PathBuf,
    marker_paths: Vec<PathBuf>,
    evidence: Vec<String>,
    rebuild_hint: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct TreeStats {
    logical_bytes: u64,
    allocated_bytes: u64,
    file_count: u64,
    newest_mtime: Option<SystemTime>,
    complete: bool,
    cancelled: bool,
}

#[derive(Debug, Clone, Copy)]
struct ScanProgress {
    discovered_count: u64,
    measured_count: u64,
    skipped_entries: u64,
    cancelled: bool,
    truncated: bool,
}

impl TreeStats {
    fn new() -> Self {
        Self {
            complete: true,
            ..Self::default()
        }
    }
}

enum MeasurementMessage {
    Started {
        artifact_id: String,
        project_name: String,
        kind: DeveloperArtifactKind,
    },
    Finished {
        candidate: Box<Candidate>,
        stats: TreeStats,
    },
}

pub struct DeveloperArtifactScanner;

impl DeveloperArtifactScanner {
    pub fn scan<F>(
        workspace_ids: &[String],
        cancel: Arc<AtomicBool>,
        mut on_event: F,
    ) -> Result<DeveloperArtifactInventory, String>
    where
        F: FnMut(DeveloperArtifactScanEvent),
    {
        let workspaces = workspace_snapshot(workspace_ids)?;
        let scan_id = Uuid::new_v4().to_string();
        on_event(DeveloperArtifactScanEvent::Started {
            scan_id: scan_id.clone(),
            workspace_count: workspaces.len() as u64,
        });

        let mut candidates = Vec::new();
        let mut discovered_count = 0u64;
        let mut skipped_entries = 0u64;
        let mut truncated = false;

        for workspace in workspaces {
            if cancel.load(Ordering::Relaxed) {
                return Self::finish(
                    scan_id,
                    workspace_ids,
                    HashMap::new(),
                    ScanProgress {
                        discovered_count,
                        measured_count: 0,
                        skipped_entries,
                        cancelled: true,
                        truncated,
                    },
                    &mut on_event,
                );
            }

            on_event(DeveloperArtifactScanEvent::WorkspaceStarted {
                workspace: workspace.workspace.clone(),
            });

            let mut seen_paths = HashSet::new();
            if let Some(candidate) = global_go_module_candidate(&workspace, &mut seen_paths) {
                if candidates.len() < MAX_CANDIDATES {
                    on_event(DeveloperArtifactScanEvent::ProjectDiscovered {
                        workspace_id: workspace.workspace.id.clone(),
                        project_name: candidate.project_name.clone(),
                        ecosystem: candidate.ecosystem,
                    });
                    candidates.push(candidate);
                    discovered_count = discovered_count.saturating_add(1);
                } else {
                    truncated = true;
                }
            }

            discover_workspace(
                &workspace,
                &cancel,
                &mut candidates,
                &mut discovered_count,
                &mut skipped_entries,
                &mut truncated,
                &mut seen_paths,
                &mut on_event,
            );
            on_event(DeveloperArtifactScanEvent::WorkspaceFinished {
                workspace_id: workspace.workspace.id.clone(),
            });

            if cancel.load(Ordering::Relaxed) {
                return Self::finish(
                    scan_id,
                    workspace_ids,
                    HashMap::new(),
                    ScanProgress {
                        discovered_count,
                        measured_count: 0,
                        skipped_entries,
                        cancelled: true,
                        truncated,
                    },
                    &mut on_event,
                );
            }
        }

        let worker_count = std::thread::available_parallelism()
            .map(|count| count.get().min(4))
            .unwrap_or(2)
            .max(1);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_count)
            .build()
            .map_err(|error| format!("Could not create artifact scan workers: {error}"))?;

        let (sender, receiver) = mpsc::channel();
        let worker_cancel = cancel.clone();
        let worker_candidates = candidates;
        let worker = std::thread::spawn(move || {
            pool.install(|| {
                worker_candidates
                    .into_par_iter()
                    .for_each_with(sender, |tx, candidate| {
                        let _ = tx.send(MeasurementMessage::Started {
                            artifact_id: candidate.id.clone(),
                            project_name: candidate.project_name.clone(),
                            kind: candidate.kind,
                        });
                        if worker_cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        let stats = measure_tree(
                            &candidate.path,
                            candidate.workspace.identity.device,
                            &worker_cancel,
                            0,
                        );
                        let _ = tx.send(MeasurementMessage::Finished {
                            candidate: Box::new(candidate),
                            stats,
                        });
                    });
            });
        });

        let mut records = HashMap::new();
        let mut measured_count = 0u64;
        for message in receiver {
            match message {
                MeasurementMessage::Started {
                    artifact_id,
                    project_name,
                    kind,
                } => on_event(DeveloperArtifactScanEvent::ArtifactMeasurementStarted {
                    artifact_id,
                    project_name,
                    kind,
                }),
                MeasurementMessage::Finished { candidate, stats } => {
                    if stats.cancelled {
                        continue;
                    }
                    measured_count = measured_count.saturating_add(1);
                    let Some(record) = record_from_measurement(*candidate, stats) else {
                        skipped_entries = skipped_entries.saturating_add(1);
                        continue;
                    };
                    let artifact = record.artifact.clone();
                    let workspace_id = artifact.workspace_id.clone();
                    records.insert(artifact.id.clone(), record);
                    on_event(DeveloperArtifactScanEvent::ArtifactFound { artifact });
                    on_event(DeveloperArtifactScanEvent::Progress {
                        workspace_id,
                        discovered_count,
                        measured_count,
                        skipped_entries,
                    });
                }
            }
        }
        let _ = worker.join();

        let cancelled = cancel.load(Ordering::Relaxed);
        Self::finish(
            scan_id,
            workspace_ids,
            records,
            ScanProgress {
                discovered_count,
                measured_count,
                skipped_entries,
                cancelled,
                truncated,
            },
            &mut on_event,
        )
    }

    fn finish<F>(
        scan_id: String,
        workspace_ids: &[String],
        records: HashMap<String, DeveloperArtifactRecord>,
        progress: ScanProgress,
        on_event: &mut F,
    ) -> Result<DeveloperArtifactInventory, String>
    where
        F: FnMut(DeveloperArtifactScanEvent),
    {
        let inventory = DeveloperArtifactInventory {
            scan_id: scan_id.clone(),
            records,
            workspace_ids: workspace_ids.to_vec(),
            created_at: unix_timestamp(),
            discovered_count: progress.discovered_count,
            measured_count: progress.measured_count,
            skipped_entries: progress.skipped_entries,
            cancelled: progress.cancelled,
            truncated: progress.truncated,
        };
        if progress.cancelled {
            on_event(DeveloperArtifactScanEvent::Cancelled {
                scan_id: scan_id.clone(),
            });
        }
        let result = result_from_inventory(&inventory);
        on_event(DeveloperArtifactScanEvent::Finished { result });
        Ok(inventory)
    }
}

pub fn result_from_inventory(
    inventory: &DeveloperArtifactInventory,
) -> DeveloperArtifactScanResult {
    let mut items = inventory
        .records
        .values()
        .map(|record| record.artifact.clone())
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .allocated_bytes
            .cmp(&left.allocated_bytes)
            .then_with(|| right.logical_bytes.cmp(&left.logical_bytes))
            .then_with(|| left.project_name.cmp(&right.project_name))
            .then_with(|| left.path.cmp(&right.path))
    });
    DeveloperArtifactScanResult {
        scan_id: inventory.scan_id.clone(),
        items,
        discovered_count: inventory.discovered_count,
        measured_count: inventory.measured_count,
        skipped_entries: inventory.skipped_entries,
        cancelled: inventory.cancelled,
        truncated: inventory.truncated,
    }
}

pub fn workspace_snapshot(ids: &[String]) -> Result<Vec<DeveloperWorkspaceRecord>, String> {
    if ids.is_empty() {
        return Err("Select at least one workspace to scan.".to_string());
    }
    let records = WORKSPACES.lock().expect("WORKSPACES poisoned");
    let mut result = Vec::with_capacity(ids.len());
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        let record = records
            .get(id)
            .cloned()
            .ok_or_else(|| "The selected workspace is unknown. Add it again.".to_string())?;
        let current = FileIdentity::from_path(&record.path)
            .ok_or_else(|| "The selected workspace disappeared or became a symlink.".to_string())?;
        if current != record.identity {
            return Err(
                "The selected workspace changed. Add it again before scanning.".to_string(),
            );
        }
        result.push(record);
    }
    if result.is_empty() {
        return Err("Select at least one workspace to scan.".to_string());
    }
    Ok(result)
}

pub fn pick_workspace() -> Result<Option<DeveloperWorkspace>, String> {
    let Some(path) = native_pick_workspace_path()? else {
        return Ok(None);
    };
    Ok(Some(register_workspace_path(&path)?.workspace))
}

pub fn register_home_workspace() -> Result<DeveloperWorkspace, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "Could not resolve the user home directory".to_string())?;
    let canonical = fs::canonicalize(&home)
        .map_err(|_| "Could not resolve the user home directory".to_string())?;
    let metadata = fs::symlink_metadata(&canonical)
        .map_err(|_| "Could not inspect the user home directory".to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("The user home directory is not a stable directory.".to_string());
    }
    #[cfg(unix)]
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err("The user home directory must be owned by the current user.".to_string());
    }
    Ok(store_workspace(canonical, "This Mac".to_string(), true)?.workspace)
}

pub fn register_workspace_path(path: &Path) -> Result<DeveloperWorkspaceRecord, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "Could not resolve the user home directory".to_string())?;
    let canonical = validate_workspace_root(path, &home)?;
    let name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Workspace")
        .to_string();
    store_workspace(canonical, name, false)
}

fn store_workspace(
    canonical: PathBuf,
    name: String,
    whole_home: bool,
) -> Result<DeveloperWorkspaceRecord, String> {
    let identity = FileIdentity::from_path(&canonical)
        .ok_or_else(|| "The selected workspace is not a stable directory.".to_string())?;

    let mut workspaces = WORKSPACES.lock().expect("WORKSPACES poisoned");
    if let Some(existing) = workspaces.values().find(|record| record.path == canonical) {
        return Ok(existing.clone());
    }
    if workspaces.len() >= MAX_WORKSPACES {
        if let Some(oldest_id) = workspaces
            .iter()
            .min_by_key(|(_, record)| record.created_at)
            .map(|(id, _)| id.clone())
        {
            workspaces.remove(&oldest_id);
        }
    }
    let id = Uuid::new_v4().to_string();
    let record = DeveloperWorkspaceRecord {
        workspace: DeveloperWorkspace {
            id: id.clone(),
            name,
            display_path: canonical.to_string_lossy().into_owned(),
        },
        path: canonical,
        identity,
        created_at: unix_timestamp(),
        whole_home,
    };
    workspaces.insert(id, record.clone());
    Ok(record)
}

pub fn validate_workspace_root(path: &Path, home: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        "Choose an existing workspace directory inside your home folder.".to_string()
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("The selected workspace must be a real directory.".to_string());
    }
    let canonical_home = fs::canonicalize(home)
        .map_err(|_| "Could not resolve the user home directory".to_string())?;
    SymlinkGuard::validate_no_symlink_ancestors(path, &canonical_home)
        .map_err(|_| "The selected workspace contains a symbolic-link component.".to_string())?;
    let canonical = fs::canonicalize(path)
        .map_err(|_| "Could not resolve the selected workspace".to_string())?;
    if canonical == canonical_home || !canonical.starts_with(&canonical_home) {
        return Err("Workspace roots must be a child of your home directory.".to_string());
    }
    let protected_workspace_prefixes = [
        "Library",
        ".ssh",
        ".gnupg",
        ".aws",
        ".azure",
        ".kube",
        ".config",
        "Desktop",
        "Documents",
        "Pictures",
        "Movies",
        "Music",
    ];
    if Blacklist::is_blacklisted(&canonical)
        || protected_workspace_prefixes.iter().any(|prefix| {
            let protected = canonical_home.join(prefix);
            canonical == protected || canonical.starts_with(&protected)
        })
    {
        return Err("That location is protected and cannot be used as a workspace.".to_string());
    }
    #[cfg(unix)]
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err("Workspace roots must be owned by the current user.".to_string());
    }
    Ok(canonical)
}

#[allow(clippy::too_many_arguments)]
fn discover_workspace<F>(
    workspace: &DeveloperWorkspaceRecord,
    cancel: &AtomicBool,
    candidates: &mut Vec<Candidate>,
    discovered_count: &mut u64,
    skipped_entries: &mut u64,
    truncated: &mut bool,
    seen_paths: &mut HashSet<PathBuf>,
    on_event: &mut F,
) where
    F: FnMut(DeveloperArtifactScanEvent),
{
    let mut pending = VecDeque::from([(workspace.path.clone(), 0usize)]);
    let mut entries_seen = 0u64;
    while let Some((directory, depth)) = pending.pop_front() {
        if cancel.load(Ordering::Relaxed) || entries_seen >= MAX_DISCOVERY_ENTRIES {
            if entries_seen >= MAX_DISCOVERY_ENTRIES {
                *truncated = true;
            }
            return;
        }
        if depth > MAX_DISCOVERY_DEPTH {
            *skipped_entries = skipped_entries.saturating_add(1);
            continue;
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                *skipped_entries = skipped_entries.saturating_add(1);
                continue;
            }
        };
        for entry in entries {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            entries_seen = entries_seen.saturating_add(1);
            if entries_seen >= MAX_DISCOVERY_ENTRIES {
                *truncated = true;
                return;
            }
            let Ok(entry) = entry else {
                *skipped_entries = skipped_entries.saturating_add(1);
                continue;
            };
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                *skipped_entries = skipped_entries.saturating_add(1);
                continue;
            };
            if metadata.file_type().is_symlink() {
                *skipped_entries = skipped_entries.saturating_add(1);
                continue;
            }
            if !metadata.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".git" {
                continue;
            }
            if should_skip_protected_discovery_path(workspace, &path, &name) {
                *skipped_entries = skipped_entries.saturating_add(1);
                continue;
            }
            if seen_paths.contains(&path) {
                // A special candidate such as ~/go/pkg/mod may have been
                // registered before its parent directory is visited.
                continue;
            }

            if let Some(candidate) = recognize_artifact(workspace, &directory, &name) {
                if !seen_paths.insert(candidate.path.clone()) {
                    continue;
                }
                if candidates.len() >= MAX_CANDIDATES {
                    *truncated = true;
                    continue;
                }
                on_event(DeveloperArtifactScanEvent::ProjectDiscovered {
                    workspace_id: workspace.workspace.id.clone(),
                    project_name: candidate.project_name.clone(),
                    ecosystem: candidate.ecosystem,
                });
                *discovered_count = discovered_count.saturating_add(1);
                candidates.push(candidate);
                // Recognized artifact trees are measured in Phase B and are
                // never descended during discovery.
                continue;
            }

            if should_skip_discovery_directory(&name) {
                continue;
            }
            pending.push_back((path, depth + 1));
        }
    }
}

fn recognize_artifact(
    workspace: &DeveloperWorkspaceRecord,
    project_root: &Path,
    child_name: &str,
) -> Option<Candidate> {
    let artifact_match = match child_name {
        "target" => recognize_target(project_root),
        "node_modules" => recognize_node(project_root),
        ".venv" | "venv" => recognize_python(project_root, child_name),
        "vendor" => recognize_vendor(project_root),
        "build" => recognize_build(project_root, child_name),
        ".gradle" => recognize_gradle(project_root, child_name),
        "bin" | "obj" => recognize_dotnet(project_root, child_name),
        ".build" => recognize_swift(project_root, child_name),
        ".dart_tool" => recognize_flutter(project_root, child_name),
        "_build" | "deps" => recognize_elixir(project_root, child_name),
        ".terraform" => recognize_terraform(project_root, child_name),
        "pkg" => None,
        _ => None,
    }?;
    let candidate_path = project_root.join(&artifact_match.artifact_relative);
    if !candidate_path.is_dir() {
        return None;
    }
    Some(candidate_from_match(
        workspace,
        candidate_path,
        project_root,
        child_name,
        artifact_match,
    ))
}

fn global_go_module_candidate(
    workspace: &DeveloperWorkspaceRecord,
    seen_paths: &mut HashSet<PathBuf>,
) -> Option<Candidate> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let canonical_home = fs::canonicalize(&home).ok()?;
    let expected_root = fs::canonicalize(home.join("go")).ok()?;
    if workspace.path != expected_root && workspace.path != canonical_home {
        return None;
    }
    let path = expected_root.join("pkg/mod");
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || !seen_paths.insert(path.clone()) {
        return None;
    }
    Some(Candidate {
        id: Uuid::new_v4().to_string(),
        workspace: workspace.clone(),
        project_name: "Go module cache".to_string(),
        ecosystem: DeveloperEcosystem::Go,
        kind: DeveloperArtifactKind::GoModuleCache,
        path,
        project_root: expected_root,
        artifact_relative: PathBuf::from("pkg/mod"),
        marker_paths: Vec::new(),
        evidence: vec![if workspace.whole_home {
            "Built-in Scan this Mac scope".to_string()
        } else {
            "Explicitly selected ~/go workspace".to_string()
        }],
        rebuild_hint: Some("go mod download".to_string()),
    })
}

fn candidate_from_match(
    workspace: &DeveloperWorkspaceRecord,
    path: PathBuf,
    project_root: &Path,
    child_name: &str,
    artifact_match: ArtifactMatch,
) -> Candidate {
    let project_name = artifact_match
        .project_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(project_root.to_string_lossy().as_ref())
        .to_string();
    let artifact_relative = if artifact_match.artifact_relative.as_os_str().is_empty() {
        PathBuf::from(child_name)
    } else {
        artifact_match.artifact_relative
    };
    Candidate {
        id: Uuid::new_v4().to_string(),
        workspace: workspace.clone(),
        project_name,
        ecosystem: artifact_match.ecosystem,
        kind: artifact_match.kind,
        path,
        project_root: artifact_match.project_root,
        artifact_relative,
        marker_paths: artifact_match.marker_paths,
        evidence: artifact_match.evidence,
        rebuild_hint: artifact_match.rebuild_hint,
    }
}

fn recognize_target(project_root: &Path) -> Option<ArtifactMatch> {
    if let Some(marker) = find_named_marker(project_root, &["Cargo.toml"]) {
        return Some(ArtifactMatch {
            ecosystem: DeveloperEcosystem::Rust,
            kind: DeveloperArtifactKind::CargoTarget,
            project_root: project_root.to_path_buf(),
            artifact_relative: PathBuf::from("target"),
            marker_paths: vec![marker],
            evidence: vec!["Cargo.toml".to_string()],
            rebuild_hint: Some("cargo build".to_string()),
        });
    }
    let marker = find_named_marker(project_root, &["pom.xml"])?;
    Some(ArtifactMatch {
        ecosystem: DeveloperEcosystem::Java,
        kind: DeveloperArtifactKind::MavenTarget,
        project_root: project_root.to_path_buf(),
        artifact_relative: PathBuf::from("target"),
        marker_paths: vec![marker],
        evidence: vec!["pom.xml".to_string()],
        rebuild_hint: Some("mvn clean package".to_string()),
    })
}

fn recognize_node(project_root: &Path) -> Option<ArtifactMatch> {
    let package_json = find_named_marker(project_root, &["package.json"])?;
    let mut marker_paths = vec![package_json.clone()];
    let mut evidence = vec!["package.json".to_string()];
    let lockfiles = [
        ("pnpm-lock.yaml", "pnpm install"),
        ("package-lock.json", "npm ci"),
        ("yarn.lock", "yarn install"),
        ("bun.lock", "bun install"),
        ("bun.lockb", "bun install"),
    ];
    let package_root = package_json.parent()?;
    let mut hint = "npm install".to_string();
    for (lockfile, lock_hint) in lockfiles {
        let path = package_root.join(lockfile);
        if is_regular_file(&path) {
            marker_paths.push(path);
            evidence.push(lockfile.to_string());
            hint = lock_hint.to_string();
            break;
        }
    }
    Some(ArtifactMatch {
        ecosystem: DeveloperEcosystem::Node,
        kind: DeveloperArtifactKind::NodeModules,
        project_root: project_root.to_path_buf(),
        artifact_relative: PathBuf::from("node_modules"),
        marker_paths,
        evidence,
        rebuild_hint: Some(hint),
    })
}

fn recognize_python(project_root: &Path, child_name: &str) -> Option<ArtifactMatch> {
    let names = [
        "pyproject.toml",
        "uv.lock",
        "poetry.lock",
        "requirements.txt",
        "Pipfile.lock",
    ];
    let marker = find_named_marker(project_root, &names)?;
    if !is_regular_file(&project_root.join(child_name).join("pyvenv.cfg")) {
        return None;
    }
    let marker_name = marker.file_name()?.to_string_lossy();
    let hint = match marker_name.as_ref() {
        "uv.lock" => "uv sync",
        "poetry.lock" => "poetry install",
        "requirements.txt" => "python -m pip install -r requirements.txt",
        "Pipfile.lock" => "pipenv sync",
        _ => "python -m pip install -e .",
    };
    Some(ArtifactMatch {
        ecosystem: DeveloperEcosystem::Python,
        kind: DeveloperArtifactKind::PythonVenv,
        project_root: project_root.to_path_buf(),
        artifact_relative: PathBuf::from(child_name),
        marker_paths: vec![marker.clone()],
        evidence: vec![marker_name.into_owned()],
        rebuild_hint: Some(hint.to_string()),
    })
}

fn recognize_vendor(project_root: &Path) -> Option<ArtifactMatch> {
    if let Some(marker) = find_named_marker(project_root, &["composer.json"]) {
        let vendor = project_root.join("vendor");
        let installed = vendor.join("composer/installed.json");
        let autoload = vendor.join("autoload.php");
        let generated_evidence = if is_regular_file(&installed) {
            installed
        } else if is_regular_file(&autoload) {
            autoload
        } else {
            return None;
        };
        let mut marker_paths = vec![marker];
        let lock = project_root.join("composer.lock");
        let mut evidence = vec!["composer.json".to_string()];
        if is_regular_file(&lock) {
            marker_paths.push(lock);
            evidence.push("composer.lock".to_string());
        }
        marker_paths.push(generated_evidence.clone());
        evidence.push(
            generated_evidence
                .strip_prefix(project_root)
                .unwrap_or(&generated_evidence)
                .to_string_lossy()
                .into_owned(),
        );
        return Some(ArtifactMatch {
            ecosystem: DeveloperEcosystem::Php,
            kind: DeveloperArtifactKind::ComposerVendor,
            project_root: project_root.to_path_buf(),
            artifact_relative: PathBuf::from("vendor"),
            marker_paths,
            evidence,
            rebuild_hint: Some("composer install".to_string()),
        });
    }

    let marker = find_named_marker(project_root, &["Gemfile"])?;
    let bundle = project_root.join("vendor/bundle");
    if !bundle.is_dir() {
        return None;
    }
    let mut marker_paths = vec![marker];
    let lock = project_root.join("Gemfile.lock");
    let mut evidence = vec!["Gemfile".to_string()];
    if is_regular_file(&lock) {
        marker_paths.push(lock);
        evidence.push("Gemfile.lock".to_string());
    }
    Some(ArtifactMatch {
        ecosystem: DeveloperEcosystem::Ruby,
        kind: DeveloperArtifactKind::RubyBundle,
        project_root: project_root.to_path_buf(),
        artifact_relative: PathBuf::from("vendor/bundle"),
        marker_paths,
        evidence,
        rebuild_hint: Some("bundle install".to_string()),
    })
}

fn recognize_build(project_root: &Path, child_name: &str) -> Option<ArtifactMatch> {
    let gradle = find_named_marker(
        project_root,
        &[
            "build.gradle.kts",
            "build.gradle",
            "settings.gradle.kts",
            "settings.gradle",
            "gradlew",
        ],
    );
    if let Some(marker) = gradle {
        let is_kotlin = marker
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.ends_with(".kts"));
        return Some(ArtifactMatch {
            ecosystem: if is_kotlin {
                DeveloperEcosystem::Kotlin
            } else {
                DeveloperEcosystem::Java
            },
            kind: DeveloperArtifactKind::GradleBuild,
            project_root: project_root.to_path_buf(),
            artifact_relative: PathBuf::from(child_name),
            marker_paths: vec![marker.clone()],
            evidence: vec![marker.file_name()?.to_string_lossy().into_owned()],
            rebuild_hint: Some("./gradlew build".to_string()),
        });
    }
    let marker = find_named_marker(project_root, &["CMakeLists.txt"])?;
    let cache = project_root.join(child_name).join("CMakeCache.txt");
    if !is_regular_file(&cache) {
        return None;
    }
    Some(ArtifactMatch {
        ecosystem: DeveloperEcosystem::Cpp,
        kind: DeveloperArtifactKind::CMakeBuild,
        project_root: project_root.to_path_buf(),
        artifact_relative: PathBuf::from(child_name),
        marker_paths: vec![marker, cache],
        evidence: vec![
            "CMakeLists.txt".to_string(),
            "build/CMakeCache.txt".to_string(),
        ],
        rebuild_hint: Some("cmake --build build".to_string()),
    })
}

fn recognize_gradle(project_root: &Path, child_name: &str) -> Option<ArtifactMatch> {
    let marker = find_named_marker(
        project_root,
        &[
            "build.gradle.kts",
            "build.gradle",
            "settings.gradle.kts",
            "settings.gradle",
            "gradlew",
        ],
    )?;
    let marker_name = marker.file_name()?.to_string_lossy().into_owned();
    let is_kotlin = marker_name.ends_with(".kts");
    Some(ArtifactMatch {
        ecosystem: if is_kotlin {
            DeveloperEcosystem::Kotlin
        } else {
            DeveloperEcosystem::Java
        },
        kind: DeveloperArtifactKind::GradleCache,
        project_root: project_root.to_path_buf(),
        artifact_relative: PathBuf::from(child_name),
        marker_paths: vec![marker],
        evidence: vec![marker_name],
        rebuild_hint: Some("./gradlew build".to_string()),
    })
}

fn recognize_dotnet(project_root: &Path, child_name: &str) -> Option<ArtifactMatch> {
    let marker =
        find_project_extension_marker(project_root, &["csproj", "fsproj", "vbproj", "sln"])?;
    let kind = if child_name == "bin" {
        DeveloperArtifactKind::DotnetBin
    } else {
        DeveloperArtifactKind::DotnetObj
    };
    Some(ArtifactMatch {
        ecosystem: DeveloperEcosystem::Dotnet,
        kind,
        project_root: project_root.to_path_buf(),
        artifact_relative: PathBuf::from(child_name),
        marker_paths: vec![marker.clone()],
        evidence: vec![marker.file_name()?.to_string_lossy().into_owned()],
        rebuild_hint: Some("dotnet restore".to_string()),
    })
}

fn recognize_swift(project_root: &Path, child_name: &str) -> Option<ArtifactMatch> {
    let marker = find_named_marker(project_root, &["Package.swift"])?;
    Some(ArtifactMatch {
        ecosystem: DeveloperEcosystem::Swift,
        kind: DeveloperArtifactKind::SwiftBuild,
        project_root: project_root.to_path_buf(),
        artifact_relative: PathBuf::from(child_name),
        marker_paths: vec![marker],
        evidence: vec!["Package.swift".to_string()],
        rebuild_hint: Some("swift build".to_string()),
    })
}

fn recognize_flutter(project_root: &Path, child_name: &str) -> Option<ArtifactMatch> {
    let marker = find_named_marker(project_root, &["pubspec.yaml"])?;
    Some(ArtifactMatch {
        ecosystem: DeveloperEcosystem::Dart,
        kind: DeveloperArtifactKind::FlutterTooling,
        project_root: project_root.to_path_buf(),
        artifact_relative: PathBuf::from(child_name),
        marker_paths: vec![marker],
        evidence: vec!["pubspec.yaml".to_string()],
        rebuild_hint: Some("flutter pub get".to_string()),
    })
}

fn recognize_elixir(project_root: &Path, child_name: &str) -> Option<ArtifactMatch> {
    let marker = find_named_marker(project_root, &["mix.exs"])?;
    let kind = if child_name == "_build" {
        DeveloperArtifactKind::ElixirBuild
    } else {
        DeveloperArtifactKind::ElixirDeps
    };
    Some(ArtifactMatch {
        ecosystem: DeveloperEcosystem::Elixir,
        kind,
        project_root: project_root.to_path_buf(),
        artifact_relative: PathBuf::from(child_name),
        marker_paths: vec![marker],
        evidence: vec!["mix.exs".to_string()],
        rebuild_hint: Some("mix deps.get".to_string()),
    })
}

fn recognize_terraform(project_root: &Path, child_name: &str) -> Option<ArtifactMatch> {
    let marker = find_named_marker(project_root, &[".terraform.lock.hcl"])
        .or_else(|| find_project_extension_marker(project_root, &["tf"]))?;
    Some(ArtifactMatch {
        ecosystem: DeveloperEcosystem::Terraform,
        kind: DeveloperArtifactKind::TerraformCache,
        project_root: project_root.to_path_buf(),
        artifact_relative: PathBuf::from(child_name),
        marker_paths: vec![marker.clone()],
        evidence: vec![marker.file_name()?.to_string_lossy().into_owned()],
        rebuild_hint: Some("terraform init".to_string()),
    })
}

fn find_named_marker(project_root: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .map(|name| project_root.join(name))
        .find(|path| is_regular_file(path))
}

fn find_project_extension_marker(project_root: &Path, extensions: &[&str]) -> Option<PathBuf> {
    let entries = fs::read_dir(project_root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_regular_file(&path) {
            continue;
        }
        if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
        {
            return Some(path);
        }
    }
    None
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn should_skip_discovery_directory(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "target"
            | "node_modules"
            | ".venv"
            | "venv"
            | "build"
            | ".gradle"
            | "vendor"
            | "bin"
            | "obj"
            | ".build"
            | ".dart_tool"
            | "_build"
            | "deps"
            | ".terraform"
            | "dist"
            | "out"
            | ".next"
            | ".nuxt"
    )
}

fn should_skip_protected_discovery_path(
    workspace: &DeveloperWorkspaceRecord,
    path: &Path,
    name: &str,
) -> bool {
    if name.ends_with(".app") {
        return true;
    }
    if !workspace.whole_home {
        return false;
    }
    if Blacklist::is_blacklisted(path) {
        return true;
    }
    let credential_names = [
        ".ssh", ".gnupg", ".aws", ".azure", ".kube", ".config", ".Trash",
    ];
    if credential_names.contains(&name) {
        return true;
    }
    if path.parent() != Some(workspace.path.as_path()) {
        return false;
    }
    matches!(
        name,
        "Library"
            | "Desktop"
            | "Documents"
            | "Pictures"
            | "Movies"
            | "Music"
            | ".cache"
            | ".local"
            | ".cargo"
            | ".rustup"
            | ".npm"
            | ".pnpm-store"
            | ".yarn"
            | ".bun"
            | ".gradle"
            | ".m2"
            | ".ivy2"
            | ".nuget"
            | ".gem"
            | ".composer"
            | ".docker"
            | ".orbstack"
            | ".vscode"
            | ".vscode-insiders"
    )
}

fn measure_tree(path: &Path, root_device: u64, cancel: &AtomicBool, depth: usize) -> TreeStats {
    let mut stats = TreeStats::new();
    if cancel.load(Ordering::Relaxed) {
        stats.complete = false;
        stats.cancelled = true;
        return stats;
    }
    if depth > MAX_MEASUREMENT_DEPTH {
        stats.complete = false;
        return stats;
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => {
            stats.complete = false;
            return stats;
        }
    };
    #[cfg(unix)]
    if metadata.dev() != root_device {
        stats.complete = false;
        return stats;
    }
    if metadata.file_type().is_symlink() {
        stats.complete = false;
        return stats;
    }
    merge_mtime(&mut stats.newest_mtime, metadata.modified().ok());
    if metadata.is_file() {
        stats.logical_bytes = metadata.len();
        stats.allocated_bytes = allocated_bytes(&metadata);
        stats.file_count = 1;
        return stats;
    }
    if !metadata.is_dir() {
        return stats;
    }
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => {
            stats.complete = false;
            return stats;
        }
    };
    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            stats.complete = false;
            stats.cancelled = true;
            break;
        }
        let Ok(entry) = entry else {
            stats.complete = false;
            continue;
        };
        let child = entry.path();
        if child.file_name().and_then(|value| value.to_str()) == Some(".git") {
            continue;
        }
        let child_stats = measure_tree(&child, root_device, cancel, depth + 1);
        stats.logical_bytes = stats
            .logical_bytes
            .saturating_add(child_stats.logical_bytes);
        stats.allocated_bytes = stats
            .allocated_bytes
            .saturating_add(child_stats.allocated_bytes);
        stats.file_count = stats.file_count.saturating_add(child_stats.file_count);
        merge_mtime(&mut stats.newest_mtime, child_stats.newest_mtime);
        if !child_stats.complete {
            stats.complete = false;
        }
        if child_stats.cancelled {
            stats.cancelled = true;
            break;
        }
    }
    stats
}

fn record_from_measurement(
    candidate: Candidate,
    stats: TreeStats,
) -> Option<DeveloperArtifactRecord> {
    if stats.logical_bytes == 0 && stats.allocated_bytes == 0 {
        return None;
    }
    let identity = FileIdentity::from_path(&candidate.path)?;
    let project_identity = FileIdentity::from_path(&candidate.project_root)?;
    let marker_identities = candidate
        .marker_paths
        .iter()
        .filter_map(|path| FileIdentity::from_path(path).map(|identity| (path.clone(), identity)))
        .collect::<Vec<_>>();
    let complete = stats.complete && marker_identities.len() == candidate.marker_paths.len();
    let artifact = DeveloperArtifact {
        id: candidate.id,
        workspace_id: candidate.workspace.workspace.id.clone(),
        project_name: candidate.project_name,
        ecosystem: candidate.ecosystem,
        kind: candidate.kind,
        path: candidate.path.to_string_lossy().into_owned(),
        logical_bytes: stats.logical_bytes,
        allocated_bytes: stats.allocated_bytes,
        file_count: stats.file_count,
        newest_mtime: stats
            .newest_mtime
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs()),
        rebuild_hint: candidate.rebuild_hint,
        evidence: candidate.evidence,
        complete,
        incomplete_reason: (!complete).then(|| {
            "Some entries or project markers could not be verified during the scan.".to_string()
        }),
        selected_by_default: false,
    };
    Some(DeveloperArtifactRecord {
        artifact,
        path: candidate.path,
        identity,
        workspace_path: candidate.workspace.path,
        workspace_identity: candidate.workspace.identity,
        project_root: candidate.project_root,
        project_identity,
        artifact_relative: candidate.artifact_relative,
        marker_identities,
    })
}

fn merge_mtime(current: &mut Option<SystemTime>, candidate: Option<SystemTime>) {
    if let Some(candidate) = candidate {
        *current = Some(match *current {
            Some(existing) => existing.max(candidate),
            None => candidate,
        });
    }
}

fn allocated_bytes(metadata: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        metadata.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        metadata.len()
    }
}

fn native_pick_workspace_path() -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    {
        let script = r#"
            try
                set selectedFolder to choose folder with prompt "Choose a developer workspace"
                return POSIX path of selectedFolder
            on error number -128
                return ""
            end try
        "#;
        let output = std::process::Command::new("osascript")
            .args(["-e", script])
            .output()
            .map_err(|error| format!("Could not open the workspace picker: {error}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() {
            return Ok(None);
        }
        Ok(Some(PathBuf::from(path)))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Developer workspace selection is currently available on macOS only".to_string())
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
    use std::fs;

    fn workspace_record(root: &Path) -> DeveloperWorkspaceRecord {
        let identity = FileIdentity::from_path(root).unwrap();
        DeveloperWorkspaceRecord {
            workspace: DeveloperWorkspace {
                id: Uuid::new_v4().to_string(),
                name: root.file_name().unwrap().to_string_lossy().into_owned(),
                display_path: root.to_string_lossy().into_owned(),
            },
            path: root.to_path_buf(),
            identity,
            created_at: unix_timestamp(),
            whole_home: false,
        }
    }

    #[test]
    fn recognizes_ecosystem_markers_without_age_gates() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("target")).unwrap();
        fs::write(project.join("Cargo.toml"), "[package]\nname='demo'\n").unwrap();
        let workspace = workspace_record(temp.path());
        let candidate = recognize_artifact(&workspace, &project, "target").unwrap();
        assert_eq!(candidate.kind, DeveloperArtifactKind::CargoTarget);
        assert_eq!(candidate.ecosystem, DeveloperEcosystem::Rust);
    }

    #[test]
    fn recognizes_java_kotlin_php_dotnet_and_script_markers() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = workspace_record(temp.path());

        let gradle = temp.path().join("kotlin");
        fs::create_dir_all(gradle.join("build")).unwrap();
        fs::write(gradle.join("build.gradle.kts"), "plugins {}\n").unwrap();
        assert_eq!(
            recognize_artifact(&workspace, &gradle, "build")
                .unwrap()
                .ecosystem,
            DeveloperEcosystem::Kotlin
        );

        let php = temp.path().join("php");
        fs::create_dir_all(php.join("vendor")).unwrap();
        fs::write(php.join("composer.json"), "{}\n").unwrap();
        fs::write(php.join("vendor/autoload.php"), "<?php\n").unwrap();
        assert_eq!(
            recognize_artifact(&workspace, &php, "vendor").unwrap().kind,
            DeveloperArtifactKind::ComposerVendor
        );

        let dotnet = temp.path().join("dotnet");
        fs::create_dir_all(dotnet.join("bin")).unwrap();
        fs::write(dotnet.join("demo.csproj"), "<Project />\n").unwrap();
        assert_eq!(
            recognize_artifact(&workspace, &dotnet, "bin")
                .unwrap()
                .ecosystem,
            DeveloperEcosystem::Dotnet
        );
    }

    #[test]
    fn scanner_streams_measured_candidates_from_an_explicit_workspace() {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let temp = tempfile::tempdir_in(&home).unwrap();
        let workspace_path = temp.path().join("workspace");
        let rust_project = workspace_path.join("rust-app");
        let node_project = workspace_path.join("web-app");
        fs::create_dir_all(rust_project.join("target")).unwrap();
        fs::create_dir_all(node_project.join("node_modules")).unwrap();
        fs::write(rust_project.join("Cargo.toml"), "[package]\nname='demo'\n").unwrap();
        fs::write(rust_project.join("target/output.bin"), [1u8]).unwrap();
        fs::write(node_project.join("package.json"), "{}\n").unwrap();
        fs::write(node_project.join("node_modules/package.json"), "{}\n").unwrap();
        let workspace = workspace_record(&workspace_path);
        WORKSPACES
            .lock()
            .expect("WORKSPACES poisoned")
            .insert(workspace.workspace.id.clone(), workspace.clone());
        let mut events = Vec::new();
        let inventory = DeveloperArtifactScanner::scan(
            std::slice::from_ref(&workspace.workspace.id),
            Arc::new(AtomicBool::new(false)),
            |event| events.push(event),
        )
        .unwrap();
        WORKSPACES
            .lock()
            .expect("WORKSPACES poisoned")
            .remove(&workspace.workspace.id);

        assert_eq!(inventory.records.len(), 2);
        assert!(events
            .iter()
            .any(|event| matches!(event, DeveloperArtifactScanEvent::ArtifactFound { .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event, DeveloperArtifactScanEvent::Finished { .. })));
    }

    #[test]
    fn whole_home_scan_finds_projects_and_bypasses_protected_trees() {
        let temp = tempfile::tempdir().unwrap();
        let safe_project = temp.path().join("work/rust-app");
        fs::create_dir_all(safe_project.join("target")).unwrap();
        fs::write(safe_project.join("Cargo.toml"), "[package]\nname='safe'\n").unwrap();
        fs::write(safe_project.join("target/output.bin"), [1u8]).unwrap();

        for protected in ["Library", ".ssh"] {
            let project = temp.path().join(protected).join("hidden-project");
            fs::create_dir_all(project.join("target")).unwrap();
            fs::write(project.join("Cargo.toml"), "[package]\nname='hidden'\n").unwrap();
            fs::write(project.join("target/output.bin"), [1u8]).unwrap();
        }
        let app = temp
            .path()
            .join("Applications/Demo.app/Contents/Resources/app");
        fs::create_dir_all(app.join("node_modules")).unwrap();
        fs::write(app.join("package.json"), "{}\n").unwrap();
        fs::write(app.join("node_modules/package.json"), "{}\n").unwrap();

        let mut workspace = workspace_record(temp.path());
        workspace.whole_home = true;
        WORKSPACES
            .lock()
            .expect("WORKSPACES poisoned")
            .insert(workspace.workspace.id.clone(), workspace.clone());
        let inventory = DeveloperArtifactScanner::scan(
            std::slice::from_ref(&workspace.workspace.id),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        WORKSPACES
            .lock()
            .expect("WORKSPACES poisoned")
            .remove(&workspace.workspace.id);

        assert_eq!(inventory.records.len(), 1);
        let artifact = &inventory.records.values().next().unwrap().artifact;
        assert_eq!(artifact.project_name, "rust-app");
        assert_eq!(artifact.kind, DeveloperArtifactKind::CargoTarget);
        assert!(inventory.skipped_entries >= 3);
    }

    #[test]
    fn same_named_directories_without_evidence_are_not_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = workspace_record(temp.path());
        let project = temp.path().join("random");
        fs::create_dir_all(project.join("build")).unwrap();
        fs::create_dir_all(project.join("vendor")).unwrap();
        assert!(recognize_artifact(&workspace, &project, "build").is_none());
        assert!(recognize_artifact(&workspace, &project, "vendor").is_none());
    }

    #[test]
    fn ancestor_markers_do_not_authorize_nested_same_named_source_directories() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = workspace_record(temp.path());
        fs::write(temp.path().join("Cargo.toml"), "[package]\nname='demo'\n").unwrap();
        let source = temp.path().join("src");
        fs::create_dir_all(source.join("target")).unwrap();
        fs::write(source.join("target/module.rs"), "pub fn keep_me() {}\n").unwrap();

        assert!(recognize_artifact(&workspace, &source, "target").is_none());
    }

    #[test]
    fn ambiguous_environment_and_build_names_need_generated_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = workspace_record(temp.path());

        let python = temp.path().join("python");
        fs::create_dir_all(python.join("venv")).unwrap();
        fs::write(python.join("pyproject.toml"), "[project]\nname='demo'\n").unwrap();
        assert!(recognize_artifact(&workspace, &python, "venv").is_none());
        fs::write(python.join("venv/pyvenv.cfg"), "home = /usr/bin\n").unwrap();
        assert!(recognize_artifact(&workspace, &python, "venv").is_some());

        let php = temp.path().join("php");
        fs::create_dir_all(php.join("vendor")).unwrap();
        fs::write(php.join("composer.json"), "{}\n").unwrap();
        assert!(recognize_artifact(&workspace, &php, "vendor").is_none());
        fs::write(php.join("vendor/autoload.php"), "<?php\n").unwrap();
        assert!(recognize_artifact(&workspace, &php, "vendor").is_some());

        let cmake = temp.path().join("cmake");
        fs::create_dir_all(cmake.join("build")).unwrap();
        fs::write(cmake.join("CMakeLists.txt"), "project(demo)\n").unwrap();
        assert!(recognize_artifact(&workspace, &cmake, "build").is_none());
        fs::write(cmake.join("build/CMakeCache.txt"), "# generated\n").unwrap();
        assert!(recognize_artifact(&workspace, &cmake, "build").is_some());
    }

    #[test]
    fn zero_byte_artifact_directories_are_not_reported() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("target")).unwrap();
        fs::write(project.join("Cargo.toml"), "[package]\nname='demo'\n").unwrap();
        let workspace = workspace_record(temp.path());
        let candidate = recognize_artifact(&workspace, &project, "target").unwrap();
        let stats = measure_tree(
            &candidate.path,
            workspace.identity.device,
            &AtomicBool::new(false),
            0,
        );

        assert!(record_from_measurement(candidate, stats).is_none());
    }

    #[test]
    fn measurement_collects_size_count_and_newest_mtime_in_one_pass() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.bin");
        fs::write(&file, [1u8, 2, 3, 4]).unwrap();
        let device = FileIdentity::from_path(temp.path()).unwrap().device;
        let stats = measure_tree(temp.path(), device, &AtomicBool::new(false), 0);
        assert!(stats.complete);
        assert_eq!(stats.file_count, 1);
        assert_eq!(stats.logical_bytes, 4);
        assert!(stats.newest_mtime.is_some());
    }

    #[test]
    fn workspace_validation_requires_a_real_child_of_the_selected_home() {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let temp = tempfile::tempdir_in(&home).unwrap();
        let workspace = temp.path().join("src");
        fs::create_dir_all(&workspace).unwrap();
        assert_eq!(
            validate_workspace_root(&workspace, &home).unwrap(),
            workspace
        );
        assert!(validate_workspace_root(&home, &home).is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let linked = temp.path().join("linked");
            symlink(&workspace, &linked).unwrap();
            assert!(validate_workspace_root(&linked, &home).is_err());
        }
    }

    #[test]
    fn backend_owned_home_scope_registers_without_the_folder_picker() {
        let workspace = register_home_workspace().unwrap();
        let canonical_home = fs::canonicalize(std::env::var_os("HOME").unwrap()).unwrap();
        let mut workspaces = WORKSPACES.lock().expect("WORKSPACES poisoned");
        let record = workspaces.get(&workspace.id).unwrap();

        assert_eq!(record.path, canonical_home);
        assert!(record.whole_home);
        assert_eq!(workspace.name, "This Mac");
        workspaces.remove(&workspace.id);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_artifact_content_is_not_followed_and_is_incomplete() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), [1u8]).unwrap();
        symlink(outside.path(), temp.path().join("linked")).unwrap();
        let device = FileIdentity::from_path(temp.path()).unwrap().device;
        let stats = measure_tree(temp.path(), device, &AtomicBool::new(false), 0);
        assert!(!stats.complete);
        assert_eq!(stats.file_count, 0);
    }
}
