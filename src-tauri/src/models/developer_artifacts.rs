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
    C,
    Cpp,
    Swift,
    Dart,
    Elixir,
    Erlang,
    Scala,
    Clojure,
    Haskell,
    Zig,
    ObjectiveC,
    R,
    Julia,
    Lua,
    Terraform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DeveloperArtifactKind {
    CargoTarget,
    NodeModules,
    PythonVenv,
    GoModuleCache,
    MavenTarget,
    SbtTarget,
    ClojureTarget,
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
    ErlangBuild,
    HaskellStackWork,
    HaskellDistNewstyle,
    ZigCache,
    TerraformCache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DeveloperArtifactStatus {
    /// Every measured entry and project marker was verified.
    Complete,
    /// The generated-folder scope and project evidence are verified, but one
    /// or more descendants could not be measured. Manual cleanup is allowed
    /// after an explicit warning and execution-time revalidation.
    MeasurementIncomplete,
    /// A safety boundary (for example a symlink, filesystem boundary, or
    /// project marker identity) could not be verified. Cleanup is forbidden.
    SafetyBlocked,
    /// The scan was cancelled before this artifact could be fully validated.
    ScanCancelled,
}

impl DeveloperArtifactStatus {
    pub fn allows_manual_cleanup(self) -> bool {
        matches!(self, Self::Complete | Self::MeasurementIncomplete)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DeveloperArtifact {
    pub id: String,
    pub workspace_id: String,
    pub project_name: String,
    pub ecosystem: DeveloperEcosystem,
    pub kind: DeveloperArtifactKind,
    pub path: String,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub logical_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub allocated_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub file_count: u64,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub newest_mtime: Option<u64>,
    pub rebuild_hint: Option<String>,
    pub evidence: Vec<String>,
    pub status: DeveloperArtifactStatus,
    pub incomplete_reason: Option<String>,
    pub selected_by_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DeveloperArtifactScanResult {
    pub scan_id: String,
    pub items: Vec<DeveloperArtifact>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub discovered_count: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub measured_count: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entries: u64,
    pub cancelled: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeveloperArtifactScanEvent {
    Started {
        scan_id: String,
        #[serde(with = "crate::ipc_numeric::u64")]
        #[specta(type = u64)]
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
        #[serde(with = "crate::ipc_numeric::u64")]
        #[specta(type = u64)]
        discovered_count: u64,
        #[serde(with = "crate::ipc_numeric::u64")]
        #[specta(type = u64)]
        measured_count: u64,
        #[serde(with = "crate::ipc_numeric::u64")]
        #[specta(type = u64)]
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
