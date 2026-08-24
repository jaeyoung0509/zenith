use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DeveloperWorkspace {
    pub id: String,
    pub name: String,
    pub display_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DeveloperEcosystem {
    Rust,
    Node,
    Python,
    Go,
    Java,
    Kotlin,
    Php,
    Ruby,
    Dotnet,
    Cpp,
    Swift,
    Dart,
    Elixir,
    Terraform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DeveloperArtifactKind {
    CargoTarget,
    NodeModules,
    PythonVenv,
    GoModuleCache,
    GoBuild,
    MavenTarget,
    GradleBuild,
    GradleCache,
    ComposerVendor,
    RubyBundle,
    DotnetBin,
    DotnetObj,
    CMakeBuild,
    SwiftBuild,
    FlutterTooling,
    ElixirBuild,
    ElixirDeps,
    TerraformCache,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DeveloperArtifact {
    pub id: String,
    pub workspace_id: String,
    pub project_name: String,
    pub ecosystem: DeveloperEcosystem,
    pub kind: DeveloperArtifactKind,
    pub path: String,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub file_count: u64,
    pub newest_mtime: Option<u64>,
    pub rebuild_hint: Option<String>,
    pub evidence: Vec<String>,
    pub complete: bool,
    pub incomplete_reason: Option<String>,
    pub selected_by_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DeveloperArtifactScanResult {
    pub scan_id: String,
    pub items: Vec<DeveloperArtifact>,
    pub discovered_count: u64,
    pub measured_count: u64,
    pub skipped_entries: u64,
    pub cancelled: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeveloperArtifactScanEvent {
    Started {
        scan_id: String,
        workspace_count: u64,
    },
    WorkspaceStarted {
        workspace: DeveloperWorkspace,
    },
    ProjectDiscovered {
        workspace_id: String,
        project_name: String,
        ecosystem: DeveloperEcosystem,
    },
    ArtifactMeasurementStarted {
        artifact_id: String,
        project_name: String,
        kind: DeveloperArtifactKind,
    },
    ArtifactFound {
        artifact: DeveloperArtifact,
    },
    Progress {
        workspace_id: String,
        discovered_count: u64,
        measured_count: u64,
        skipped_entries: u64,
    },
    WorkspaceFinished {
        workspace_id: String,
    },
    Finished {
        result: DeveloperArtifactScanResult,
    },
    Cancelled {
        scan_id: String,
    },
}
