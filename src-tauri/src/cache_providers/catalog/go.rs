use crate::models::{CacheArtifactKind, CleanerFamily};

use super::ProviderSpec;

pub(super) const BUILD: ProviderSpec = ProviderSpec {
    signature_id: "dev.go.build",
    executable: "go",
    discovery_args: &["env", "GOCACHE"],
    prune_args: &["clean", "-cache"],
    display_name: "Go Build Cache",
    consequence: "Go packages compile again on demand.",
    artifact_kind: CacheArtifactKind::BuildArtifact,
    family: CleanerFamily::Developer,
    local_toolchain_only: true,
};

pub(super) const MODULE: ProviderSpec = ProviderSpec {
    signature_id: "dev.go.mod",
    executable: "go",
    discovery_args: &["env", "GOMODCACHE"],
    prune_args: &["clean", "-modcache"],
    display_name: "Go Module Cache",
    consequence: "Modules may need to be downloaded again.",
    artifact_kind: CacheArtifactKind::DownloadCache,
    family: CleanerFamily::PackageManagers,
    local_toolchain_only: true,
};
