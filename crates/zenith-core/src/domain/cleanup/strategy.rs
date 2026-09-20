use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CleanStrategy {
    /// Remove the path's children, whatever their age.
    DeleteContents,
    /// Remove the path and everything under it.
    DeleteDirectory,
    /// Remove the entries under the path whose *own* age satisfies the
    /// signature's age policy, leaving everything newer and everything
    /// structured in place.
    ///
    /// This is the strategy for a cache namespace that is written to while it
    /// is being cleaned: a single recent file no longer disqualifies the
    /// gigabytes beside it, and no entry is removed on a verdict about its
    /// neighbours.
    DeleteStaleContents,
    ExternalCommand,
    DockerPrune,
    /// A registered owner-scoped provider enumerates and mutates the units of
    /// a store whose semantics only its owner knows.
    ///
    /// The store is inventoried by the provider, not by the signature: the
    /// catalog names the owner and the provider resolves its own root from the
    /// environment. Unlike [`Self::DeleteContents`] the units are not generic
    /// filesystem targets — the provider re-derives each one and refuses any
    /// entry its store contract does not describe — and unlike
    /// [`Self::LifecycleProvider`] the units are addressable, so a user selects
    /// the ones to remove.
    OwnerProvider,
    /// A reviewed lifecycle-aware provider performs the operation through the
    /// interface the owning system or application provides.
    ///
    /// The unit owns no host path: the provider re-derives its own state at
    /// execution time, states its prerequisites itself, and verifies its own
    /// postcondition. Unlike [`Self::Manual`] the action is executable — but
    /// only by the provider the catalog named, never by a filesystem
    /// primitive.
    LifecycleProvider,
    Manual,
}
