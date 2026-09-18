//! What a planned target actually authorizes.
//!
//! A [`DeleteTarget`] carries a path, a strategy, and a signature ID because
//! those are what the scan and the planner observe. That does not mean every
//! strategy may touch the path: a container prune runs through the runtime's
//! own CLI, and a provider prune runs through the tool's own fixed arguments
//! after confirming the location the plan was built against still matches what
//! the provider resolves now.
//!
//! This module makes the distinction structural instead of leaving it to the
//! order of a match. [`FilesystemCleanup`] is the only variant that names a
//! path as mutation authority; the container and provider variants carry the
//! signature ID — and, for a provider, the reviewed location purely as a
//! staleness assertion — so a pseudo path such as `docker://images` cannot
//! reach a filesystem primitive by being classified with the wrong strategy.
//!
//! [`DeleteTarget`]: super::DeleteTarget

use std::path::Path;

use super::{CleanStrategy, DeleteTarget};

/// The operation a planned target authorizes.
///
/// Built by [`CleanupOperation::of`], which is the single classification of a
/// strategy into an operation. A target whose strategy authorizes nothing
/// yields `None`, and a caller that receives `None` refuses the target rather
/// than falling back to a filesystem mutation.
#[derive(Debug)]
pub enum CleanupOperation<'a> {
    Filesystem(FilesystemCleanup<'a>),
    Container(ContainerCleanup<'a>),
    Provider(ProviderCleanup<'a>),
}

impl<'a> CleanupOperation<'a> {
    /// Classifies a planned target by what it authorizes.
    ///
    /// `Manual` is not an operation: a resource that needs a dedicated adapter
    /// never becomes a generic mutation, so it classifies to `None`.
    pub fn of(target: &'a DeleteTarget) -> Option<Self> {
        match target.strategy {
            CleanStrategy::DeleteContents => Some(Self::Filesystem(FilesystemCleanup {
                target,
                removes: FilesystemMutation::Contents,
            })),
            CleanStrategy::DeleteDirectory => Some(Self::Filesystem(FilesystemCleanup {
                target,
                removes: FilesystemMutation::Directory,
            })),
            CleanStrategy::DeleteStaleContents => Some(Self::Filesystem(FilesystemCleanup {
                target,
                removes: FilesystemMutation::StaleContents,
            })),
            CleanStrategy::DockerPrune => Some(Self::Container(ContainerCleanup {
                signature_id: &target.signature_id,
            })),
            CleanStrategy::ExternalCommand => Some(Self::Provider(ProviderCleanup {
                signature_id: &target.signature_id,
                expected_location: &target.path,
            })),
            CleanStrategy::Manual => None,
        }
    }
}

/// Which part of a reviewed path a filesystem mutation removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemMutation {
    /// The path's children are removed; the directory itself stays.
    Contents,
    /// The directory and everything under it is removed.
    Directory,
    /// The path's children whose own age satisfies the target's age policy are
    /// removed; the rest and the directory itself stay.
    StaleContents,
}

/// A signature-scoped filesystem mutation.
///
/// This is the only operation that carries a path as mutation authority, and
/// the authority is not the path alone: execution revalidates the target
/// through the safety layer (presence, lexical and canonical blacklist,
/// symlink integrity, captured identity, and any age constraint) immediately
/// before the primitive runs.
#[derive(Debug)]
pub struct FilesystemCleanup<'a> {
    target: &'a DeleteTarget,
    removes: FilesystemMutation,
}

impl<'a> FilesystemCleanup<'a> {
    /// The planned target this mutation was classified from.
    pub fn target(&self) -> &'a DeleteTarget {
        self.target
    }

    /// The reviewed path the mutation applies to.
    pub fn path(&self) -> &'a Path {
        &self.target.path
    }

    /// Which part of the path is removed.
    pub fn removes(&self) -> FilesystemMutation {
        self.removes
    }
}

/// A container-runtime prune.
///
/// The runtime prunes the resources it owns; there is no path to delete, and a
/// pseudo path in the plan is presentation, never authority.
#[derive(Debug)]
pub struct ContainerCleanup<'a> {
    signature_id: &'a str,
}

impl<'a> ContainerCleanup<'a> {
    /// The signature whose declared scope the runtime prunes.
    pub fn signature_id(&self) -> &'a str {
        self.signature_id
    }
}

/// A provider-owned cache invalidation.
///
/// The provider runs its own prune command with fixed arguments. The location
/// travels with the operation only so execution can reject a plan whose
/// location no longer matches what the provider resolves today; it is never
/// passed to a delete primitive.
#[derive(Debug)]
pub struct ProviderCleanup<'a> {
    signature_id: &'a str,
    expected_location: &'a Path,
}

impl<'a> ProviderCleanup<'a> {
    /// The signature that names the provider.
    pub fn signature_id(&self) -> &'a str {
        self.signature_id
    }

    /// The location the plan was built against, checked for staleness only.
    pub fn expected_location(&self) -> &'a Path {
        self.expected_location
    }
}

#[cfg(test)]
mod tests {
    use super::{CleanupOperation, FilesystemMutation};
    use crate::domain::cleanup::{CleanStrategy, DeleteTarget};
    use crate::domain::risk::RiskTier;

    fn target(strategy: CleanStrategy) -> DeleteTarget {
        DeleteTarget {
            item_id: "item".to_string(),
            signature_id: "dev.example.cache".to_string(),
            name: "Example cache".to_string(),
            path: std::path::PathBuf::from("/Users/tester/.cache/example"),
            strategy,
            expected_bytes: 1_024,
            risk: RiskTier::Safe,
            identity: None,
            exclusions: Vec::new(),
            min_age_days: None,
            unit: crate::domain::scan::CleanupUnit::fixed_path("/Users/tester/.cache/example"),
            target_kind: crate::domain::cleanup::EntryKind::Directory,
            owner: crate::domain::scan::CleanupOwnership::unknown(),
            process_guard: crate::domain::cleanup::RunningProcessPolicy::none(),
        }
    }

    #[test]
    fn a_pseudo_path_strategy_never_classifies_as_a_filesystem_mutation() {
        let docker = target(CleanStrategy::DockerPrune);
        let provider = target(CleanStrategy::ExternalCommand);

        let container = CleanupOperation::of(&docker).expect("a container prune is an operation");
        assert!(matches!(container, CleanupOperation::Container(_)));
        let provider = CleanupOperation::of(&provider).expect("a provider prune is an operation");
        assert!(matches!(provider, CleanupOperation::Provider(_)));
        // The provider's location is a staleness assertion, not authority: it
        // is reachable only through the operation that asks the provider to
        // prune its own cache.
        if let CleanupOperation::Provider(cleanup) = provider {
            assert_eq!(
                cleanup.expected_location(),
                std::path::Path::new("/Users/tester/.cache/example")
            );
        }
    }

    #[test]
    fn a_filesystem_strategy_reports_which_part_it_removes() {
        let contents = target(CleanStrategy::DeleteContents);
        let directory = target(CleanStrategy::DeleteDirectory);

        match CleanupOperation::of(&contents).expect("contents removal is an operation") {
            CleanupOperation::Filesystem(cleanup) => {
                assert_eq!(cleanup.removes(), FilesystemMutation::Contents);
                assert_eq!(cleanup.path(), contents.path);
            }
            other => panic!("expected a filesystem operation, got {other:?}"),
        }
        match CleanupOperation::of(&directory).expect("directory removal is an operation") {
            CleanupOperation::Filesystem(cleanup) => {
                assert_eq!(cleanup.removes(), FilesystemMutation::Directory);
            }
            other => panic!("expected a filesystem operation, got {other:?}"),
        }
    }

    #[test]
    fn manual_is_not_a_generic_operation() {
        assert!(CleanupOperation::of(&target(CleanStrategy::Manual)).is_none());
    }
}
