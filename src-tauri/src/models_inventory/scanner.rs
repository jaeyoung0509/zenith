use crate::models::{LocalModelItem, ModelSource};
use crate::platform::PlatformEnvironment;
use crate::scanner::SizeCalculator;
use crate::signatures::SignatureLoader;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct LocalModelScanner;

impl LocalModelScanner {
    /// Resolves one model search root through the environment's own path rules
    /// and keeps it only when the stated machine actually has that directory.
    fn resolve_root(pattern: &str, environment: &PlatformEnvironment) -> Option<PathBuf> {
        SignatureLoader::expand_path(pattern, environment).filter(|root| root.exists())
    }

    /// Discovers all local models across Ollama, HuggingFace Hub, LM Studio, and Apple MLX.
    ///
    /// Every root is resolved through the stated environment, so a relocated or
    /// redirected profile is honored instead of the literal `~` spelling.
    pub fn scan_all_models(environment: &PlatformEnvironment) -> Vec<LocalModelItem> {
        let mut models = Vec::new();

        // 1. Ollama models
        models.extend(Self::scan_ollama(environment));

        // 2. HuggingFace Hub models
        models.extend(Self::scan_huggingface(environment));

        // 3. LM Studio models
        models.extend(Self::scan_lmstudio(environment));

        // 4. Apple MLX models
        models.extend(Self::scan_mlx(environment));

        models
    }

    /// Scans Ollama manifest directory to identify installed models and their sizes.
    pub fn scan_ollama(environment: &PlatformEnvironment) -> Vec<LocalModelItem> {
        let manifests_root = match Self::resolve_root("~/.ollama/models/manifests", environment) {
            Some(root) => root,
            None => return Vec::new(),
        };

        let mut models = Vec::new();
        // Ollama manifests structure: ~/.ollama/models/manifests/registry.ollama.ai/library/<model>/<tag>
        if let Ok(registries) = fs::read_dir(&manifests_root) {
            for reg in registries.flatten() {
                if !reg.path().is_dir() {
                    continue;
                }
                if let Ok(namespaces) = fs::read_dir(reg.path()) {
                    for ns in namespaces.flatten() {
                        if !ns.path().is_dir() {
                            continue;
                        }
                        if let Ok(model_dirs) = fs::read_dir(ns.path()) {
                            for md in model_dirs.flatten() {
                                if !md.path().is_dir() {
                                    continue;
                                }
                                let model_name = md.file_name().to_string_lossy().to_string();
                                if let Ok(tags) = fs::read_dir(md.path()) {
                                    for tag in tags.flatten() {
                                        let tag_name =
                                            tag.file_name().to_string_lossy().to_string();
                                        let full_name = format!("{}:{}", model_name, tag_name);
                                        let path = tag.path();

                                        // Read manifest JSON to calculate layer sizes
                                        let size_bytes = Self::compute_ollama_model_size(&path);
                                        let last_modified = fs::metadata(&path)
                                            .ok()
                                            .and_then(|m| m.modified().ok())
                                            .and_then(|t| {
                                                t.duration_since(SystemTime::UNIX_EPOCH).ok()
                                            })
                                            .map(|d| d.as_secs());

                                        models.push(LocalModelItem {
                                            id: format!("ollama.{}", full_name),
                                            name: full_name,
                                            source: ModelSource::Ollama,
                                            path: path.to_string_lossy().to_string(),
                                            size_bytes,
                                            format: Some("GGUF".to_string()),
                                            parameter_size: None,
                                            quantization: None,
                                            last_modified,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        models
    }

    fn compute_ollama_model_size(manifest_path: &Path) -> u64 {
        let content = match fs::read_to_string(manifest_path) {
            Ok(c) => c,
            Err(_) => return 0,
        };

        let val: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => return 0,
        };

        let mut total_size = 0u64;

        if let Some(layers) = val.get("layers").and_then(|l| l.as_array()) {
            for layer in layers {
                if let Some(size) = layer.get("size").and_then(|s| s.as_u64()) {
                    total_size += size;
                }
            }
        }

        // Add config layer size
        if let Some(cfg) = val
            .get("config")
            .and_then(|c| c.get("size"))
            .and_then(|s| s.as_u64())
        {
            total_size += cfg;
        }

        total_size
    }

    /// Scans HuggingFace Hub snapshots.
    pub fn scan_huggingface(environment: &PlatformEnvironment) -> Vec<LocalModelItem> {
        let hf_root = match Self::resolve_root("~/.cache/huggingface/hub", environment) {
            Some(root) => root,
            None => return Vec::new(),
        };

        let mut models = Vec::new();
        if let Ok(entries) = fs::read_dir(&hf_root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("models--") {
                    let clean_name = name.trim_start_matches("models--").replace("--", "/");
                    let path = entry.path();
                    let (size, _) = SizeCalculator::measure_path(&path, &[], environment);
                    let last_modified = fs::metadata(&path)
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());

                    models.push(LocalModelItem {
                        id: format!("hf.{}", clean_name),
                        name: clean_name,
                        source: ModelSource::HuggingFace,
                        path: path.to_string_lossy().to_string(),
                        size_bytes: size.reclaimable(),
                        format: Some("safetensors / PyTorch".to_string()),
                        parameter_size: None,
                        quantization: None,
                        last_modified,
                    });
                }
            }
        }
        models
    }

    /// Scans LM Studio downloaded models directory.
    pub fn scan_lmstudio(environment: &PlatformEnvironment) -> Vec<LocalModelItem> {
        let lm_root = match Self::resolve_root("~/.cache/lm-studio/models", environment) {
            Some(root) => root,
            None => return Vec::new(),
        };

        let mut models = Vec::new();
        Self::collect_gguf_files(
            &lm_root,
            &lm_root,
            ModelSource::LmStudio,
            "lmstudio",
            &mut models,
        );
        models
    }

    /// Scans MLX model weights directory.
    pub fn scan_mlx(environment: &PlatformEnvironment) -> Vec<LocalModelItem> {
        let mlx_root = match Self::resolve_root("~/.cache/mlx", environment) {
            Some(root) => root,
            None => return Vec::new(),
        };

        let mut models = Vec::new();
        if let Ok(entries) = fs::read_dir(&mlx_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                let meta = match fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if meta.file_type().is_symlink() {
                    continue;
                }

                if meta.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let (size, _) = SizeCalculator::measure_path(&path, &[], environment);
                    let last_modified = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());

                    models.push(LocalModelItem {
                        id: format!("mlx.{}", name),
                        name,
                        source: ModelSource::Mlx,
                        path: path.to_string_lossy().to_string(),
                        size_bytes: size.reclaimable(),
                        format: Some("MLX 4-bit / 8-bit".to_string()),
                        parameter_size: None,
                        quantization: None,
                        last_modified,
                    });
                }
            }
        }
        models
    }

    fn collect_gguf_files(
        dir: &Path,
        root: &Path,
        source: ModelSource,
        prefix: &str,
        out: &mut Vec<LocalModelItem>,
    ) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let meta = match fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                // Anti-symlink protection: NEVER follow symlinks in local model scanning
                if meta.file_type().is_symlink() {
                    continue;
                }

                if meta.is_dir() {
                    Self::collect_gguf_files(&path, root, source, prefix, out);
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

                    out.push(LocalModelItem {
                        id: unique_id,
                        name: file_name,
                        source,
                        path: path.to_string_lossy().to_string(),
                        size_bytes: size,
                        format: Some("GGUF".to_string()),
                        parameter_size: None,
                        quantization: None,
                        last_modified,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LocalModelScanner;
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::{KnownFolder, PlatformEnvironment};
    use std::path::{Path, PathBuf};

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
            LocalModelScanner::scan_all_models(&environment).len(),
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
        assert!(
            LocalModelScanner::resolve_root("~/.ollama/models/manifests", &environment).is_none(),
            "the stated machine has no Ollama install"
        );
        assert_eq!(
            crate::signatures::SignatureLoader::expand_path(
                "~/.ollama/models/manifests",
                &environment
            ),
            Some(PathBuf::from(r"D:\Users\me\.ollama\models\manifests"))
        );
        assert!(
            LocalModelScanner::scan_all_models(&environment).is_empty(),
            "a stated Windows profile must not fall back to the host profile"
        );
    }

    #[test]
    fn missing_roots_yield_no_models_instead_of_host_paths() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix)
            .with_home(Path::new("/nonexistent-zenith-test-home"));
        assert!(LocalModelScanner::scan_all_models(&environment).is_empty());
    }
}
