//! Owner-focused definitions for command-backed cache cleaners.
//!
//! The registry owns orchestration and safety. Each family module owns only
//! the fixed executable contract for the tool it describes, so adding one
//! package manager cannot silently alter another manager's command or recovery
//! metadata.

mod go;
mod javascript;
mod php;
mod python;

use crate::models::{CacheArtifactKind, CleanerFamily};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ProviderKind {
    GoBuild,
    GoModule,
    Uv,
    Pnpm,
    Npm,
    Bun,
    Composer,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ProviderSpec {
    pub signature_id: &'static str,
    pub executable: &'static str,
    pub discovery_args: &'static [&'static str],
    pub prune_args: &'static [&'static str],
    pub display_name: &'static str,
    pub consequence: &'static str,
    pub artifact_kind: CacheArtifactKind,
    pub family: CleanerFamily,
    /// Go's automatic toolchain lookup can download a toolchain while merely
    /// inspecting a cache. Providers that set this require a local toolchain.
    pub local_toolchain_only: bool,
}

impl ProviderKind {
    pub(super) const ALL: [Self; 7] = [
        Self::GoBuild,
        Self::GoModule,
        Self::Uv,
        Self::Pnpm,
        Self::Npm,
        Self::Bun,
        Self::Composer,
    ];

    pub(super) fn for_signature(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.signature_id() == id)
    }

    pub(super) fn spec(self) -> &'static ProviderSpec {
        match self {
            Self::GoBuild => &go::BUILD,
            Self::GoModule => &go::MODULE,
            Self::Uv => &python::UV,
            Self::Pnpm => &javascript::PNPM,
            Self::Npm => &javascript::NPM,
            Self::Bun => &javascript::BUN,
            Self::Composer => &php::COMPOSER,
        }
    }

    pub(super) fn signature_id(self) -> &'static str {
        self.spec().signature_id
    }

    pub(super) fn executable(self) -> &'static str {
        self.spec().executable
    }

    pub(super) fn discovery_args(self) -> &'static [&'static str] {
        self.spec().discovery_args
    }

    pub(super) fn prune_args(self) -> &'static [&'static str] {
        self.spec().prune_args
    }

    pub(super) fn display_name(self) -> &'static str {
        self.spec().display_name
    }

    pub(super) fn consequence(self) -> &'static str {
        self.spec().consequence
    }

    pub(super) fn artifact_kind(self) -> CacheArtifactKind {
        self.spec().artifact_kind
    }

    pub(super) fn family(self) -> CleanerFamily {
        self.spec().family
    }

    pub(super) fn local_toolchain_only(self) -> bool {
        self.spec().local_toolchain_only
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderKind;

    #[test]
    fn every_signature_resolves_to_exactly_one_owner_spec() {
        for provider in ProviderKind::ALL {
            assert_eq!(
                ProviderKind::for_signature(provider.signature_id()),
                Some(provider)
            );
            assert!(!provider.executable().is_empty());
            assert!(!provider.discovery_args().is_empty());
            assert!(!provider.prune_args().is_empty());
            assert_ne!(
                provider.family(),
                crate::models::CleanerFamily::Unclassified
            );
        }
    }
}
