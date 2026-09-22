use crate::models::{CacheArtifactKind, CleanerFamily};

use super::ProviderSpec;

pub(super) const UV: ProviderSpec = ProviderSpec {
    signature_id: "dev.uv.cache",
    executable: "uv",
    discovery_args: &["cache", "dir"],
    prune_args: &["cache", "prune"],
    display_name: "uv Package Cache",
    consequence: "Unused archives are pruned; future environments may re-download packages.",
    artifact_kind: CacheArtifactKind::PackageStore,
    family: CleanerFamily::PackageManagers,
    local_toolchain_only: false,
};
