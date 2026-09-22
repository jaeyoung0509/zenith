use crate::models::{CacheArtifactKind, CleanerFamily};

use super::{DiscoveryOutput, ProviderSpec};

const ACTIVE_PROCESSES: &[&str] = &["dotnet", "msbuild", "devenv", "nuget"];

pub(super) const HTTP: ProviderSpec = ProviderSpec {
    signature_id: "dev.nuget.http",
    executable: "dotnet",
    discovery_args: &[
        "nuget",
        "locals",
        "http-cache",
        "--list",
        "--force-english-output",
    ],
    prune_args: &[
        "nuget",
        "locals",
        "http-cache",
        "--clear",
        "--force-english-output",
    ],
    display_name: "NuGet HTTP Cache",
    consequence: "Package metadata and archives may be downloaded again during restore.",
    artifact_kind: CacheArtifactKind::DownloadCache,
    family: CleanerFamily::PackageManagers,
    discovery_output: DiscoveryOutput::LabeledPath("http-cache"),
    active_processes: ACTIVE_PROCESSES,
    runtime_dependencies: &[],
    local_toolchain_only: false,
};

pub(super) const TEMP: ProviderSpec = ProviderSpec {
    signature_id: "dev.nuget.temp",
    executable: "dotnet",
    discovery_args: &[
        "nuget",
        "locals",
        "temp",
        "--list",
        "--force-english-output",
    ],
    prune_args: &[
        "nuget",
        "locals",
        "temp",
        "--clear",
        "--force-english-output",
    ],
    display_name: "NuGet Temporary Cache",
    consequence: "Interrupted package operations may need to recreate temporary files.",
    artifact_kind: CacheArtifactKind::Temporary,
    family: CleanerFamily::PackageManagers,
    discovery_output: DiscoveryOutput::LabeledPath("temp"),
    active_processes: ACTIVE_PROCESSES,
    runtime_dependencies: &[],
    local_toolchain_only: false,
};

pub(super) const PLUGINS: ProviderSpec = ProviderSpec {
    signature_id: "dev.nuget.plugins",
    executable: "dotnet",
    discovery_args: &[
        "nuget",
        "locals",
        "plugins-cache",
        "--list",
        "--force-english-output",
    ],
    prune_args: &[
        "nuget",
        "locals",
        "plugins-cache",
        "--clear",
        "--force-english-output",
    ],
    display_name: "NuGet Plugins Cache",
    consequence: "NuGet credential and transport plugins may need to initialize again.",
    artifact_kind: CacheArtifactKind::DownloadCache,
    family: CleanerFamily::PackageManagers,
    discovery_output: DiscoveryOutput::LabeledPath("plugins-cache"),
    active_processes: ACTIVE_PROCESSES,
    runtime_dependencies: &[],
    local_toolchain_only: false,
};

pub(super) const GLOBAL_PACKAGES: ProviderSpec = ProviderSpec {
    signature_id: "dev.nuget.global_packages",
    executable: "dotnet",
    discovery_args: &[
        "nuget",
        "locals",
        "global-packages",
        "--list",
        "--force-english-output",
    ],
    prune_args: &[
        "nuget",
        "locals",
        "global-packages",
        "--clear",
        "--force-english-output",
    ],
    display_name: "NuGet Global Packages",
    consequence: "Projects must restore all referenced packages before they can build again.",
    artifact_kind: CacheArtifactKind::PackageStore,
    family: CleanerFamily::PackageManagers,
    discovery_output: DiscoveryOutput::LabeledPath("global-packages"),
    active_processes: ACTIVE_PROCESSES,
    runtime_dependencies: &[],
    local_toolchain_only: false,
};
