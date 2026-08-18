use crate::models::{LocalModelItem, ModelSource, ZenithError};
use crate::scanner::SizeCalculator;
use crate::signatures::SignatureLoader;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct LocalModelScanner;

impl LocalModelScanner {
    /// Discovers all local models across Ollama, HuggingFace Hub, LM Studio, and Apple MLX.
    pub fn scan_all_models() -> Vec<LocalModelItem> {
        let mut models = Vec::new();

        // 1. Ollama models
        models.extend(Self::scan_ollama());

        // 2. HuggingFace Hub models
        models.extend(Self::scan_huggingface());

        // 3. LM Studio models
        models.extend(Self::scan_lmstudio());

        // 4. Apple MLX models
        models.extend(Self::scan_mlx());

        models
    }

    /// Scans Ollama manifest directory to identify installed models and their sizes.
    pub fn scan_ollama() -> Vec<LocalModelItem> {
        let manifests_root = match SignatureLoader::expand_path("~/.ollama/models/manifests") {
            Some(p) if p.exists() => p,
            _ => return Vec::new(),
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
    pub fn scan_huggingface() -> Vec<LocalModelItem> {
        let hf_root = match SignatureLoader::expand_path("~/.cache/huggingface/hub") {
            Some(p) if p.exists() => p,
            _ => return Vec::new(),
        };

        let mut models = Vec::new();
        if let Ok(entries) = fs::read_dir(&hf_root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("models--") {
                    let clean_name = name.trim_start_matches("models--").replace("--", "/");
                    let path = entry.path();
                    let (size, _) = SizeCalculator::measure_path(&path, &[]);
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
    pub fn scan_lmstudio() -> Vec<LocalModelItem> {
        let lm_root = match SignatureLoader::expand_path("~/.cache/lm-studio/models") {
            Some(p) if p.exists() => p,
            _ => return Vec::new(),
        };

        let mut models = Vec::new();
        Self::collect_gguf_files(&lm_root, ModelSource::LmStudio, "lmstudio", &mut models);
        models
    }

    /// Scans MLX model weights directory.
    pub fn scan_mlx() -> Vec<LocalModelItem> {
        let mlx_root = match SignatureLoader::expand_path("~/.cache/mlx") {
            Some(p) if p.exists() => p,
            _ => return Vec::new(),
        };

        let mut models = Vec::new();
        if let Ok(entries) = fs::read_dir(&mlx_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let (size, _) = SizeCalculator::measure_path(&path, &[]);
                    let last_modified = fs::metadata(&path)
                        .ok()
                        .and_then(|m| m.modified().ok())
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
        source: ModelSource,
        prefix: &str,
        out: &mut Vec<LocalModelItem>,
    ) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::collect_gguf_files(&path, source, prefix, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("gguf") {
                    let file_name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    let last_modified = fs::metadata(&path)
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());

                    out.push(LocalModelItem {
                        id: format!("{}.{}", prefix, file_name),
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

    /// Deletes a specific local model after safety verification.
    pub fn delete_model(model_path_str: &str) -> Result<u64, ZenithError> {
        let path = PathBuf::from(model_path_str);

        // 1. Safety validation
        crate::safety::Blacklist::validate(&path)?;

        if !path.exists() {
            return Err(ZenithError::Io(format!(
                "Path {} not found",
                model_path_str
            )));
        }

        // 2. Perform deletion
        let (size, _) = SizeCalculator::measure_path(&path, &[]);
        let bytes = size.reclaimable();

        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }

        Ok(bytes)
    }
}
