use crate::models::{LocalModelItem, ModelSource, ZenithError};
use crate::models_inventory::LocalModelScanner;
use crate::platform::PlatformEnvironment;
use crate::safety::SafeTreeDeleter;
use crate::signatures::SignatureLoader;
use crate::tooling;
use std::path::{Path, PathBuf};

pub struct LocalModelManager;

impl LocalModelManager {
    pub fn delete_by_id(
        environment: &PlatformEnvironment,
        model_id: &str,
    ) -> Result<u64, ZenithError> {
        let models = LocalModelScanner::scan_all_models(environment);
        let model = Self::resolve_by_id(&models, model_id)?;
        match model.source {
            ModelSource::Ollama => Self::delete_ollama(environment, model),
            ModelSource::HuggingFace => {
                Self::delete_filesystem_model(environment, model, "~/.cache/huggingface/hub")
            }
            ModelSource::LmStudio => {
                Self::delete_filesystem_model(environment, model, "~/.cache/lm-studio/models")
            }
            ModelSource::Mlx => Self::delete_filesystem_model(environment, model, "~/.cache/mlx"),
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

    fn delete_ollama(
        environment: &PlatformEnvironment,
        model: &LocalModelItem,
    ) -> Result<u64, ZenithError> {
        let blobs_dir = SignatureLoader::expand_path("~/.ollama/models/blobs", environment);
        let before_bytes = blobs_dir
            .as_ref()
            .map(|p| {
                crate::scanner::SizeCalculator::measure_path_logged(p, &[], environment)
                    .size
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
            let err_str = String::from_utf8_lossy(&output.stderr).trim().to_string();
            crate::diagnostics::log_error("models", &err_str);
            return Err(ZenithError::ExternalCommandFailed(
                crate::diagnostics::sanitize_log(&err_str),
            ));
        }

        let after_bytes = blobs_dir
            .as_ref()
            .map(|p| {
                crate::scanner::SizeCalculator::measure_path_logged(p, &[], environment)
                    .size
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
        environment: &PlatformEnvironment,
        model: &LocalModelItem,
        allowed_root: &str,
    ) -> Result<u64, ZenithError> {
        let root = SignatureLoader::expand_path(allowed_root, environment)
            .ok_or_else(|| ZenithError::PathNotAllowed(allowed_root.into()))?;
        let path = PathBuf::from(&model.path);
        if !Self::is_directly_scoped(&path, &root) {
            return Err(ZenithError::PathNotAllowed(model.path.clone()));
        }

        // Ancestor symlink protection
        crate::safety::SymlinkGuard::validate_no_symlink_ancestors(&path, &root, environment)?;

        let report = SafeTreeDeleter::delete_path(&path, &[], environment);
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
    use crate::models::{LocalModelItem, ModelSource, ZenithError};
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::PlatformEnvironment;
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

    #[test]
    fn the_adapter_root_comes_from_the_stated_environment() {
        let stated_home = tempfile::tempdir().unwrap();
        let model_dir = stated_home.path().join(".cache/mlx/zenith-probe");
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join("weights.npz"), vec![7u8; 2_048]).unwrap();

        let mut item = model("mlx.zenith-probe", "zenith-probe", "");
        item.source = ModelSource::Mlx;
        item.path = model_dir.to_string_lossy().into_owned();

        // The scope root is the stated profile's `.cache/mlx`, not the host's.
        let environment =
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(stated_home.path());
        let reclaimed =
            LocalModelManager::delete_filesystem_model(&environment, &item, "~/.cache/mlx")
                .expect("a model under the stated root is deletable");
        assert!(reclaimed > 0, "the deleted file's bytes are reported");
        assert!(
            !model_dir.exists(),
            "the model under the stated root is removed"
        );
    }

    #[test]
    fn a_model_outside_the_stated_root_is_refused() {
        let stated_home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mut item = model("mlx.outside", "outside", "");
        item.source = ModelSource::Mlx;
        item.path = outside
            .path()
            .join(".cache/mlx/zeenith-probe")
            .to_string_lossy()
            .into_owned();

        let environment =
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(stated_home.path());
        let error = LocalModelManager::delete_filesystem_model(&environment, &item, "~/.cache/mlx")
            .expect_err("a path outside the adapter root must be refused");
        assert!(matches!(error, ZenithError::PathNotAllowed(_)));
        assert!(outside.path().exists());
    }
}
