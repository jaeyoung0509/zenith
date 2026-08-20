use crate::models::{LocalModelItem, ModelSource, ZenithError};
use crate::models_inventory::LocalModelScanner;
use crate::safety::SafeTreeDeleter;
use crate::signatures::SignatureLoader;
use crate::tooling;
use std::path::{Path, PathBuf};

pub struct LocalModelManager;

impl LocalModelManager {
    pub fn delete_by_id(model_id: &str) -> Result<u64, ZenithError> {
        let models = LocalModelScanner::scan_all_models();
        let model = Self::resolve_by_id(&models, model_id)?;
        match model.source {
            ModelSource::Ollama => Self::delete_ollama(model),
            ModelSource::HuggingFace => {
                Self::delete_filesystem_model(model, "~/.cache/huggingface/hub")
            }
            ModelSource::LmStudio => {
                Self::delete_filesystem_model(model, "~/.cache/lm-studio/models")
            }
            ModelSource::Mlx => Self::delete_filesystem_model(model, "~/.cache/mlx"),
        }
    }

    fn resolve_by_id<'a>(
        models: &'a [LocalModelItem],
        model_id: &str,
    ) -> Result<&'a LocalModelItem, ZenithError> {
        models
            .iter()
            .find(|model| model.id == model_id)
            .ok_or_else(|| ZenithError::PathNotAllowed(format!("unknown model id: {model_id}")))
    }

    fn delete_ollama(model: &LocalModelItem) -> Result<u64, ZenithError> {
        let blobs_dir = SignatureLoader::expand_path("~/.ollama/models/blobs");
        let before_bytes = blobs_dir
            .as_ref()
            .map(|p| {
                crate::scanner::SizeCalculator::measure_path(p, &[])
                    .0
                    .reclaimable()
            })
            .unwrap_or(0);

        let mut cmd = tooling::command("ollama");
        cmd.args(Self::ollama_delete_args(model));
        let output = tooling::run_with_timeout(cmd, std::time::Duration::from_secs(15)).map_err(
            |error| {
                let err_str = error.to_string();
                if err_str.contains("No such file") || err_str.contains("not found") {
                    ZenithError::ToolUnavailable("ollama".into())
                } else {
                    ZenithError::ExternalCommandFailed(err_str)
                }
            },
        )?;
        if !output.status.success() {
            return Err(ZenithError::ExternalCommandFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }

        let after_bytes = blobs_dir
            .as_ref()
            .map(|p| {
                crate::scanner::SizeCalculator::measure_path(p, &[])
                    .0
                    .reclaimable()
            })
            .unwrap_or(0);

        let actual_reclaimed = before_bytes.saturating_sub(after_bytes);
        Ok(actual_reclaimed)
    }

    fn ollama_delete_args(model: &LocalModelItem) -> [&str; 2] {
        ["rm", model.name.as_str()]
    }

    fn delete_filesystem_model(
        model: &LocalModelItem,
        allowed_root: &str,
    ) -> Result<u64, ZenithError> {
        let root = SignatureLoader::expand_path(allowed_root)
            .ok_or_else(|| ZenithError::PathNotAllowed(allowed_root.into()))?;
        let path = PathBuf::from(&model.path);
        if !Self::is_directly_scoped(&path, &root) {
            return Err(ZenithError::PathNotAllowed(model.path.clone()));
        }

        // Ancestor symlink protection
        crate::safety::SymlinkGuard::validate_no_symlink_ancestors(&path, &root)?;

        let report = SafeTreeDeleter::delete_path(&path, &[]);
        if report.is_success() || report.reclaimed_bytes > 0 {
            Ok(report.reclaimed_bytes)
        } else {
            Err(ZenithError::Io(report.errors.join("; ")))
        }
    }

    fn is_directly_scoped(path: &Path, root: &Path) -> bool {
        path != root && path.starts_with(root)
    }
}

#[cfg(test)]
mod tests {
    use super::LocalModelManager;
    use crate::models::{LocalModelItem, ModelSource};
    use std::path::Path;

    fn model(id: &str, name: &str, path: &str) -> LocalModelItem {
        LocalModelItem {
            id: id.into(),
            name: name.into(),
            source: ModelSource::Ollama,
            path: path.into(),
            size_bytes: 42,
            format: None,
            parameter_size: None,
            quantization: None,
            last_modified: None,
        }
    }

    #[test]
    fn arbitrary_path_cannot_resolve_as_model_identity() {
        let models = vec![model("ollama.llama3:8b", "llama3:8b", "/manifest")];
        assert!(LocalModelManager::resolve_by_id(&models, "/Users/me/data").is_err());
    }

    #[test]
    fn ollama_delete_uses_model_name_not_manifest_path() {
        let item = model(
            "ollama.llama3:8b",
            "llama3:8b",
            "/Users/me/.ollama/models/manifests/library/llama3/8b",
        );
        assert_eq!(
            LocalModelManager::ollama_delete_args(&item),
            ["rm", "llama3:8b"]
        );
    }

    #[test]
    fn filesystem_models_must_be_below_their_adapter_root() {
        assert!(LocalModelManager::is_directly_scoped(
            Path::new("/Users/me/.cache/mlx/model"),
            Path::new("/Users/me/.cache/mlx")
        ));
        assert!(!LocalModelManager::is_directly_scoped(
            Path::new("/Users/me/Documents"),
            Path::new("/Users/me/.cache/mlx")
        ));
    }
}
