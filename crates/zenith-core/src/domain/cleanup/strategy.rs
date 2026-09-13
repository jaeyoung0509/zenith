use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CleanStrategy {
    DeleteContents,
    DeleteDirectory,
    ExternalCommand,
    DockerPrune,
    Manual,
}
