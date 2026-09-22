use crate::models::{CacheArtifactKind, CleanerFamily};

use super::{DiscoveryOutput, ProviderSpec};

pub(super) const COMPOSER: ProviderSpec = ProviderSpec {
    signature_id: "dev.composer.cache",
    executable: "composer",
    // Plugins are disabled during inspection and cleanup so a cache operation
    // cannot execute project- or user-supplied plugin code.
    discovery_args: &[
        "--no-interaction",
        "--no-plugins",
        "config",
        "--global",
        "cache-dir",
        "--absolute",
    ],
    prune_args: &["--no-interaction", "--no-plugins", "clear-cache"],
    display_name: "Composer Cache",
    consequence: "PHP packages and repository metadata may need to be downloaded again.",
    artifact_kind: CacheArtifactKind::PackageStore,
    family: CleanerFamily::PackageManagers,
    discovery_output: DiscoveryOutput::BarePath,
    active_processes: &["composer", "php"],
    runtime_dependencies: &["php"],
    local_toolchain_only: false,
};
