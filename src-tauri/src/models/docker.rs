use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DockerImageItem {
    pub id: String,
    pub repository: String,
    pub tag: String,
    pub size_bytes: u64,
    pub is_dangling: bool,
    pub is_in_use: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DockerContainerItem {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub size_bytes: u64,
    pub is_running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DockerVolumeItem {
    pub name: String,
    pub driver: String,
    pub size_bytes: u64,
    pub is_in_use: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DockerBuildCacheItem {
    pub id: String,
    pub cache_type: String,
    pub size_bytes: u64,
    pub is_in_use: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct DockerResourceUsage {
    pub total_bytes: u64,
    pub reclaimable_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct DockerOverview {
    pub images: DockerResourceUsage,
    pub containers: DockerResourceUsage,
    pub volumes: DockerResourceUsage,
    pub build_cache: DockerResourceUsage,
    pub total_bytes: u64,
    pub total_reclaimable_bytes: u64,
    pub safe_cleanable_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DockerStatus {
    pub is_available: bool,
    pub is_running: bool,
    pub version: Option<String>,
    pub error_message: Option<String>,
    pub overview: Option<DockerOverview>,
    pub images: Vec<DockerImageItem>,
    pub containers: Vec<DockerContainerItem>,
    pub volumes: Vec<DockerVolumeItem>,
}
