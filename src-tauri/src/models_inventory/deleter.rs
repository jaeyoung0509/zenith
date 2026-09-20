use crate::models::{LocalModelItem, ModelSource, ZenithError};
use crate::models_inventory::LocalModelScanner;
use crate::safety::{Blacklist, SafeTreeDeleter, SymlinkGuard, TreeDeleteReport};
use crate::signatures::SignatureLoader;
use crate::tooling;
use std::path::{Path, PathBuf};
use zenith_platform::PlatformEnvironment;

/// A local model path that passed model-inventory scope and symlink validation.
///
/// The type is minted only inside this module, and [`Self::validate`] resolves
/// the allowed root from the model's own source, so a caller cannot present a
/// root of its choosing and thereby turn model-inventory authority into general
/// filesystem authority. `SafeTreeDeleter` accepts it only through
/// `FilesystemDeleteAuthority`.
#[derive(Debug, Clone)]
pub struct ValidatedModelTarget {
    path: PathBuf,
}

impl ValidatedModelTarget {
    /// Validates that `path` is directly scoped under the managed root of
    /// `source`, is not blacklisted, and contains no intermediate symlink
    /// ancestors.
    fn validate(
        source: ModelSource,
        path: &Path,
        environment: &PlatformEnvironment,
    ) -> Result<Self, ZenithError> {
        let managed_root = source
            .managed_root()
            .ok_or_else(|| ZenithError::PathNotAllowed(path.to_string_lossy().to_string()))?;
        let allowed_root = SignatureLoader::expand_path(managed_root, environment)
            .ok_or_else(|| ZenithError::PathNotAllowed(managed_root.to_string()))?;

        if path == allowed_root || !path.starts_with(&allowed_root) {
            return Err(ZenithError::PathNotAllowed(
                path.to_string_lossy().to_string(),
            ));
        }

        Blacklist::validate_with(path, environment)?;
        SymlinkGuard::validate_canonical_blacklist_strict(path, environment)?;
        SymlinkGuard::validate_no_symlink_ancestors(path, &allowed_root, environment)?;

        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

pub struct LocalModelManager;

impl LocalModelManager {
    pub fn delete_by_id(
        environment: &PlatformEnvironment,
        model_id: &str,
    ) -> Result<Option<u64>, ZenithError> {
        let inventory = LocalModelScanner::scan_all_models(environment);
        let model = Self::resolve_by_id(&inventory.items, model_id)?;
        match model.source {
            // A CLI-owned model is removed through the CLI; only a source with a
            // managed filesystem root reaches the deletion primitive.
            ModelSource::Ollama => Self::delete_ollama(environment, model),
            source => Self::delete_filesystem_model(environment, model, source),
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

    /// Deletes one model through the owning CLI.
    ///
    /// The returned amount is `None` when either measurement of the blob store
    /// was incomplete: the model was deleted, and the caller reports that the
    /// reclaimed bytes are unknown instead of subtracting two partial numbers.
    fn delete_ollama(
        environment: &PlatformEnvironment,
        model: &LocalModelItem,
    ) -> Result<Option<u64>, ZenithError> {
        let blobs_dir = SignatureLoader::expand_path("~/.ollama/models/blobs", environment);
        let before = blobs_dir
            .as_ref()
            .map(|p| crate::scanner::SizeCalculator::measure_path_logged(p, &[], environment));

        let mut cmd = tooling::command("ollama");
        cmd.args(Self::ollama_delete_args(model));
        let output =
            zenith_platform::subprocess::run_with_timeout(cmd, std::time::Duration::from_secs(15))
                .map_err(|error| {
                    let err_str = error.to_string();
                    if err_str.contains("No such file") || err_str.contains("not found") {
                        ZenithError::ToolUnavailable("ollama".into())
                    } else {
                        ZenithError::ExternalCommandFailed(err_str)
                    }
                })?;
        if !output.status.success() {
            let err_str = String::from_utf8_lossy(&output.stderr).trim().to_string();
            crate::diagnostics::log_error("models", &err_str);
            return Err(ZenithError::ExternalCommandFailed(
                crate::diagnostics::sanitize_log(&err_str),
            ));
        }

        let after = blobs_dir
            .as_ref()
            .map(|p| crate::scanner::SizeCalculator::measure_path_logged(p, &[], environment));

        Ok(match (before, after) {
            (Some(before), Some(after)) => crate::scanner::size::reclaimed_between(&before, &after),
            _ => None,
        })
    }

    fn ollama_delete_args(model: &LocalModelItem) -> [&str; 2] {
        ["rm", model.name.as_str()]
    }

    fn delete_filesystem_model(
        environment: &PlatformEnvironment,
        model: &LocalModelItem,
        source: ModelSource,
    ) -> Result<Option<u64>, ZenithError> {
        let path = PathBuf::from(&model.path);
        let validated = ValidatedModelTarget::validate(source, &path, environment)?;
        let report = SafeTreeDeleter::delete_path_validated(&validated, environment);
        Self::filesystem_delete_result(report)
    }

    fn filesystem_delete_result(report: TreeDeleteReport) -> Result<Option<u64>, ZenithError> {
        if report.is_success() {
            // The tree deleter's own accounting is exact: it reports what it
            // removed, not a difference between two measurements.
            Ok(Some(report.reclaimed_bytes))
        } else {
            let detail = report.errors.join("; ");
            let message = if report.reclaimed_bytes > 0 {
                format!(
                    "Model deletion was partial after reclaiming {} bytes: {detail}",
                    report.reclaimed_bytes
                )
            } else {
                detail
            };
            Err(ZenithError::Io(message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LocalModelManager, ValidatedModelTarget};
    use crate::models::{LocalModelItem, ModelSource, ZenithError};
    use crate::safety::TreeDeleteReport;
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::PlatformEnvironment;

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
            quality: crate::models::ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entries: 0,
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
    fn filesystem_models_must_be_below_their_source_managed_root() {
        let dir = tempfile::tempdir().unwrap();
        // The root is where the source keeps its blobs, under the stated
        // profile: `~/.cache/mlx` for the MLX source.
        let root = dir.path().join(".cache/mlx");
        let model = root.join("model");
        std::fs::create_dir_all(&model).unwrap();
        let outside = dir.path().join("Documents");
        std::fs::create_dir_all(&outside).unwrap();

        // The allowed root is the model source's own managed root, resolved
        // through the stated profile: no caller supplies one.
        let environment = zenith_platform::PlatformEnvironment::simulated(
            zenith_platform::path_algebra::PathFlavor::current(),
        )
        .with_home(dir.path());
        assert!(ValidatedModelTarget::validate(ModelSource::Mlx, &model, &environment).is_ok());
        assert!(ValidatedModelTarget::validate(ModelSource::Mlx, &outside, &environment).is_err());
        assert!(ValidatedModelTarget::validate(ModelSource::Mlx, &root, &environment).is_err());
        // A source whose deletion runs through its CLI has no filesystem root
        // to validate against, so it can never mint filesystem authority.
        assert!(ValidatedModelTarget::validate(ModelSource::Ollama, &model, &environment).is_err());
    }

    #[test]
    fn a_partial_filesystem_delete_is_not_reported_as_success() {
        let result = LocalModelManager::filesystem_delete_result(TreeDeleteReport {
            reclaimed_bytes: 42,
            deleted_files: 1,
            skipped_files: 1,
            errors: vec!["locked shard".to_string()],
            os_error_codes: vec![],
            protect_structured_state: false,
            allow_cargo_registry_contents: false,
        });

        let error = result.expect_err("a partial delete must remain a failure");
        let message = error.to_string();
        assert!(message.contains("partial"), "{message}");
        assert!(message.contains("42 bytes"), "{message}");
        assert!(message.contains("locked shard"), "{message}");
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
            LocalModelManager::delete_filesystem_model(&environment, &item, ModelSource::Mlx)
                .expect("a model under the stated root is deletable");
        assert!(
            reclaimed.is_some_and(|bytes| bytes > 0),
            "the deleted file's bytes are reported"
        );
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
        let error =
            LocalModelManager::delete_filesystem_model(&environment, &item, ModelSource::Mlx)
                .expect_err("a path outside the adapter root must be refused");
        assert!(matches!(error, ZenithError::PathNotAllowed(_)));
        assert!(outside.path().exists());
    }
}
