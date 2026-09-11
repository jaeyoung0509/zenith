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
                signature.category == category
                    && signature.supports_current_platform()
                    && (intensive_cleanup || !signature.intensive_only)
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

    /// Returns which platform (if any) a path pattern or placeholder is specifically tied to.
    pub fn is_platform_specific_path(pattern: &str) -> Option<crate::models::PlatformKind> {
        let p = pattern.trim();
        if p.starts_with("~/Library")
            || p.starts_with("/Applications")
            || p.starts_with("/Library")
            || p.starts_with("/System")
            || p.starts_with("/private/")
        {
            Some(crate::models::PlatformKind::Macos)
        } else if p.contains("${LOCAL_APP_DATA}")
            || p.contains("${ROAMING_APP_DATA}")
            || p.contains("${PROGRAM_DATA}")
            || p.contains("${PROGRAM_FILES}")
            || p.starts_with("C:\\")
            || p.starts_with("c:\\")
            || p.starts_with("%USERPROFILE%")
            || p.starts_with("%APPDATA%")
            || p.starts_with("%LOCALAPPDATA%")
            || p.starts_with("%PROGRAMDATA%")
            || p.starts_with("%PROGRAMFILES%")
            || p.starts_with("%SYSTEMROOT%")
            || p.starts_with("%WINDIR%")
        {
            Some(crate::models::PlatformKind::Windows)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SignatureRegistry;
    use crate::models::{Category, RiskTier};

    #[test]
    fn intensive_signatures_are_opt_in() {
        let registry = SignatureRegistry::load_embedded().unwrap();

        let standard = registry.by_category_for_mode(Category::System, false);
        assert!(standard.iter().all(|signature| !signature.intensive_only));

        #[cfg(target_os = "macos")]
        {
            let intensive = registry.by_category_for_mode(Category::System, true);
            assert!(intensive.iter().any(|signature| signature.intensive_only));
            assert!(intensive.len() > standard.len());
        }

        #[cfg(target_os = "windows")]
        {
            let intensive = registry.by_category_for_mode(Category::System, true);
            assert_eq!(intensive.len(), standard.len());
            assert!(intensive.iter().all(|signature| !signature.intensive_only));
        }
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

    #[test]
    fn user_app_caches_exclude_tool_managed_dotslash_cache() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let signature = registry.get("system.intensive.user_app_caches").unwrap();

        assert!(
            signature
                .exclude_prefixes
                .iter()
                .any(|prefix| prefix == "dotslash"),
            "dotslash cache must stay excluded: it is content-addressed and managed by the dotslash CLI"
        );
    }

    #[test]
    fn platform_specific_gpu_signatures_do_not_cross_platforms() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let ai = registry.by_category_for_mode(Category::Ai, false);
        #[cfg(target_os = "macos")]
        {
            assert!(ai
                .iter()
                .any(|signature| signature.id == "ai.llamacpp.opencl.macos"));
            assert!(ai
                .iter()
                .all(|signature| signature.id != "ai.gpu.directx_shader"));
        }
        #[cfg(target_os = "windows")]
        {
            assert!(ai
                .iter()
                .any(|signature| signature.id == "ai.gpu.directx_shader"));
            assert!(ai
                .iter()
                .all(|signature| signature.id != "ai.llamacpp.opencl.macos"));
        }
    }

    #[test]
    fn shared_package_stores_are_never_generic_safe_deletions() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        for id in [
            "dev.npm.cache",
            "dev.pnpm.store",
            "dev.yarn.cache",
            "dev.bun.cache",
            "dev.pip.cache",
            "dev.uv.cache",
            "dev.gradle.caches",
            "dev.m2.repository",
        ] {
            let signature = registry.get(id).unwrap();
            assert_ne!(signature.risk, RiskTier::Safe, "{id}");
        }
        assert_eq!(
            registry.get("dev.uv.cache").unwrap().strategy,
            crate::models::CleanStrategy::ExternalCommand
        );
        assert_eq!(
            registry.get("dev.pnpm.store").unwrap().strategy,
            crate::models::CleanStrategy::ExternalCommand
        );
        assert_eq!(
            registry.get("dev.npm.cache").unwrap().strategy,
            crate::models::CleanStrategy::ExternalCommand
        );
    }

    #[test]
    fn every_signature_resolving_to_platform_specific_root_declares_platforms() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        for signature in registry.all() {
            let mut specific_platforms = std::collections::HashSet::new();
            for path in &signature.paths {
                if let Some(platform) = SignatureRegistry::is_platform_specific_path(path) {
                    specific_platforms.insert(platform);
                }
            }

            // If a signature only resolves to roots for a single platform, it must declare `platforms`
            if specific_platforms.len() == 1 {
                let target_platform = specific_platforms.into_iter().next().unwrap();
                assert!(
                    !signature.platforms.is_empty(),
                    "Signature {} resolves to {target_platform:?}-specific root but does not declare `platforms`",
                    signature.id
                );
                assert!(
                    signature.platforms.contains(&target_platform),
                    "Signature {} must include {target_platform:?} in `platforms`",
                    signature.id
                );
            }
        }
    }

    #[test]
    fn intensive_signatures_are_unavailable_on_windows() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let intensive_sigs: Vec<_> = registry
            .all()
            .into_iter()
            .filter(|s| s.intensive_only)
            .collect();
        assert!(
            !intensive_sigs.is_empty(),
            "Expected at least one intensive signature"
        );
        for sig in intensive_sigs {
            assert!(
                sig.platforms.contains(&crate::models::PlatformKind::Macos)
                    && !sig
                        .platforms
                        .contains(&crate::models::PlatformKind::Windows),
                "Intensive signature {} must not target Windows",
                sig.id
            );
        }
    }
}
