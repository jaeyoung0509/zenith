//! Reviewed removal of Homebrew's direct downloaded files.
//!
//! This is deliberately separate from `brew cleanup`: that command also
//! manages installed formula versions and does not promise to purge current
//! downloads. One regular file is one authorization unit. API metadata,
//! bootsnap data, links, nested directories, and unrecognized names stay.

use super::OwnerScopedProvider;
use crate::models::{
    CleanFailureReason, OwnerProviderRefusal, OwnerProviderSelection, OwnerProviderUnit,
    OwnerStoreObservation, OwnerUnitObservation, OwnerUnitOutcome, OwnerUnitState, PlatformKind,
    ProviderStatus,
};
use crate::safety::{Blacklist, SymlinkGuard, ToctouGuard};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use zenith_core::domain::cleanup::{
    OwnerProviderAuthorization, OwnerProviderExecution, OwnerUnitMeasurer, RunningProcessPolicy,
    RunningProcessProbe,
};
use zenith_platform::PlatformEnvironment;

pub struct HomebrewDownloadsProvider {
    process: Arc<dyn RunningProcessProbe>,
    measuring: Arc<dyn OwnerUnitMeasurer>,
}

impl HomebrewDownloadsProvider {
    pub fn new(
        process: Arc<dyn RunningProcessProbe>,
        measuring: Arc<dyn OwnerUnitMeasurer>,
    ) -> Self {
        Self { process, measuring }
    }

    fn root(environment: &PlatformEnvironment) -> Option<PathBuf> {
        environment
            .user_home()
            .map(|home| home.join("Library/Caches/Homebrew/downloads"))
    }

    fn read_store(
        &self,
        environment: &PlatformEnvironment,
        guard: &RunningProcessPolicy,
    ) -> OwnerStoreObservation {
        let Some(root) = Self::root(environment) else {
            return OwnerStoreObservation::refused(
                ProviderStatus::Unsupported,
                None,
                "This environment has no user home for the Homebrew cache",
            );
        };
        match self.process.running(guard) {
            None => {
                return OwnerStoreObservation::refused(
                    ProviderStatus::Blocked,
                    Some(root),
                    "The process table could not prove Homebrew is idle",
                )
            }
            Some(running) if !running.is_empty() => {
                return OwnerStoreObservation::refused(
                    ProviderStatus::PrerequisiteNotMet,
                    Some(root),
                    format!(
                        "Close {} before reviewing Homebrew downloads",
                        running.join(", ")
                    ),
                )
            }
            Some(_) => {}
        }
        let metadata = match fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return OwnerStoreObservation::ready(Some(root), Vec::new())
            }
            Err(error) => {
                return OwnerStoreObservation::refused(
                    ProviderStatus::Blocked,
                    Some(root),
                    format!("Homebrew downloads could not be inspected: {error}"),
                )
            }
        };
        if !metadata.is_dir() || SymlinkGuard::validate_anchored_path(&root, environment).is_err() {
            return OwnerStoreObservation::refused(
                ProviderStatus::Blocked,
                Some(root),
                "Homebrew downloads is not an ordinary directory under the stated home",
            );
        }
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) => {
                return OwnerStoreObservation::refused(
                    ProviderStatus::Blocked,
                    Some(root),
                    format!("Homebrew downloads could not be listed: {error}"),
                )
            }
        };
        let mut units = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    return OwnerStoreObservation::refused(
                        ProviderStatus::Blocked,
                        Some(root),
                        format!("Homebrew downloads listing was incomplete: {error}"),
                    )
                }
            };
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    return OwnerStoreObservation::refused(
                        ProviderStatus::Blocked,
                        Some(root),
                        format!("A Homebrew download could not be inspected: {error}"),
                    )
                }
            };
            let measurement = self.measuring.measure(&path);
            let ready = metadata.is_file()
                && !metadata.file_type().is_symlink()
                && download_name(&name)
                && single_link(&metadata)
                && measurement.complete;
            let unit = if !measurement.complete {
                OwnerUnitObservation::blocked(
                    &name,
                    path,
                    measurement.logical_bytes,
                    measurement.allocated_bytes,
                    measurement.entry_count,
                    measurement.detail.unwrap_or_else(|| {
                        "The Homebrew entry could not be measured completely".into()
                    }),
                )
            } else if ready {
                OwnerUnitObservation::ready(
                    &name,
                    path,
                    measurement.logical_bytes,
                    measurement.allocated_bytes,
                    1,
                )
            } else {
                OwnerUnitObservation::advisory(
                    &name,
                    path,
                    measurement.logical_bytes,
                    measurement.allocated_bytes,
                    measurement.entry_count,
                    "This entry is not a completely measured, single-linked Homebrew download file",
                )
            };
            units.push(unit);
        }
        OwnerStoreObservation::ready(Some(root), units)
    }

    fn prepare_units(
        &self,
        environment: &PlatformEnvironment,
        guard: &RunningProcessPolicy,
        selections: &[OwnerProviderSelection],
    ) -> Result<OwnerProviderAuthorization, OwnerProviderRefusal> {
        let observation = self.read_store(environment, guard);
        if !observation.status.is_ready() {
            return Err(OwnerProviderRefusal::for_selections(
                observation.status,
                observation
                    .detail
                    .unwrap_or_else(|| "Homebrew downloads are unavailable".into()),
                Vec::new(),
            ));
        }
        let root = observation.root.expect("ready store has root");
        let mut plan = OwnerProviderAuthorization {
            signature_id: String::new(),
            provider_id: self.id().to_string(),
            risk: crate::models::RiskTier::Rebuild,
            units: Vec::new(),
            refusals: Vec::new(),
            process_guard: guard.clone(),
            requires_confirmation: false,
        };
        for selection in selections {
            let found = observation.units.iter().find(|unit| {
                unit.path == selection.path
                    && unit.state == OwnerUnitState::Ready
                    && unit.allocated_bytes == selection.expected_bytes
            });
            let Some(unit) = found else {
                plan.refusals.push(crate::models::OwnerUnitRefusal {
                    item_id: selection.item_id.clone(),
                    item_name: selection.name.clone(),
                    status: ProviderStatus::Blocked,
                    reason: CleanFailureReason::ProviderRefused,
                    detail: "The selected Homebrew download changed or is no longer eligible; scan again".into(),
                });
                continue;
            };
            let Some(identity) = ToctouGuard::capture(&unit.path) else {
                plan.refusals.push(crate::models::OwnerUnitRefusal {
                    item_id: selection.item_id.clone(),
                    item_name: selection.name.clone(),
                    status: ProviderStatus::Blocked,
                    reason: CleanFailureReason::ProviderRefused,
                    detail: "The download's filesystem identity could not be captured".into(),
                });
                continue;
            };
            plan.units.push(OwnerProviderUnit {
                item_id: selection.item_id.clone(),
                unit_key: unit.unit_key.clone(),
                name: selection.name.clone(),
                root: root.clone(),
                path: unit.path.clone(),
                identity,
                expected_bytes: unit.allocated_bytes,
                entry_count: 1,
            });
        }
        if plan.units.is_empty() {
            return Err(OwnerProviderRefusal::for_selections(
                ProviderStatus::Blocked,
                "No selected Homebrew download is still eligible",
                plan.refusals,
            ));
        }
        Ok(plan)
    }

    fn remove_unit(
        &self,
        environment: &PlatformEnvironment,
        unit: &OwnerProviderUnit,
    ) -> OwnerUnitOutcome {
        let refuse = |detail: String| {
            OwnerUnitOutcome::refused(
                unit.item_id.clone(),
                unit.unit_key.clone(),
                ProviderStatus::Blocked,
                detail,
            )
        };
        let Some(root) = Self::root(environment) else {
            return refuse("The Homebrew download root is no longer available".into());
        };
        if unit.root != root || unit.path.parent() != Some(root.as_path()) {
            return refuse("The selected file is outside Homebrew's download root".into());
        }
        if let Err(error) = Blacklist::validate_with(&unit.path, environment) {
            return refuse(format!("The download path is protected: {error}"));
        }
        if let Err(error) = SymlinkGuard::validate_anchored_path(&unit.path, environment) {
            return refuse(format!("The download path is no longer safe: {error}"));
        }
        if let Err(error) =
            SymlinkGuard::validate_canonical_blacklist_strict(&unit.path, environment)
        {
            return refuse(format!("The resolved download path is protected: {error}"));
        }
        if let Err(error) = ToctouGuard::verify(&unit.path, &unit.identity) {
            return refuse(format!("The selected download changed: {error}"));
        }
        let name = unit
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let metadata = match fs::symlink_metadata(&unit.path) {
            Ok(metadata) => metadata,
            Err(error) => return refuse(format!("The download could not be inspected: {error}")),
        };
        if !download_name(name) || !metadata.is_file() || !single_link(&metadata) {
            return refuse(
                "The selected unit is no longer a single-linked Homebrew download".into(),
            );
        }
        let measurement = self.measuring.measure(&unit.path);
        if !measurement.complete || measurement.allocated_bytes != unit.expected_bytes {
            return refuse("The download measurement changed; scan again".into());
        }
        if let Err(error) = fs::remove_file(&unit.path) {
            return refuse(format!("Homebrew download removal failed: {error}"));
        }
        match fs::symlink_metadata(&unit.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                OwnerUnitOutcome::cleaned(
                    unit.item_id.clone(),
                    unit.unit_key.clone(),
                    measurement.allocated_bytes,
                )
            }
            _ => OwnerUnitOutcome::partially_cleaned(
                unit.item_id.clone(),
                unit.unit_key.clone(),
                0,
                None,
                "The path still exists after removal; its replacement was left in place",
            ),
        }
    }
}

impl OwnerScopedProvider for HomebrewDownloadsProvider {
    fn id(&self) -> &'static str {
        "homebrew.downloads"
    }
    fn platforms(&self) -> &'static [PlatformKind] {
        &[PlatformKind::Macos]
    }
    fn consequence(&self) -> &'static str {
        "Homebrew downloads these files again when a future installation needs them."
    }
    fn requires_confirmation(&self) -> bool {
        false
    }
    fn unit_label(&self, unit: &OwnerUnitObservation) -> String {
        let label = unit
            .unit_key
            .split_once("--")
            .map_or(unit.unit_key.as_str(), |(_, name)| name);
        let mut chars = label.chars();
        let short: String = chars.by_ref().take(60).collect();
        if chars.next().is_some() {
            format!("{short}…")
        } else {
            short
        }
    }
    fn scan(
        &self,
        environment: &PlatformEnvironment,
        guard: &RunningProcessPolicy,
    ) -> OwnerStoreObservation {
        self.read_store(environment, guard)
    }
    fn prepare(
        &self,
        environment: &PlatformEnvironment,
        guard: &RunningProcessPolicy,
        selections: &[OwnerProviderSelection],
    ) -> Result<OwnerProviderAuthorization, OwnerProviderRefusal> {
        self.prepare_units(environment, guard, selections)
    }
    fn execute(
        &self,
        environment: &PlatformEnvironment,
        authorization: &OwnerProviderAuthorization,
    ) -> OwnerProviderExecution {
        let running = self.process.running(&authorization.process_guard);
        let refusal = match running {
            None => Some("The process table could not prove Homebrew is idle".to_string()),
            Some(running) if !running.is_empty() => {
                Some(format!("Homebrew is running: {}", running.join(", ")))
            }
            Some(_) => None,
        };
        OwnerProviderExecution {
            units: authorization
                .units
                .iter()
                .map(|unit| match &refusal {
                    Some(detail) => OwnerUnitOutcome::refused(
                        unit.item_id.clone(),
                        unit.unit_key.clone(),
                        ProviderStatus::PrerequisiteNotMet,
                        detail.clone(),
                    ),
                    None => self.remove_unit(environment, unit),
                })
                .collect(),
        }
    }
}

fn download_name(name: &str) -> bool {
    let Some((hash, payload)) = name.split_once("--") else {
        return false;
    };
    hash.len() == 64
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && !payload.is_empty()
        && ![".incomplete", ".partial", ".tmp", ".lock"]
            .iter()
            .any(|suffix| payload.ends_with(suffix))
}

#[cfg(unix)]
fn single_link(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() == 1
}

#[cfg(not(unix))]
fn single_link(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::SizeCalculatorMeasurement;
    use zenith_core::domain::cleanup::RunningProcessProbe;

    struct Idle;
    impl RunningProcessProbe for Idle {
        fn running(&self, _guard: &RunningProcessPolicy) -> Option<Vec<String>> {
            Some(Vec::new())
        }
    }

    struct Busy;
    impl RunningProcessProbe for Busy {
        fn running(&self, _guard: &RunningProcessPolicy) -> Option<Vec<String>> {
            Some(vec!["brew".into()])
        }
    }

    fn fixture() -> (
        tempfile::TempDir,
        PlatformEnvironment,
        HomebrewDownloadsProvider,
        RunningProcessPolicy,
    ) {
        let temp = tempfile::tempdir().expect("fixture");
        let environment = PlatformEnvironment::native().with_home(temp.path());
        let provider =
            HomebrewDownloadsProvider::new(Arc::new(Idle), Arc::new(SizeCalculatorMeasurement));
        let guard = RunningProcessPolicy::guarding(vec!["brew".into()]);
        (temp, environment, provider, guard)
    }

    #[test]
    fn only_direct_downloads_are_reviewable_and_a_selected_file_is_removed() {
        let (_temp, environment, provider, guard) = fixture();
        let root = HomebrewDownloadsProvider::root(&environment).unwrap();
        fs::create_dir_all(&root).unwrap();
        let valid = root.join(format!("{}--archive.tar.gz", "a".repeat(64)));
        fs::write(&valid, b"downloaded bytes").unwrap();
        fs::write(root.join("unexpected.txt"), b"keep").unwrap();
        let observation = provider.scan(&environment, &guard);
        assert_eq!(observation.units.len(), 2);
        let ready = observation
            .units
            .iter()
            .find(|unit| unit.path == valid)
            .unwrap();
        assert_eq!(ready.state, OwnerUnitState::Ready);
        assert!(observation
            .units
            .iter()
            .any(|unit| unit.state == OwnerUnitState::Advisory));
        let selection = OwnerProviderSelection {
            item_id: "download-1".into(),
            name: "archive".into(),
            path: valid.clone(),
            expected_bytes: ready.allocated_bytes,
        };
        let plan = provider
            .prepare(&environment, &guard, &[selection])
            .unwrap();
        let outcome = provider.execute(&environment, &plan);
        assert_eq!(outcome.units.len(), 1);
        assert_eq!(outcome.units[0].status, ProviderStatus::Cleaned);
        assert!(!valid.exists());
        assert!(root.join("unexpected.txt").exists());
    }

    #[test]
    fn replacement_after_prepare_is_refused() {
        let (_temp, environment, provider, guard) = fixture();
        let root = HomebrewDownloadsProvider::root(&environment).unwrap();
        fs::create_dir_all(&root).unwrap();
        let valid = root.join(format!("{}--archive.tar.gz", "a".repeat(64)));
        fs::write(&valid, b"first").unwrap();
        let ready = provider.scan(&environment, &guard).units.remove(0);
        let selection = OwnerProviderSelection {
            item_id: "download-1".into(),
            name: "archive".into(),
            path: valid.clone(),
            expected_bytes: ready.allocated_bytes,
        };
        let plan = provider
            .prepare(&environment, &guard, &[selection])
            .unwrap();
        fs::remove_file(&valid).unwrap();
        fs::write(&valid, b"replacement").unwrap();
        let outcome = provider.execute(&environment, &plan);
        assert_eq!(outcome.units[0].status, ProviderStatus::Blocked);
        assert!(valid.exists());
    }

    #[test]
    fn a_running_owner_blocks_download_review() {
        let (_temp, environment, _provider, guard) = fixture();
        let provider =
            HomebrewDownloadsProvider::new(Arc::new(Busy), Arc::new(SizeCalculatorMeasurement));
        let observation = provider.scan(&environment, &guard);
        assert_eq!(observation.status, ProviderStatus::PrerequisiteNotMet);
    }

    #[test]
    fn a_symlinked_download_is_never_ready() {
        let (temp, environment, provider, guard) = fixture();
        let root = HomebrewDownloadsProvider::root(&environment).unwrap();
        fs::create_dir_all(&root).unwrap();
        let outside = temp.path().join("keep");
        fs::write(&outside, b"protected").unwrap();
        let linked = root.join(format!("{}--archive.tar.gz", "b".repeat(64)));
        std::os::unix::fs::symlink(&outside, linked).unwrap();
        let observation = provider.scan(&environment, &guard);
        assert_eq!(observation.units.len(), 1);
        assert_ne!(observation.units[0].state, OwnerUnitState::Ready);
        assert_eq!(fs::read(outside).unwrap(), b"protected");
    }
}
