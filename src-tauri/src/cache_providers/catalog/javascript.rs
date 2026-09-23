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
    // Match the owner CLI only. Every unrelated Node app can run without
    // holding this store's pruning operation open.
    active_processes: &["pnpm", "pnpm-cli"],
    runtime_dependencies: &["node"],
    local_toolchain_only: false,
};

pub(super) const NPM: ProviderSpec = ProviderSpec {
    signature_id: "dev.npm.cache",
    executable: "npm",
    discovery_args: &["config", "get", "cache"],
    prune_args: &["cache", "verify"],
    display_name: "npm Cache",
    consequence: "npm verifies the cache and garbage-collects unneeded entries; packages may need to be downloaded again.",
    artifact_kind: CacheArtifactKind::PackageStore,
    family: CleanerFamily::PackageManagers,
    discovery_output: DiscoveryOutput::BarePath,
    // npm's CLI is a Node process, so recognizing its entrypoint avoids
    // treating an unrelated Node app as an active npm cache owner.
    active_processes: &["npm", "npm-cli"],
    runtime_dependencies: &["node"],
    local_toolchain_only: false,
};
