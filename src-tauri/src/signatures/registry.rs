use crate::models::{Category, RiskTier, Signature, ZenithError};
use crate::signatures::SignatureLoader;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const EMBEDDED_AI_TOML: &str = include_str!("../../../signatures/ai.toml");
const EMBEDDED_DEV_TOML: &str = include_str!("../../../signatures/developer.toml");
const EMBEDDED_CONTAINERS_TOML: &str = include_str!("../../../signatures/containers.toml");
const EMBEDDED_MODELS_TOML: &str = include_str!("../../../signatures/models.toml");
const EMBEDDED_SYSTEM_TOML: &str = include_str!("../../../signatures/system.toml");

#[derive(Debug, Clone)]
pub struct SignatureRegistry {
    signatures: HashMap<String, Signature>,
}

impl Default for SignatureRegistry {
    fn default() -> Self {
        Self::load_embedded().unwrap_or_else(|_| Self {
            signatures: HashMap::new(),
        })
    }
}

impl SignatureRegistry {
    pub fn new() -> Self {
        Self {
            signatures: HashMap::new(),
        }
    }

    /// Loads built-in embedded signatures for AI, Developer, Container, and Model categories.
    pub fn load_embedded() -> Result<Self, ZenithError> {
        let mut registry = Self::new();

        let tomls = [
            EMBEDDED_AI_TOML,
            EMBEDDED_DEV_TOML,
            EMBEDDED_CONTAINERS_TOML,
            EMBEDDED_MODELS_TOML,
            EMBEDDED_SYSTEM_TOML,
        ];

        for toml_str in &tomls {
            let sigs = SignatureLoader::load_str(toml_str)?;
            for sig in sigs {
                registry.register(sig);
            }
        }

        Ok(registry)
    }

    /// Loads signatures from a directory containing `.toml` files.
    pub fn load_from_dir<P: AsRef<Path>>(&mut self, dir_path: P) -> Result<usize, ZenithError> {
        let dir = dir_path.as_ref();
        if !dir.exists() || !dir.is_dir() {
            return Ok(0);
        }

        let mut count = 0;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                    if let Ok(sigs) = SignatureLoader::load_file(&path) {
                        for sig in sigs {
                            self.register(sig);
                            count += 1;
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    /// Registers a single signature.
    pub fn register(&mut self, signature: Signature) {
        self.signatures.insert(signature.id.clone(), signature);
    }

    /// Gets a signature by ID.
    pub fn get(&self, id: &str) -> Option<&Signature> {
        self.signatures.get(id)
    }

    /// Lists all signatures.
    pub fn all(&self) -> Vec<&Signature> {
        self.signatures.values().collect()
    }

    /// Lists signatures by category.
    pub fn by_category(&self, category: Category) -> Vec<&Signature> {
        self.signatures
            .values()
            .filter(|s| s.category == category)
            .collect()
    }

    /// Lists signatures available for the selected scan scope.
    pub fn by_category_for_mode(
        &self,
        category: Category,
        intensive_cleanup: bool,
    ) -> Vec<&Signature> {
        self.signatures
            .values()
            .filter(|signature| {
                signature.category == category && (intensive_cleanup || !signature.intensive_only)
            })
            .collect()
    }

    /// Lists signatures by risk tier.
    pub fn by_risk(&self, risk: RiskTier) -> Vec<&Signature> {
        self.signatures
            .values()
            .filter(|s| s.risk == risk)
            .collect()
    }

    /// Resolves expanded paths for a given signature.
    pub fn resolve_paths(&self, signature: &Signature) -> Vec<PathBuf> {
        signature
            .paths
            .iter()
            .filter_map(|p| SignatureLoader::expand_path(p))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::SignatureRegistry;
    use crate::models::Category;

    #[test]
    fn intensive_signatures_are_opt_in() {
        let registry = SignatureRegistry::load_embedded().unwrap();

        let standard = registry.by_category_for_mode(Category::System, false);
        assert!(standard.iter().all(|signature| !signature.intensive_only));

        let intensive = registry.by_category_for_mode(Category::System, true);
        assert!(intensive.iter().any(|signature| signature.intensive_only));
        assert!(intensive.len() > standard.len());
    }

    #[test]
    fn developer_temp_prefixes_keep_the_three_day_age_gate() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let signature = registry.get("system.developer_temp").unwrap();

        for prefix in [
            "agent-browser-chrome-",
            "metro-cache",
            "metro-file-map-",
            "node-compile-cache",
            "openai-docs-cache",
            "pytest-of-",
            "v8-compile-cache-",
        ] {
            assert!(
                signature
                    .include_prefixes
                    .iter()
                    .any(|entry| entry == prefix),
                "missing reviewed prefix: {prefix}"
            );
        }
        assert_eq!(signature.min_age_days, Some(3));
    }
}
