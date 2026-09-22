use crate::models::{CacheArtifactKind, CleanerFamily};

use super::{DiscoveryOutput, ProviderSpec};

pub(super) const PNPM: ProviderSpec = ProviderSpec {
    signature_id: "dev.pnpm.store",
    executable: "pnpm",
    discovery_args: &["store", "path"],
    prune_args: &["store", "prune"],
    display_name: "pnpm Content-Addressable Store",
    consequence: "Unreferenced packages are pruned; future installs may download them again.",
    artifact_kind: CacheArtifactKind::PackageStore,
    family: CleanerFamily::PackageManagers,
    discovery_output: DiscoveryOutput::BarePath,
    active_processes: &["pnpm", "node"],
    runtime_dependencies: &["node"],
    local_toolchain_only: false,
};

pub(super) const NPM: ProviderSpec = ProviderSpec {
    signature_id: "dev.npm.cache",
    executable: "npm",
    discovery_args: &["config", "get", "cache"],
    prune_args: &["cache", "clean", "--force"],
    display_name: "npm Cache",
    consequence: "A full cleanup can force package downloads on later installs.",
    artifact_kind: CacheArtifactKind::PackageStore,
    family: CleanerFamily::PackageManagers,
    discovery_output: DiscoveryOutput::BarePath,
    active_processes: &["npm", "node"],
    runtime_dependencies: &["node"],
    local_toolchain_only: false,
};
