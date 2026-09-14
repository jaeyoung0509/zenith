use crate::models::{LocalModelInventory, LocalModelItem, ModelSource, ObservationQuality};
use crate::scanner::SizeCalculator;
use crate::signatures::SignatureLoader;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use zenith_platform::PlatformEnvironment;

const MAX_INCOMPLETE_REASONS: usize = 32;

#[derive(Default)]
pub struct ModelDiscoverySink {
    pub items: Vec<LocalModelItem>,
    pub skipped_entries: u64,
    pub incomplete_reasons: Vec<String>,
    observed_scopes: u64,
}

impl ModelDiscoverySink {
    pub fn record_failure(&mut self, reason: String) {
        self.record_skipped(1, Some(reason));
    }

    pub fn record_skipped(&mut self, skipped: u64, reason: Option<String>) {
        self.skipped_entries = self.skipped_entries.saturating_add(skipped);
        if let Some(reason) = reason {
            if self.incomplete_reasons.len() < MAX_INCOMPLETE_REASONS {
                self.incomplete_reasons.push(reason);
            }
        }
    }

    fn record_observed_scope(&mut self) {
        self.observed_scopes = self.observed_scopes.saturating_add(1);
    }
}

enum OllamaSizeObservation {
    Exact(u64),
    Unavailable(String),
}

pub struct LocalModelScanner;

impl LocalModelScanner {
    /// Resolves one model search root through the environment's own path rules.
    /// Distinguishes normal absence (NotFound) from access/permission errors and symlinks,
    /// recording failures into the discovery sink so partial/unavailable observation is preserved.
    fn resolve_root(
        pattern: &str,
        environment: &PlatformEnvironment,
        provider_name: &str,
        sink: &mut ModelDiscoverySink,
    ) -> Option<PathBuf> {
        let root = match SignatureLoader::expand_path(pattern, environment) {
            Some(root) => root,
            None => {
                sink.record_failure(format!(
                    "Could not resolve {provider_name} discovery root for pattern '{pattern}'"
                ));
                return None;
            }
        };
        match crate::safety::SymlinkGuard::is_symlink_metadata(&root) {
            Ok(true) => {
                // Symlink exclusion is an intentional scan boundary, not an
                // observation failure. Treat the configured scope as inspected
                // without following it.
                sink.record_observed_scope();
                None
            }
            Ok(false) => match fs::symlink_metadata(&root) {
                Ok(meta) => {
                    if !meta.is_dir() {
                        sink.record_failure(format!(
                            "{provider_name} root {} is not a directory",
                            root.display()
                        ));
                        None
                    } else {
                        Some(root)
                    }
                }
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::NotFound {
                        sink.record_observed_scope();
                        None
                    } else {
                        sink.record_failure(format!(
                            "Could not access {provider_name} root {}: {err}",
                            root.display()
                        ));
                        None
                    }
                }
            },
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    sink.record_observed_scope();
                    None
                } else {
                    sink.record_failure(format!(
                        "Could not inspect symlink metadata for {provider_name} root {}: {err}",
                        root.display()
                    ));
                    None
                }
            }
        }
    }

    /// Inspects a model candidate entry fail-closed against symbolic links and junctions.
    /// Returns the metadata if the path is a safe directory or file.
    /// Deliberately returns `None` for symlinks (excluding them), and records a failure on access errors.
    fn inspect_model_entry(
        path: &Path,
        provider_name: &str,
        sink: &mut ModelDiscoverySink,
    ) -> Option<fs::Metadata> {
        match crate::safety::SymlinkGuard::is_symlink_metadata(path) {
            Ok(true) => None,
            Ok(false) => match fs::symlink_metadata(path) {
                Ok(meta) => Some(meta),
                Err(err) => {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        sink.record_failure(format!(
                            "Could not read metadata for {provider_name} entry {}: {err}",
                            path.display()
                        ));
                    }
                    None
                }
            },
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    sink.record_failure(format!(
                        "Could not inspect symlink metadata for {provider_name} entry {}: {err}",
                        path.display()
                    ));
                }
                None
            }
        }
    }

    /// Discovers all local models across Ollama, HuggingFace Hub, LM Studio, and Apple MLX.
    ///
    /// Every root is resolved through the stated environment, so a relocated or
    /// redirected profile is honored instead of the literal `~` spelling.
    pub fn scan_all_models(environment: &PlatformEnvironment) -> LocalModelInventory {
        let mut sink = ModelDiscoverySink::default();

        Self::scan_ollama_into(environment, &mut sink);
        Self::scan_huggingface_into(environment, &mut sink);
        Self::scan_lmstudio_into(environment, &mut sink);
        Self::scan_mlx_into(environment, &mut sink);

        let quality = if sink.skipped_entries == 0 {
            ObservationQuality::Fresh
        } else if sink.observed_scopes > 0 {
            ObservationQuality::Partial
        } else {
            ObservationQuality::Unavailable
        };

        LocalModelInventory {
            items: sink.items,
            quality,
            skipped_entry_count: sink.skipped_entries,
            incomplete_reasons: sink.incomplete_reasons,
        }
    }

    pub fn scan_ollama(environment: &PlatformEnvironment) -> Vec<LocalModelItem> {
        let mut sink = ModelDiscoverySink::default();
        Self::scan_ollama_into(environment, &mut sink);
        sink.items
    }

    pub fn scan_huggingface(environment: &PlatformEnvironment) -> Vec<LocalModelItem> {
        let mut sink = ModelDiscoverySink::default();
        Self::scan_huggingface_into(environment, &mut sink);
        sink.items
    }

    pub fn scan_lmstudio(environment: &PlatformEnvironment) -> Vec<LocalModelItem> {
        let mut sink = ModelDiscoverySink::default();
        Self::scan_lmstudio_into(environment, &mut sink);
        sink.items
    }

    pub fn scan_mlx(environment: &PlatformEnvironment) -> Vec<LocalModelItem> {
        let mut sink = ModelDiscoverySink::default();
        Self::scan_mlx_into(environment, &mut sink);
        sink.items
    }

    pub fn scan_ollama_into(environment: &PlatformEnvironment, sink: &mut ModelDiscoverySink) {
        let manifests_root =
            match Self::resolve_root("~/.ollama/models/manifests", environment, "Ollama", sink) {
                Some(root) => root,
                None => return,
            };

        // Ollama manifests structure: ~/.ollama/models/manifests/registry.ollama.ai/library/<model>/<tag>
        let registries = match fs::read_dir(&manifests_root) {
            Ok(r) => r,
            Err(e) => {
                sink.record_failure(format!(
                    "Could not read Ollama manifests directory {}: {e}",
                    manifests_root.display()
                ));
                return;
            }
        };
        sink.record_observed_scope();

        for reg in registries {
            let reg = match reg {
                Ok(r) => r,
                Err(e) => {
                    sink.record_failure(format!("Could not read Ollama registry entry: {e}"));
                    continue;
                }
            };
            let reg_path = reg.path();
            let reg_meta = match Self::inspect_model_entry(&reg_path, "Ollama registry", sink) {
                Some(m) => m,
                None => continue,
            };
            if !reg_meta.is_dir() {
                continue;
            }

            let namespaces = match fs::read_dir(&reg_path) {
                Ok(ns) => ns,
                Err(e) => {
                    sink.record_failure(format!(
                        "Could not read Ollama namespace directory {}: {e}",
                        reg_path.display()
                    ));
                    continue;
                }
            };

            for ns in namespaces {
                let ns = match ns {
                    Ok(n) => n,
                    Err(e) => {
                        sink.record_failure(format!("Could not read Ollama namespace entry: {e}"));
                        continue;
                    }
                };
                let ns_path = ns.path();
                let ns_meta = match Self::inspect_model_entry(&ns_path, "Ollama namespace", sink) {
                    Some(m) => m,
                    None => continue,
                };
                if !ns_meta.is_dir() {
                    continue;
                }

                let model_dirs = match fs::read_dir(&ns_path) {
                    Ok(md) => md,
                    Err(e) => {
                        sink.record_failure(format!(
                            "Could not read Ollama model directory {}: {e}",
                            ns_path.display()
                        ));
                        continue;
                    }
                };

                for md in model_dirs {
                    let md = match md {
                        Ok(m) => m,
                        Err(e) => {
                            sink.record_failure(format!("Could not read Ollama model entry: {e}"));
                            continue;
                        }
                    };
                    let md_path = md.path();
                    let md_meta = match Self::inspect_model_entry(&md_path, "Ollama model", sink) {
                        Some(m) => m,
                        None => continue,
                    };
                    if !md_meta.is_dir() {
                        continue;
                    }

                    let model_name = md.file_name().to_string_lossy().to_string();
                    let tags = match fs::read_dir(&md_path) {
                        Ok(t) => t,
                        Err(e) => {
                            sink.record_failure(format!(
                                "Could not read Ollama tags directory {}: {e}",
                                md_path.display()
                            ));
                            continue;
                        }
                    };

                    for tag in tags {
                        let tag = match tag {
                            Ok(t) => t,
                            Err(e) => {
                                sink.record_failure(format!(
                                    "Could not read Ollama tag entry: {e}"
                                ));
                                continue;
                            }
                        };
                        let path = tag.path();
                        let tag_meta = match Self::inspect_model_entry(&path, "Ollama tag", sink) {
                            Some(meta) => meta,
                            None => continue,
                        };

                        let tag_name = tag.file_name().to_string_lossy().to_string();
                        let full_name = format!("{}:{}", model_name, tag_name);

                        // Read manifest JSON to calculate layer sizes
                        let size_observation = Self::compute_ollama_model_size(&path);
                        let (size_bytes, quality, incomplete_reason, skipped_entries) =
                            match size_observation {
                                OllamaSizeObservation::Exact(size) => {
                                    (size, ObservationQuality::Fresh, None, 0)
                                }
                                OllamaSizeObservation::Unavailable(reason) => {
                                    sink.record_failure(reason.clone());
                                    (0, ObservationQuality::Unavailable, Some(reason), 1)
                                }
                            };

                        let last_modified = tag_meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs());

                        sink.items.push(LocalModelItem {
                            id: format!("ollama.{}", full_name),
                            name: full_name,
                            source: ModelSource::Ollama,
                            path: path.to_string_lossy().to_string(),
                            size_bytes,
                            format: Some("GGUF".to_string()),
                            parameter_size: None,
                            quantization: None,
                            last_modified,
                            quality,
                            incomplete_reason,
                            skipped_entries,
                        });
                    }
                }
            }
        }
    }

    fn compute_ollama_model_size(manifest_path: &Path) -> OllamaSizeObservation {
        let content = match fs::read_to_string(manifest_path) {
            Ok(c) => c,
            Err(e) => {
                return OllamaSizeObservation::Unavailable(format!(
                    "Could not read Ollama manifest {}: {e}",
                    manifest_path.display()
                ));
            }
        };

        let val: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                return OllamaSizeObservation::Unavailable(format!(
                    "Could not parse Ollama manifest {}: {e}",
                    manifest_path.display()
                ));
            }
        };

        let unavailable = |detail: &str| {
            OllamaSizeObservation::Unavailable(format!(
                "Could not measure Ollama manifest {}: {detail}",
                manifest_path.display()
            ))
        };
        let Some(layers) = val.get("layers").and_then(|layers| layers.as_array()) else {
            return unavailable("missing or invalid layers array");
        };
        let mut total_size = 0u64;
        for layer in layers {
            let Some(size) = layer.get("size").and_then(|size| size.as_u64()) else {
                return unavailable("a layer is missing a valid size");
            };
            let Some(next) = total_size.checked_add(size) else {
                return unavailable("layer sizes overflow u64");
            };
            total_size = next;
        }

        let Some(config_size) = val
            .get("config")
            .and_then(|c| c.get("size"))
            .and_then(|s| s.as_u64())
        else {
            return unavailable("missing or invalid config size");
        };
        let Some(total_size) = total_size.checked_add(config_size) else {
            return unavailable("layer and config sizes overflow u64");
        };

        OllamaSizeObservation::Exact(total_size)
    }

    /// Scans HuggingFace Hub snapshots.
    pub fn scan_huggingface_into(environment: &PlatformEnvironment, sink: &mut ModelDiscoverySink) {
        let hf_root = match Self::resolve_root(
            "~/.cache/huggingface/hub",
            environment,
            "HuggingFace",
            sink,
        ) {
            Some(root) => root,
            None => return,
        };

        let entries = match fs::read_dir(&hf_root) {
            Ok(e) => e,
            Err(err) => {
                sink.record_failure(format!(
                    "Could not read HuggingFace hub root {}: {err}",
                    hf_root.display()
                ));
                return;
            }
        };
        sink.record_observed_scope();

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    sink.record_failure(format!(
                        "Could not read HuggingFace hub entry in {}: {err}",
                        hf_root.display()
                    ));
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("models--") {
                let path = entry.path();
                let meta = match Self::inspect_model_entry(&path, "HuggingFace model", sink) {
                    Some(m) => m,
                    None => continue,
                };
                if !meta.is_dir() {
                    continue;
                }
                let clean_name = name.trim_start_matches("models--").replace("--", "/");
                let measurement = SizeCalculator::measure_path_logged(&path, &[], environment);
                let quality = if measurement.complete && measurement.skipped_entries == 0 {
                    ObservationQuality::Fresh
                } else if measurement.size.reclaimable() > 0 {
                    ObservationQuality::Partial
                } else {
                    ObservationQuality::Unavailable
                };
                if quality != ObservationQuality::Fresh {
                    sink.record_skipped(
                        measurement.skipped_entries.max(1),
                        measurement.incomplete_reason.clone(),
                    );
                }
                let last_modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                sink.items.push(LocalModelItem {
                    id: format!("hf.{}", clean_name),
                    name: clean_name,
                    source: ModelSource::HuggingFace,
                    path: path.to_string_lossy().to_string(),
                    size_bytes: measurement.size.reclaimable(),
                    format: Some("safetensors / PyTorch".to_string()),
                    parameter_size: None,
                    quantization: None,
                    last_modified,
                    quality,
                    incomplete_reason: measurement.incomplete_reason,
                    skipped_entries: measurement.skipped_entries,
                });
            }
        }
    }

    /// Scans LM Studio downloaded models directory.
    pub fn scan_lmstudio_into(environment: &PlatformEnvironment, sink: &mut ModelDiscoverySink) {
        let lm_root =
            match Self::resolve_root("~/.cache/lm-studio/models", environment, "LM Studio", sink) {
                Some(root) => root,
                None => return,
            };

        if Self::collect_gguf_files(&lm_root, &lm_root, ModelSource::LmStudio, "lmstudio", sink) {
            sink.record_observed_scope();
        }
    }

    /// Scans MLX model weights directory.
    pub fn scan_mlx_into(environment: &PlatformEnvironment, sink: &mut ModelDiscoverySink) {
        let mlx_root = match Self::resolve_root("~/.cache/mlx", environment, "Apple MLX", sink) {
            Some(root) => root,
            None => return,
        };

        let entries = match fs::read_dir(&mlx_root) {
            Ok(e) => e,
            Err(err) => {
                sink.record_failure(format!(
                    "Could not read Apple MLX root {}: {err}",
                    mlx_root.display()
                ));
                return;
            }
        };
        sink.record_observed_scope();

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    sink.record_failure(format!(
                        "Could not read Apple MLX entry in {}: {err}",
                        mlx_root.display()
                    ));
                    continue;
                }
            };
            let path = entry.path();
            let meta = match Self::inspect_model_entry(&path, "Apple MLX model", sink) {
                Some(m) => m,
                None => continue,
            };

            if meta.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                let measurement = SizeCalculator::measure_path_logged(&path, &[], environment);
                let quality = if measurement.complete && measurement.skipped_entries == 0 {
                    ObservationQuality::Fresh
                } else if measurement.size.reclaimable() > 0 {
                    ObservationQuality::Partial
                } else {
                    ObservationQuality::Unavailable
                };
                if quality != ObservationQuality::Fresh {
                    sink.record_skipped(
                        measurement.skipped_entries.max(1),
                        measurement.incomplete_reason.clone(),
                    );
                }
                let last_modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                sink.items.push(LocalModelItem {
                    id: format!("mlx.{}", name),
                    name,
                    source: ModelSource::Mlx,
                    path: path.to_string_lossy().to_string(),
                    size_bytes: measurement.size.reclaimable(),
                    format: Some("MLX 4-bit / 8-bit".to_string()),
                    parameter_size: None,
                    quantization: None,
                    last_modified,
                    quality,
                    incomplete_reason: measurement.incomplete_reason,
                    skipped_entries: measurement.skipped_entries,
                });
            }
        }
    }

    fn collect_gguf_files(
        dir: &Path,
        root: &Path,
        source: ModelSource,
        prefix: &str,
        sink: &mut ModelDiscoverySink,
    ) -> bool {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(err) => {
                sink.record_failure(format!(
                    "Could not read LM Studio directory {}: {err}",
                    dir.display()
                ));
                return false;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    sink.record_failure(format!(
                        "Could not read LM Studio entry in {}: {err}",
                        dir.display()
                    ));
                    continue;
                }
            };
            let path = entry.path();
            let meta = match Self::inspect_model_entry(&path, "LM Studio entry", sink) {
                Some(m) => m,
                None => continue,
            };

            if meta.is_dir() {
                let _ = Self::collect_gguf_files(&path, root, source, prefix, sink);
            } else if path.extension().and_then(|e| e.to_str()) == Some("gguf") {
                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                let size = meta.len();
                let last_modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                let rel_path = path.strip_prefix(root).unwrap_or(&path);
                let unique_id = format!("{}:{}", prefix, rel_path.to_string_lossy());

                sink.items.push(LocalModelItem {
                    id: unique_id,
                    name: file_name,
                    source,
                    path: path.to_string_lossy().to_string(),
                    size_bytes: size,
                    format: Some("GGUF".to_string()),
                    parameter_size: None,
                    quantization: None,
                    last_modified,
                    quality: ObservationQuality::Fresh,
                    incomplete_reason: None,
                    skipped_entries: 0,
                });
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::LocalModelScanner;
    use crate::models::ObservationQuality;
    use std::path::{Path, PathBuf};
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::{KnownFolder, PlatformEnvironment};

    /// A stated POSIX home holding one fixture per supported model source.
    fn fixture_home() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let manifests = home
            .path()
            .join(".ollama/models/manifests/registry.ollama.ai/library/zenith-env-probe");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(
            manifests.join("local"),
            r#"{"layers":[{"size":4096}],"config":{"size":512}}"#,
        )
        .unwrap();

        let hf = home
            .path()
            .join(".cache/huggingface/hub/models--zenith--probe");
        std::fs::create_dir_all(&hf).unwrap();
        std::fs::write(hf.join("weights.safetensors"), vec![1u8; 2_048]).unwrap();

        let lm = home.path().join(".cache/lm-studio/models/zenith-gguf");
        std::fs::create_dir_all(&lm).unwrap();
        std::fs::write(lm.join("model.gguf"), vec![2u8; 1_024]).unwrap();

        let mlx = home.path().join(".cache/mlx/zenith-mlx");
        std::fs::create_dir_all(&mlx).unwrap();
        std::fs::write(mlx.join("weights.npz"), vec![3u8; 512]).unwrap();

        home
    }

    #[test]
    fn every_model_root_is_resolved_from_the_stated_environment() {
        let home = fixture_home();
        // The fixture comes from `tempfile`, so it follows the host's path
        // rules; the test is about which profile answers, not about spelling.
        let environment =
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(home.path());

        let ollama = LocalModelScanner::scan_ollama(&environment);
        assert_eq!(ollama.len(), 1, "the stated home holds one Ollama manifest");
        assert_eq!(ollama[0].id, "ollama.zenith-env-probe:local");
        assert_eq!(
            ollama[0].size_bytes, 4_608,
            "layer and config sizes come from the manifest"
        );
        assert!(ollama[0].path.starts_with(home.path().to_str().unwrap()));

        let huggingface = LocalModelScanner::scan_huggingface(&environment);
        assert_eq!(huggingface.len(), 1);
        assert_eq!(huggingface[0].name, "zenith/probe");
        // Measured from disk, so the block-rounded allocation is at least the
        // file's logical length; a scanner that reported nothing would fail.
        assert!(
            huggingface[0].size_bytes >= 2_048,
            "huggingface snapshot size: {}",
            huggingface[0].size_bytes
        );

        let lmstudio = LocalModelScanner::scan_lmstudio(&environment);
        assert_eq!(lmstudio.len(), 1);
        assert_eq!(lmstudio[0].name, "model.gguf");
        assert_eq!(lmstudio[0].size_bytes, 1_024);

        let mlx = LocalModelScanner::scan_mlx(&environment);
        assert_eq!(mlx.len(), 1);
        assert_eq!(mlx[0].name, "zenith-mlx");
        assert!(
            mlx[0].size_bytes >= 512,
            "mlx weights size: {}",
            mlx[0].size_bytes
        );

        assert_eq!(
            LocalModelScanner::scan_all_models(&environment).items.len(),
            4,
            "one model per source is discovered through the stated home"
        );
    }

    #[test]
    fn a_stated_windows_profile_is_never_replaced_by_the_host_profile() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows)
            .with_home(r"D:\Users\me")
            .with_known_folder(KnownFolder::Downloads, r"D:\Redirected\Downloads");

        // The `~` spelling resolves through the stated profile and its own
        // separators, not through whatever home the host happens to have.
        let mut sink = super::ModelDiscoverySink::default();
        assert!(
            LocalModelScanner::resolve_root(
                "~/.ollama/models/manifests",
                &environment,
                "Ollama",
                &mut sink
            )
            .is_none(),
            "the stated machine has no Ollama install"
        );
        assert_eq!(sink.skipped_entries, 0);
        assert_eq!(
            crate::signatures::SignatureLoader::expand_path(
                "~/.ollama/models/manifests",
                &environment
            ),
            Some(PathBuf::from(r"D:\Users\me\.ollama\models\manifests"))
        );
        assert!(
            LocalModelScanner::scan_all_models(&environment)
                .items
                .is_empty(),
            "a stated Windows profile must not fall back to the host profile"
        );
    }

    #[test]
    fn missing_roots_yield_no_models_instead_of_host_paths() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix)
            .with_home(Path::new("/nonexistent-zenith-test-home"));
        assert!(LocalModelScanner::scan_all_models(&environment)
            .items
            .is_empty());
    }

    #[test]
    fn unresolvable_model_root_records_failure_and_marks_inventory_unavailable() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix);
        let inventory = LocalModelScanner::scan_all_models(&environment);
        assert!(inventory.items.is_empty());
        assert_eq!(inventory.quality, ObservationQuality::Unavailable);
        assert!(inventory.skipped_entry_count >= 1);
        assert!(inventory
            .incomplete_reasons
            .iter()
            .any(|r| r.contains("Could not resolve")));
    }

    #[test]
    fn corrupted_ollama_manifest_is_not_reported_as_zero_bytes_and_marks_inventory_partial() {
        let home = tempfile::tempdir().unwrap();
        let ollama = home
            .path()
            .join(".ollama/models/manifests/registry.ollama.ai/library/corrupt");
        std::fs::create_dir_all(&ollama).unwrap();
        std::fs::write(ollama.join("badtag"), b"not valid json").unwrap();

        let environment =
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(home.path());
        let inventory = LocalModelScanner::scan_all_models(&environment);

        assert_eq!(inventory.items.len(), 1);
        let item = &inventory.items[0];
        assert_eq!(item.id, "ollama.corrupt:badtag");
        assert_eq!(item.name, "corrupt:badtag");
        assert_eq!(item.size_bytes, 0);
        assert_eq!(item.quality, ObservationQuality::Unavailable);
        assert!(item.incomplete_reason.is_some());
        assert_eq!(item.skipped_entries, 1);

        assert_eq!(inventory.quality, ObservationQuality::Partial);
        assert!(inventory.skipped_entry_count >= 1);
        assert!(!inventory.incomplete_reasons.is_empty());
    }

    #[test]
    fn structurally_invalid_ollama_manifest_is_not_reported_as_exact_zero_bytes() {
        let home = tempfile::tempdir().unwrap();
        let ollama = home
            .path()
            .join(".ollama/models/manifests/registry.ollama.ai/library/invalid");
        std::fs::create_dir_all(&ollama).unwrap();
        std::fs::write(ollama.join("badtag"), br#"{"layers":[{}],"config":{}}"#).unwrap();

        let environment =
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(home.path());
        let inventory = LocalModelScanner::scan_all_models(&environment);

        assert_eq!(inventory.items.len(), 1);
        assert_eq!(inventory.items[0].quality, ObservationQuality::Unavailable);
        assert!(inventory.items[0]
            .incomplete_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("valid size")));
        assert_eq!(inventory.quality, ObservationQuality::Partial);
    }

    #[test]
    fn a_failed_scope_and_known_empty_scopes_produce_partial_empty_inventory() {
        let home = tempfile::tempdir().unwrap();
        let manifests = home.path().join(".ollama/models/manifests");
        std::fs::create_dir_all(manifests.parent().unwrap()).unwrap();
        std::fs::write(&manifests, b"not a directory").unwrap();

        let environment =
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(home.path());
        let inventory = LocalModelScanner::scan_all_models(&environment);

        assert!(inventory.items.is_empty());
        assert_eq!(inventory.quality, ObservationQuality::Partial);
        assert!(inventory.skipped_entry_count >= 1);
    }

    #[cfg(unix)]
    #[test]
    fn huggingface_scan_excludes_symlinked_model_entry() {
        let home = tempfile::tempdir().unwrap();
        let hf_hub = home.path().join(".cache/huggingface/hub");
        let target_dir = home.path().join("external_target");
        std::fs::create_dir_all(&hf_hub).unwrap();
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(target_dir.join("model.bin"), b"payload").unwrap();

        let symlink_entry = hf_hub.join("models--fake--escaped");
        std::os::unix::fs::symlink(&target_dir, &symlink_entry).unwrap();

        let environment =
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(home.path());
        let items = LocalModelScanner::scan_huggingface(&environment);
        assert!(
            items.is_empty(),
            "symlinked HuggingFace model entry must be excluded from inventory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_model_root_is_an_intentional_exclusion_not_an_io_failure() {
        let home = tempfile::tempdir().unwrap();
        let real_manifests = home.path().join("real_manifests");
        std::fs::create_dir_all(&real_manifests).unwrap();
        let symlink_root = home.path().join(".ollama/models/manifests");
        std::fs::create_dir_all(symlink_root.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real_manifests, &symlink_root).unwrap();

        let environment =
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(home.path());
        let mut sink = super::ModelDiscoverySink::default();
        let resolved = LocalModelScanner::resolve_root(
            "~/.ollama/models/manifests",
            &environment,
            "Ollama",
            &mut sink,
        );
        assert!(resolved.is_none());
        assert_eq!(sink.skipped_entries, 0);
        assert!(sink.incomplete_reasons.is_empty());
        assert_eq!(sink.observed_scopes, 1);
    }
}
