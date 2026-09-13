use crate::models::ObservationQuality;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ModelSource {
    Ollama,
    HuggingFace,
    LmStudio,
    Mlx,
}

impl ModelSource {
    pub fn display_name(&self) -> &'static str {
        match self {
            ModelSource::Ollama => "Ollama",
            ModelSource::HuggingFace => "HuggingFace Hub",
            ModelSource::LmStudio => "LM Studio",
            ModelSource::Mlx => "Apple MLX",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LocalModelItem {
    pub id: String,
    pub name: String,
    pub source: ModelSource,
    pub path: String,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub size_bytes: u64,
    pub format: Option<String>,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub last_modified: Option<u64>,
    #[serde(default)]
    pub quality: ObservationQuality,
    #[serde(default)]
    pub incomplete_reason: Option<String>,
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entries: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LocalModelInventory {
    pub items: Vec<LocalModelItem>,
    pub quality: ObservationQuality,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entry_count: u64,
    pub incomplete_reasons: Vec<String>,
}
