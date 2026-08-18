use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Ai,
    Developer,
    Container,
    Model,
    System,
}

impl Category {
    pub fn display_name(&self) -> &'static str {
        match self {
            Category::Ai => "AI Tools",
            Category::Developer => "Developer",
            Category::Container => "Docker & Containers",
            Category::Model => "Local Models",
            Category::System => "System",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Category::Ai => "Caches, temporary indices, and diagnostic logs from AI coding tools",
            Category::Developer => "Compiler artifacts, package manager stores, and build caches",
            Category::Container => "Unused build layers, dangling images, and stopped containers",
            Category::Model => "Downloaded GGUF, transformer, and Apple Silicon model weights",
            Category::System => "System-level temporary files and developer simulator caches",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}
