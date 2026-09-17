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
    Manual,
}
