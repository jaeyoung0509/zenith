use crate::models::{CacheArtifactKind, CleanerFamily};

use super::{DiscoveryOutput, ProviderSpec};

pub(super) const PIP: ProviderSpec = ProviderSpec {
    signature_id: "dev.pip.cache",
    executable: "pip3",
    discovery_args: &["cache", "dir"],
    prune_args: &["cache", "purge"],
    display_name: "pip Download and Wheel Cache",
    consequence: "Python packages and locally built wheels may need to be downloaded or rebuilt.",
    artifact_kind: CacheArtifactKind::DownloadCache,
    family: CleanerFamily::PackageManagers,
    discovery_output: DiscoveryOutput::BarePath,
    active_processes: &["pip", "pip3", "python", "python3"],
    runtime_dependencies: &[],
    local_toolchain_only: false,
};

pub(super) const UV: ProviderSpec = ProviderSpec {
    signature_id: "dev.uv.cache",
    executable: "uv",
    discovery_args: &["cache", "dir"],
    prune_args: &["cache", "prune"],
    display_name: "uv Package Cache",
    consequence: "Unused archives are pruned; future environments may re-download packages.",
    artifact_kind: CacheArtifactKind::PackageStore,
    family: CleanerFamily::PackageManagers,
    discovery_output: DiscoveryOutput::BarePath,
    // uv serializes cache-modifying commands with its cache lock. Guarding all
    // Python processes rejects unrelated applications and duplicates uv's
    // owner-level coordination; the prune command itself waits for that lock.
    active_processes: &[],
    runtime_dependencies: &[],
    local_toolchain_only: false,
};
