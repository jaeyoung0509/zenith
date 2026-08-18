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
        let output = tooling::command("ollama")
            .args(Self::ollama_delete_args(model))
            .output()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    ZenithError::ToolUnavailable("ollama".into())
                } else {
                    ZenithError::ExternalCommandFailed(error.to_string())
                }
            })?;
        if !output.status.success() {
            return Err(ZenithError::ExternalCommandFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        Ok(model.size_bytes)
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
        SafeTreeDeleter::delete_path(&path, &[]).map_err(ZenithError::from)
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
