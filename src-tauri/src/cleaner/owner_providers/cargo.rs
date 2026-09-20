//! The Cargo stores an owner-scoped provider owns.
//!
//! Cargo keeps three kinds of state under its home, and they are not the same
//! kind of object:
//!
//! * `registry/cache` holds the `.crate` archives Cargo downloaded. Each one is
//!   an immutable input that a build unpacks on demand, and nothing in the
//!   directory is written by anything but Cargo's own download path;
//! * `registry/src` holds the extracted trees. A build writes into them, Cargo
//!   reuses them across builds, and their contents are package-authored: a
//!   published crate may legitimately ship a `Cargo.lock`, a lockfile, a
//!   script, or a database-shaped fixture;
//! * `git/checkouts` and `git/db` hold working trees and bare repositories,
//!   which have their own locks, local modifications, and identity.
//!
//! One provider per store, because they do not share a mutation policy. The
//! archive provider removes the archives a measurement verified against its own
//! contract; the source and git providers enumerate and measure what they own
//! and state that Cargo decides when it goes. None of them authorizes another,
//! and none of them is a filename rule: the archive provider never looks inside
//! an archive, so what a crate ships is opaque to it, while the source provider
//! never removes anything at all.

use crate::safety::{SymlinkGuard, ToctouGuard};
use crate::models::PlatformKind;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use zenith_core::domain::cleanup::{
    OwnerProviderAuthorization, OwnerProviderExecution, OwnerProviderRefusal,
    OwnerProviderSelection, OwnerProviderUnit, OwnerStoreObservation, OwnerUnitMeasurement,
    OwnerUnitMeasurer, OwnerUnitObservation, OwnerUnitOutcome, ProviderStatus, RunningProcessPolicy,
    RunningProcessProbe,
};
use zenith_platform::PlatformEnvironment;

/// Which Cargo store one provider owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CargoStore {
    /// `registry/cache`: downloaded `.crate` archives, one directory per
    /// registry index.
    RegistryArchive,
    /// `registry/src`: extracted source trees, one directory per registry
    /// index.
    RegistrySource,
    /// `git/checkouts` and `git/db`: git dependencies Cargo cloned.
    Git,
}

impl CargoStore {
    /// The roots this store owns, relative to the Cargo home.
    fn relative_roots(self) -> &'static [&'static str] {
        match self {
            Self::RegistryArchive => &["registry/cache"],
            Self::RegistrySource => &["registry/src"],
            Self::Git => &["git/checkouts", "git/db"],
        }
    }

    /// Whether this store's units are removable by this build.
    fn removes_units(self) -> bool {
        matches!(self, Self::RegistryArchive)
    }

    /// The label a unit key carries when the store owns more than one root, so
    /// two roots cannot produce the same unit identity.
    fn root_label(relative_root: &str) -> &str {
        match relative_root.rsplit('/').next() {
            Some(label) => label,
            None => relative_root,
        }
    }
}

/// One store-specific mutation policy, implemented once per store.
pub struct CargoStoreProvider {
    id: &'static str,
    store: CargoStore,
    consequence: &'static str,
    process: Arc<dyn RunningProcessProbe>,
    measuring: Arc<dyn OwnerUnitMeasurer>,
}

/// `registry/cache`: downloaded archives.
pub struct CargoRegistryArchiveProvider(CargoStoreProvider);

/// `registry/src`: extracted source trees, advisory.
pub struct CargoRegistrySourceProvider(CargoStoreProvider);

/// `git/checkouts` and `git/db`: git dependencies, advisory.
pub struct CargoGitProvider(CargoStoreProvider);

impl CargoRegistryArchiveProvider {
    pub fn new(
        process: Arc<dyn RunningProcessProbe>,
        measuring: Arc<dyn OwnerUnitMeasurer>,
    ) -> Self {
        Self(CargoStoreProvider {
            id: "cargo.registry.archive",
            store: CargoStore::RegistryArchive,
            consequence: "The downloaded crate archives are removed. Cargo downloads an archive again the next time a build needs that dependency.",
            process,
            measuring,
        })
    }
}

impl CargoRegistrySourceProvider {
    pub fn new(
        process: Arc<dyn RunningProcessProbe>,
        measuring: Arc<dyn OwnerUnitMeasurer>,
    ) -> Self {
        Self(CargoStoreProvider {
            id: "cargo.registry.source",
            store: CargoStore::RegistrySource,
            consequence: "Cargo owns this store: it decides when an extracted source tree is stale and removes it itself. Zenith reports the bytes and does not remove them.",
            process,
            measuring,
        })
    }
}

impl CargoGitProvider {
    pub fn new(
        process: Arc<dyn RunningProcessProbe>,
        measuring: Arc<dyn OwnerUnitMeasurer>,
    ) -> Self {
        Self(CargoStoreProvider {
            id: "cargo.git",
            store: CargoStore::Git,
            consequence: "Cargo owns git checkouts and their databases: a checkout may hold local modifications and its own locks, so Cargo decides when it goes. Zenith reports the bytes and does not remove them.",
            process,
            measuring,
        })
    }
}

macro_rules! impl_provider {
    ($type:ty) => {
        impl crate::cleaner::owner_providers::OwnerScopedProvider for $type {
            fn id(&self) -> &'static str {
                self.0.id
            }

            fn platforms(&self) -> &'static [PlatformKind] {
                static PLATFORMS: [PlatformKind; 3] = [
                    PlatformKind::Macos,
                    PlatformKind::Windows,
                    PlatformKind::Linux,
                ];
                &PLATFORMS
            }

            fn consequence(&self) -> &'static str {
                self.0.consequence
            }

            fn requires_confirmation(&self) -> bool {
                true
            }

            fn scan(
                &self,
                environment: &PlatformEnvironment,
                guard: &RunningProcessPolicy,
            ) -> OwnerStoreObservation {
                self.0.read_store(environment, guard)
            }

            fn prepare(
                &self,
                environment: &PlatformEnvironment,
                guard: &RunningProcessPolicy,
                selections: &[OwnerProviderSelection],
            ) -> Result<OwnerProviderAuthorization, OwnerProviderRefusal> {
                self.0.prepare(environment, guard, selections)
            }

            fn execute(
                &self,
                environment: &PlatformEnvironment,
                authorization: &OwnerProviderAuthorization,
            ) -> OwnerProviderExecution {
                self.0.execute(environment, authorization)
            }
        }
    };
}

impl_provider!(CargoRegistryArchiveProvider);
impl_provider!(CargoRegistrySourceProvider);
impl_provider!(CargoGitProvider);

impl CargoStoreProvider {
    /// The roots this provider resolves in the stated environment.
    ///
    /// Resolution goes through the environment rather than the process: a
    /// provider that read `HOME` or `CARGO_HOME` for itself could enumerate a
    /// store the scan never mentioned, and every caller would have to know
    /// which of the two it was looking at.
    fn resolved_roots(&self, environment: &PlatformEnvironment) -> Result<Vec<PathBuf>, String> {
        let Some(cargo_home) = environment.cargo_home() else {
            return Err(
                "This environment does not resolve a Cargo home, so no Cargo store can be addressed"
                    .to_string(),
            );
        };
        Ok(self
            .store
            .relative_roots()
            .iter()
            .map(|relative| {
                relative
                    .split('/')
                    .fold(cargo_home.clone(), |path, part| path.join(part))
            })
            .filter(|root| root.starts_with(&cargo_home))
            .collect())
    }

    /// Reads every root this store owns and reports the units it found.
    fn read_store(
        &self,
        environment: &PlatformEnvironment,
        guard: &RunningProcessPolicy,
    ) -> OwnerStoreObservation {
        let roots = match self.resolved_roots(environment) {
            Ok(roots) => roots,
            Err(detail) => {
                return OwnerStoreObservation::refused(ProviderStatus::Unsupported, None, detail)
            }
        };
        let primary = roots.first().cloned();
        // The owner's own state decides whether its store may be read at all:
        // a cache Cargo is writing right now is not a set of stale archives.
        match self.process.running(guard) {
            None => {
                return OwnerStoreObservation::refused(
                    ProviderStatus::Blocked,
                    primary,
                    "The process table could not be read, so it cannot be proven that Cargo is idle",
                )
            }
            Some(running) if !running.is_empty() => {
                return OwnerStoreObservation::refused(
                    ProviderStatus::PrerequisiteNotMet,
                    primary,
                    format!(
                        "{} is running and holds its own store open; close it before cleaning Cargo caches",
                        running.join(", ")
                    ),
                )
            }
            Some(_) => {}
        }

        let mut units = Vec::new();
        let mut root_that_blocked = None;
        for root in &roots {
            match self.read_root(environment, root) {
                Ok(mut found) => units.append(&mut found),
                Err(reason) => {
                    root_that_blocked = Some((root.clone(), reason));
                    break;
                }
            }
        }
        if let Some((root, reason)) = root_that_blocked {
            return OwnerStoreObservation::refused(ProviderStatus::Blocked, Some(root), reason);
        }
        units.sort_by(|left, right| left.unit_key.cmp(&right.unit_key));
        OwnerStoreObservation::ready(primary, units)
    }

    /// Reads one root, or states why the whole store is refused.
    fn read_root(
        &self,
        environment: &PlatformEnvironment,
        root: &Path,
    ) -> Result<Vec<OwnerUnitObservation>, String> {
        let metadata = match fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            // A store that was never created holds nothing to clean. It is a
            // real answer, not a failure.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(zenith_platform::environment::describe_access_refusal(
                    environment,
                    root,
                    &error.to_string(),
                ))
            }
        };
        if SymlinkGuard::is_symlink(root) || !metadata.is_dir() {
            return Err(format!(
                "{} is not a directory Cargo wrote; refusing to enumerate through it",
                root.display()
            ));
        }

        let entries = fs::read_dir(root).map_err(|error| {
            zenith_platform::environment::describe_access_refusal(
                environment,
                root,
                &error.to_string(),
            )
        })?;
        let mut units = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    return Err(zenith_platform::environment::describe_access_refusal(
                        environment,
                        root,
                        &error.to_string(),
                    ))
                }
            };
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    return Err(zenith_platform::environment::describe_access_refusal(
                        environment,
                        &path,
                        &error.to_string(),
                    ))
                }
            };
            // Cargo's layout describes every child of these roots: one
            // directory per registry index, one per checkout or database. An
            // entry that is not a directory is something else's, and a store
            // whose contents are not what its owner's layout describes is a
            // store whose units cannot be told apart.
            if SymlinkGuard::is_symlink(&path) || !metadata.is_dir() {
                return Err(format!(
                    "{} is not a store directory Cargo's layout describes; the store is refused rather than partially enumerated",
                    path.display()
                ));
            }
            let unit_key = self.unit_key(root, &name);
            units.push(self.observe_unit(root, &unit_key, &path));
        }
        Ok(units)
    }

    /// The unit identity one child carries inside this store.
    ///
    /// A store with several roots prefixes the root's own name, so
    /// `git/checkouts/<name>` and `git/db/<name>` are different units.
    fn unit_key(&self, root: &Path, name: &str) -> String {
        if self.store.relative_roots().len() > 1 {
            let label = root
                .file_name()
                .map(|label| label.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("{label}/{name}")
        } else {
            name.to_string()
        }
    }

    /// Measures one unit and, for a removable store, verifies its contents.
    fn observe_unit(
        &self,
        _root: &Path,
        unit_key: &str,
        path: &Path,
    ) -> OwnerUnitObservation {
        let measurement = self.measuring.measure(path);
        if !self.store.removes_units() {
            // The owner decides when an advisory store's units go. An
            // incomplete measurement is stated with the bytes that were read.
            let detail = advisory_detail(&measurement, path);
            return OwnerUnitObservation::advisory(
                unit_key,
                path.to_path_buf(),
                measurement.logical_bytes,
                measurement.allocated_bytes,
                measurement.entry_count,
                detail,
            );
        }
        if !measurement.complete {
            return OwnerUnitObservation::blocked(
                unit_key,
                path.to_path_buf(),
                measurement.logical_bytes,
                measurement.allocated_bytes,
                measurement.entry_count,
                measurement.detail.unwrap_or_else(|| {
                    format!(
                        "The contents of {} could not be read completely",
                        path.display()
                    )
                }),
            );
        }
        match self.verify_archive_entries(path) {
            Ok((logical, allocated, count)) => OwnerUnitObservation::ready(
                unit_key,
                path.to_path_buf(),
                logical,
                allocated,
                count,
            ),
            Err(reason) => OwnerUnitObservation::blocked(
                unit_key,
                path.to_path_buf(),
                measurement.logical_bytes,
                measurement.allocated_bytes,
                measurement.entry_count,
                reason,
            ),
        }
    }

    /// Verifies that a unit directory holds only downloaded archives.
    ///
    /// This is the whole of the archive policy, and it never looks inside an
    /// archive: a crate's own contents are opaque package input, so a
    /// `Cargo.lock`, a `flake.lock`, a script, or a database fixture inside one
    /// is not a fact about the directory at all. What the directory must be is
    /// a set of regular files Cargo's download path wrote, each named as an
    /// archive and each actually a gzip stream.
    ///
    /// Anything else — an unexpected entry, a link, a special file, an archive
    /// whose bytes were replaced — refuses the whole unit. A partial removal
    /// would leave a store whose remaining contents nothing here could explain.
    fn verify_archive_entries(&self, unit: &Path) -> Result<(u64, u64, u64), String> {
        let entries = fs::read_dir(unit)
            .map_err(|error| format!("{} could not be read: {error}", unit.display()))?;
        let mut logical = 0u64;
        let mut allocated = 0u64;
        let mut count = 0u64;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("{} could not be read: {error}", unit.display()))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if SymlinkGuard::is_symlink(&path) || !metadata.is_file() {
                return Err(format!(
                    "{} is not a regular file Cargo's download path wrote; the unit is refused rather than partially removed",
                    path.display()
                ));
            }
            if !name.to_ascii_lowercase().ends_with(".crate") {
                return Err(format!(
                    "{} is not a downloaded crate archive; the unit is refused rather than partially removed",
                    path.display()
                ));
            }
            if !is_gzip_archive(&path) {
                return Err(format!(
                    "{} is named as a crate archive but its contents are not one; the unit is refused rather than partially removed",
                    path.display()
                ));
            }
            let measurement = self.measuring.measure(&path);
            logical = logical.saturating_add(measurement.logical_bytes);
            allocated = allocated.saturating_add(measurement.allocated_bytes);
            count = count.saturating_add(1);
        }
        Ok((logical, allocated, count))
    }

    /// Re-reads the store and authorizes the units the selection names.
    fn prepare(
        &self,
        environment: &PlatformEnvironment,
        guard: &RunningProcessPolicy,
        selections: &[OwnerProviderSelection],
    ) -> Result<OwnerProviderAuthorization, OwnerProviderRefusal> {
        let mut private_plan = OwnerProviderAuthorization {
            signature_id: String::new(),
            provider_id: self.id.to_string(),
            risk: crate::models::RiskTier::Rebuild,
            units: Vec::new(),
            refusals: Vec::new(),
            process_guard: guard.clone(),
            requires_confirmation: true,
        };
        if !self.store.removes_units() {
            return Err(OwnerProviderRefusal::for_selections(
                ProviderStatus::Unsupported,
                format!(
                    "Cargo owns `{}`; this build reports its bytes and does not remove them",
                    self.id
                ),
                selections
                    .iter()
                    .map(|selection| crate::models::OwnerUnitRefusal {
                        item_id: selection.item_id.clone(),
                        item_name: selection.name.clone(),
                        status: ProviderStatus::Unsupported,
                        reason: crate::models::CleanFailureReason::OwnerManaged,
                        detail: "Cargo decides when this store's contents are stale".to_string(),
                    })
                    .collect(),
            ));
        }
        let observation = self.read_store(environment, guard);
        if !observation.status.is_ready() {
            let detail = observation
                .detail
                .clone()
                .unwrap_or_else(|| observation.status.display_name().to_string());
            return Err(OwnerProviderRefusal::for_selections(
                observation.status,
                detail.clone(),
                selections
                    .iter()
                    .map(|selection| crate::models::OwnerUnitRefusal {
                        item_id: selection.item_id.clone(),
                        item_name: selection.name.clone(),
                        status: observation.status,
                        reason: crate::cleaner::owner_providers::refusal_reason(observation.status),
                        detail: detail.clone(),
                    })
                    .collect(),
            ));
        }

        for selection in selections {
            let found = observation.units.iter().find(|unit| {
                unit.path == selection.path && unit.state == zenith_core::domain::cleanup::OwnerUnitState::Ready
            });
            let Some(unit) = found else {
                // The store was re-read and this unit is not one the provider
                // may remove now. The refusal names why rather than falling
                // back to a generic deletion.
                let blocked = observation
                    .units
                    .iter()
                    .find(|candidate| candidate.path == selection.path);
                let (status, detail) = match blocked {
                    Some(blocked) => (
                        ProviderStatus::Blocked,
                        blocked.detail.clone().unwrap_or_else(|| {
                            format!("{} is not removable right now", blocked.path.display())
                        }),
                    ),
                    None => (
                        ProviderStatus::PrerequisiteNotMet,
                        format!(
                            "{} is no longer part of the store Cargo describes; scan again",
                            selection.path.display()
                        ),
                    ),
                };
                private_plan.refusals.push(crate::models::OwnerUnitRefusal {
                    item_id: selection.item_id.clone(),
                    item_name: selection.name.clone(),
                    status,
                    reason: crate::cleaner::owner_providers::refusal_reason(status),
                    detail,
                });
                continue;
            };
            let Some(identity) = ToctouGuard::capture(&unit.path) else {
                private_plan.refusals.push(crate::models::OwnerUnitRefusal {
                    item_id: selection.item_id.clone(),
                    item_name: selection.name.clone(),
                    status: ProviderStatus::Blocked,
                    reason: crate::models::CleanFailureReason::ProviderRefused,
                    detail: format!(
                        "The identity of {} could not be captured, so a later replacement could not be detected",
                        unit.path.display()
                    ),
                });
                continue;
            };
            private_plan.units.push(OwnerProviderUnit {
                item_id: selection.item_id.clone(),
                unit_key: unit.unit_key.clone(),
                name: selection.name.clone(),
                root: unit
                    .path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| unit.path.clone()),
                path: unit.path.clone(),
                identity,
                expected_bytes: unit.allocated_bytes,
                entry_count: unit.entry_count,
            });
        }

        if private_plan.units.is_empty() {
            let message = private_plan
                .refusals
                .first()
                .map(|refusal| refusal.detail.clone())
                .unwrap_or_else(|| "No selected unit is removable now".to_string());
            let status = private_plan
                .refusals
                .first()
                .map(|refusal| refusal.status)
                .unwrap_or(ProviderStatus::Blocked);
            return Err(OwnerProviderRefusal::for_selections(
                status,
                message,
                private_plan.refusals,
            ));
        }
        Ok(private_plan)
    }

    /// Re-verifies every authorized unit and removes its verified entries.
    fn execute(
        &self,
        environment: &PlatformEnvironment,
        authorization: &OwnerProviderAuthorization,
    ) -> OwnerProviderExecution {
        let process_state = self.process.running(&authorization.process_guard);
        let refusal = match process_state {
            None => Some(
                "The process table could not be read, so it cannot be proven that Cargo is idle"
                    .to_string(),
            ),
            Some(running) if !running.is_empty() => Some(format!(
                "{} is running and holds its own store open; close it before cleaning Cargo caches",
                running.join(", ")
            )),
            Some(_) => None,
        };
        let roots = self.resolved_roots(environment).unwrap_or_default();
        let mut outcomes = Vec::new();
        for unit in &authorization.units {
            let outcome = match (&refusal, root_matches(&roots, unit)) {
                (Some(detail), _) => OwnerUnitOutcome::refused(
                    unit.item_id.clone(),
                    unit.unit_key.clone(),
                    ProviderStatus::PrerequisiteNotMet,
                    detail.clone(),
                ),
                (None, Err(detail)) => OwnerUnitOutcome::refused(
                    unit.item_id.clone(),
                    unit.unit_key.clone(),
                    ProviderStatus::Blocked,
                    detail,
                ),
                (None, Ok(())) => self.remove_unit(environment, unit),
            };
            outcomes.push(outcome);
        }
        OwnerProviderExecution { units: outcomes }
    }

    /// Removes one unit's verified entries and reports what verification saw.
    fn remove_unit(
        &self,
        environment: &PlatformEnvironment,
        unit: &OwnerProviderUnit,
    ) -> OwnerUnitOutcome {
        // The unit must still be the object the plan captured before anything
        // inside it is touched: a replaced directory would otherwise redirect
        // every check below at a different tree.
        if let Err(error) = ToctouGuard::verify(&unit.path, &unit.identity) {
            return OwnerUnitOutcome::refused(
                unit.item_id.clone(),
                unit.unit_key.clone(),
                ProviderStatus::Blocked,
                format!(
                    "{} is no longer the directory the plan authorized ({error}); nothing was removed",
                    unit.path.display()
                ),
            );
        }
        if let Err(reason) = self.verify_archive_entries(&unit.path).map(|_| ()) {
            return OwnerUnitOutcome::refused(
                unit.item_id.clone(),
                unit.unit_key.clone(),
                ProviderStatus::Blocked,
                reason,
            );
        }

        let entries = match fs::read_dir(&unit.path) {
            Ok(entries) => entries,
            Err(error) => {
                return OwnerUnitOutcome::refused(
                    unit.item_id.clone(),
                    unit.unit_key.clone(),
                    ProviderStatus::Blocked,
                    zenith_platform::environment::describe_access_refusal(
                        environment,
                        &unit.path,
                        &error.to_string(),
                    ),
                )
            }
        };
        let mut reclaimed = 0u64;
        let mut failures = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    failures.push(error.to_string());
                    continue;
                }
            };
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let measurement = self.measuring.measure(&path);
            // Re-verify the entry immediately before unlinking it, so a file
            // swapped in after the unit was verified is not removed under the
            // verified one's name. Unlinking never follows a link, so a
            // replacement can only make this fail, never redirect it.
            let still_verified = !SymlinkGuard::is_symlink(&path)
                && name.to_ascii_lowercase().ends_with(".crate")
                && is_gzip_archive(&path);
            if !still_verified {
                failures.push(format!(
                    "{} changed after the unit was verified; it was left in place",
                    path.display()
                ));
                continue;
            }
            match fs::remove_file(&path) {
                Ok(()) => reclaimed = reclaimed.saturating_add(measurement.allocated_bytes),
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }

        // The unit directory holds nothing but the archives just removed, so a
        // non-empty directory here means something appeared while the unit was
        // being removed. It stays, and the outcome says so.
        let removed_directory = match fs::remove_dir(&unit.path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false,
        };
        if removed_directory && failures.is_empty() {
            return OwnerUnitOutcome::cleaned(unit.item_id.clone(), unit.unit_key.clone(), reclaimed);
        }
        let remaining = self.measuring.measure(&unit.path);
        let detail = if failures.is_empty() {
            format!(
                "{} still holds entries that appeared while the unit was being removed; they were left in place",
                unit.path.display()
            )
        } else {
            failures.join("; ")
        };
        let remaining_bytes = if removed_directory {
            0
        } else {
            remaining.allocated_bytes
        };
        OwnerUnitOutcome::partially_cleaned(
            unit.item_id.clone(),
            unit.unit_key.clone(),
            reclaimed,
            Some(remaining_bytes),
            detail,
        )
    }
}

/// Whether the planned unit still sits below a root this provider resolves now.
///
/// A plan states the store its units came from, and execution re-derives that
/// fact rather than trusting it: an environment whose Cargo home moved makes
/// the plan describe a store that is no longer there.
fn root_matches(roots: &[PathBuf], unit: &OwnerProviderUnit) -> Result<(), String> {
    if roots.iter().any(|root| unit.path.starts_with(root)) {
        return Ok(());
    }
    Err(format!(
        "{} is not below a Cargo store this environment resolves; refusing to mutate it",
        unit.path.display()
    ))
}

/// The detail an advisory unit states about why its owner keeps it.
fn advisory_detail(measurement: &OwnerUnitMeasurement, path: &Path) -> String {
    if measurement.complete {
        return "Cargo decides when this store's contents are stale; Zenith does not remove them"
            .to_string();
    }
    measurement.detail.clone().unwrap_or_else(|| {
        format!(
            "{} could not be measured completely, so its size is a lower bound",
            path.display()
        )
    })
}

/// Whether a file begins with the gzip magic every `.crate` archive begins
/// with.
///
/// The check exists so the unit's contents are evidence rather than a name: a
/// file named `something.crate` that is not a compressed archive was not
/// written by Cargo's download path, whatever it is called. Reading two bytes
/// is enough to tell, and it never decompresses package content.
fn is_gzip_archive(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 2];
    file.read_exact(&mut magic).is_ok() && magic == [0x1f, 0x8b]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleaner::owner_providers::OwnerScopedProvider;
    use crate::models::Category;
    use crate::signatures::SignatureRegistry;
    use std::sync::Arc;
    use tempfile::tempdir;
    use zenith_core::domain::cleanup::OwnerUnitState;
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::paths::SimulatedPaths;

    /// A process probe whose answer the test states.
    struct StatedProcess(Option<Vec<String>>);

    impl RunningProcessProbe for StatedProcess {
        fn running(&self, _guard: &RunningProcessPolicy) -> Option<Vec<String>> {
            self.0.clone()
        }
    }

    /// A measurement that mirrors what the filesystem reports for the fixture.
    struct FixtureMeasurement;

    impl OwnerUnitMeasurer for FixtureMeasurement {
        fn measure(&self, path: &Path) -> OwnerUnitMeasurement {
            // Deliberately simpler than the production measurer: the provider's
            // rules are about which paths and entries it accepts, not about how
            // many blocks a fixture occupies.
            let mut logical = 0u64;
            let mut count = 0u64;
            let mut pending = vec![path.to_path_buf()];
            while let Some(current) = pending.pop() {
                let Ok(metadata) = fs::symlink_metadata(&current) else {
                    return OwnerUnitMeasurement::partial(logical, logical, count, "unreadable");
                };
                if metadata.is_dir() {
                    match fs::read_dir(&current) {
                        Ok(entries) => {
                            for entry in entries.flatten() {
                                pending.push(entry.path());
                            }
                        }
                        Err(error) => {
                            return OwnerUnitMeasurement::partial(
                                logical,
                                logical,
                                count,
                                error.to_string(),
                            )
                        }
                    }
                } else {
                    logical = logical.saturating_add(metadata.len());
                    count = count.saturating_add(1);
                }
            }
            OwnerUnitMeasurement::complete(logical, logical, count)
        }
    }

    /// The Cargo home the fixture environment resolves: the profile's own
    /// `.cargo`, which is where Cargo puts its state when `CARGO_HOME` is
    /// unset.
    fn cargo_home(profile: &Path) -> PathBuf {
        profile.join(".cargo")
    }

    fn environment(profile: &Path) -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current()).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::current())
                .with_home(profile.to_path_buf()),
        ))
    }

    fn idle() -> Arc<dyn RunningProcessProbe> {
        Arc::new(StatedProcess(Some(Vec::new())))
    }

    fn measuring() -> Arc<dyn OwnerUnitMeasurer> {
        Arc::new(FixtureMeasurement)
    }

    /// A gzip stream, so a fixture archive is what the provider's contract
    /// says it is rather than a file that merely carries the name.
    fn gzip_bytes(payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0x03];
        // One stored DEFLATE block: the payload is carried verbatim, which is
        // enough for a stream whose header is what the contract checks.
        bytes.push(0x01);
        let len = payload.len() as u16;
        bytes.extend_from_slice(&len.to_le_bytes());
        bytes.extend_from_slice(&(!len).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(&[0u8; 4]);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes
    }

    fn fixture_archive(profile: &Path, registry: &str, crate_name: &str, payload: &[u8]) -> PathBuf {
        let unit = cargo_home(profile).join("registry/cache").join(registry);
        fs::create_dir_all(&unit).unwrap();
        let path = unit.join(crate_name);
        fs::write(&path, gzip_bytes(payload)).unwrap();
        path
    }

    fn archive_provider(process: Arc<dyn RunningProcessProbe>) -> CargoRegistryArchiveProvider {
        CargoRegistryArchiveProvider::new(process, measuring())
    }

    /// The archive provider enumerates one unit per registry directory, and its
    /// verification is about the *directory*: what a crate ships inside its
    /// archive is never inspected, so package-authored lockfiles and database
    /// fixtures are ordinary archive content.
    #[test]
    fn archive_units_are_verified_without_looking_inside_an_archive() {
        let fixture = tempdir().unwrap();
        let environment = environment(fixture.path());
        // A payload naming the files that motivated this design: they are
        // inside the archive, so the cache directory never sees them.
        let payload = b"Cargo.lock flake.lock build.sh state.sqlite package-lock.json";
        fixture_archive(
            fixture.path(),
            "index.crates.io-1949cf8c6b5b557f",
            "uds_windows-1.2.1.crate",
            payload,
        );
        fixture_archive(fixture.path(), "github.com-1ecc6299db9ec823", "git-dep-0.1.0.crate", payload);

        let provider = archive_provider(idle());
        let observation = provider.scan(&environment, &RunningProcessPolicy::none());

        assert_eq!(observation.status, ProviderStatus::Ready);
        assert_eq!(observation.units.len(), 2);
        assert!(observation.units.iter().all(|unit| unit.is_ready()));
        assert_eq!(
            observation.units[0].unit_key,
            "github.com-1ecc6299db9ec823",
            "units are ordered by key"
        );
        let bytes = observation
            .units
            .iter()
            .map(|unit| unit.allocated_bytes)
            .sum::<u64>();
        assert!(bytes > 0, "the units report measured bytes");
        assert_eq!(observation.units[0].entry_count, 1);
    }

    /// An entry the store's contract does not describe refuses the whole unit:
    /// an injected lock, a special file, a link, or a file named as an archive
    /// whose contents are not one.
    #[test]
    fn any_entry_the_contract_does_not_describe_refuses_the_unit() {
        for case in ["injected-lock", "special-file", "link", "not-an-archive"] {
            let fixture = tempdir().unwrap();
            let environment = environment(fixture.path());
            let good = fixture_archive(
                fixture.path(),
                "index.crates.io-1949cf8c6b5b557f",
                "good-1.0.0.crate",
                b"payload",
            );
            let unit = good.parent().unwrap().to_path_buf();
            match case {
                "injected-lock" => fs::write(unit.join("app.lock"), b"lock").unwrap(),
                "special-file" => fs::write(unit.join("server.pid"), b"pid").unwrap(),
                "link" => {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&good, unit.join("link.crate")).unwrap();
                    #[cfg(not(unix))]
                    fs::write(unit.join("link.crate"), gzip_bytes(b"payload")).unwrap();
                }
                "not-an-archive" => fs::write(unit.join("forged.crate"), b"not gzip").unwrap(),
                _ => unreachable!(),
            }

            let provider = archive_provider(idle());
            let observation = provider.scan(&environment, &RunningProcessPolicy::none());

            assert_eq!(observation.status, ProviderStatus::Ready, "{case}");
            assert_eq!(observation.units.len(), 1, "{case}");
            assert_eq!(
                observation.units[0].state,
                OwnerUnitState::Blocked,
                "{case} must refuse the unit"
            );
            assert!(
                observation.units[0]
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains(&unit.display().to_string())),
                "{case} names the entry it refused: {:?}",
                observation.units[0].detail
            );
        }
    }

    /// A store root that is an indirection, a file, or holds an entry that is
    /// not a store directory is refused as a store, not enumerated partially.
    #[test]
    fn a_root_that_is_not_the_store_cargo_wrote_is_refused() {
        let fixture = tempdir().unwrap();
        let cache = cargo_home(fixture.path()).join("registry/cache");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("stray.crate"), gzip_bytes(b"payload")).unwrap();

        let provider = archive_provider(idle());
        let observation = provider.scan(
            &environment(fixture.path()),
            &RunningProcessPolicy::none(),
        );
        assert_eq!(observation.status, ProviderStatus::Blocked);
        assert!(observation.units.is_empty());
        assert!(observation
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("stray.crate")));

        // A root replaced by a link is refused too, and its target is never
        // enumerated.
        let elsewhere = tempdir().unwrap();
        fs::create_dir_all(cargo_home(elsewhere.path()).join("registry/cache")).unwrap();
        let relocated = tempdir().unwrap();
        fs::create_dir_all(cargo_home(relocated.path()).join("registry")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                cargo_home(elsewhere.path()).join("registry/cache"),
                cargo_home(relocated.path()).join("registry/cache"),
            )
            .unwrap();
            let provider = archive_provider(idle());
            let observation = provider.scan(
                &environment(relocated.path()),
                &RunningProcessPolicy::none(),
            );
            assert_eq!(
                observation.status,
                ProviderStatus::Blocked,
                "a store root replaced by a link is refused"
            );
            assert!(observation.units.is_empty());
        }
    }

    /// The owner's running state decides whether the store may be read at all,
    /// and an unreadable process table fails closed rather than assuming idle.
    #[test]
    fn the_owner_running_or_unknown_state_refuses_the_store() {
        let fixture = tempdir().unwrap();
        let environment = environment(fixture.path());
        fixture_archive(
            fixture.path(),
            "index.crates.io-1949cf8c6b5b557f",
            "good-1.0.0.crate",
            b"payload",
        );
        let guard = RunningProcessPolicy::guarding(vec!["cargo".to_string()]);

        let busy = archive_provider(Arc::new(StatedProcess(Some(vec!["cargo".to_string()]))));
        let observation = busy.scan(&environment, &guard);
        assert_eq!(observation.status, ProviderStatus::PrerequisiteNotMet);
        assert!(observation.units.is_empty());

        let unknown = archive_provider(Arc::new(StatedProcess(None)));
        let observation = unknown.scan(&environment, &guard);
        assert_eq!(observation.status, ProviderStatus::Blocked);
        assert!(observation.units.is_empty());
    }

    /// Preparing re-reads the store: a selection that no longer matches what
    /// the store holds is refused by name, and a selection that does is
    /// authorized with a captured identity.
    #[test]
    fn preparing_re_reads_the_store_and_refuses_what_it_cannot_verify() {
        let fixture = tempdir().unwrap();
        let environment = environment(fixture.path());
        let archive = fixture_archive(
            fixture.path(),
            "index.crates.io-1949cf8c6b5b557f",
            "good-1.0.0.crate",
            b"payload",
        );
        let unit = archive.parent().unwrap().to_path_buf();
        let provider = archive_provider(idle());

        let prepared = provider
            .prepare(
                &environment,
                &RunningProcessPolicy::none(),
                &[OwnerProviderSelection {
                    item_id: "dev.cargo.registry.cache.index".to_string(),
                    name: "Cargo Registry Cache (index)".to_string(),
                    path: unit.clone(),
                    expected_bytes: 1,
                }],
            )
            .expect("a verified unit is authorized");
        assert_eq!(prepared.units.len(), 1);
        assert_eq!(prepared.units[0].path, unit);
        assert_eq!(
            prepared.units[0].unit_key,
            "index.crates.io-1949cf8c6b5b557f"
        );

        let refusal = provider
            .prepare(
                &environment,
                &RunningProcessPolicy::none(),
                &[OwnerProviderSelection {
                    item_id: "dev.cargo.registry.cache.other".to_string(),
                    name: "Cargo Registry Cache (other)".to_string(),
                    path: fixture.path().join("registry/cache/elsewhere"),
                    expected_bytes: 1,
                }],
            )
            .expect_err("a selection the store does not hold is refused");
        assert_eq!(refusal.refusals.len(), 1);
        assert_eq!(
            refusal.refusals[0].item_id,
            "dev.cargo.registry.cache.other"
        );
    }

    /// Execution removes the verified entries, reports the bytes its own
    /// measurement observed, and refuses a unit whose root no longer resolves.
    #[test]
    fn execution_removes_verified_entries_and_reports_measured_bytes() {
        let fixture = tempdir().unwrap();
        let environment = environment(fixture.path());
        let first = fixture_archive(
            fixture.path(),
            "index.crates.io-1949cf8c6b5b557f",
            "first-1.0.0.crate",
            b"first payload",
        );
        let unit = first.parent().unwrap().to_path_buf();
        fixture_archive(
            fixture.path(),
            "index.crates.io-1949cf8c6b5b557f",
            "second-2.0.0.crate",
            b"second payload",
        );
        let provider = archive_provider(idle());

        let prepared = provider
            .prepare(
                &environment,
                &RunningProcessPolicy::none(),
                &[OwnerProviderSelection {
                    item_id: "dev.cargo.registry.cache.index".to_string(),
                    name: "Cargo Registry Cache (index)".to_string(),
                    path: unit.clone(),
                    expected_bytes: 1,
                }],
            )
            .expect("the verified unit is authorized");
        let outcome = provider.execute(&environment, &prepared);

        assert_eq!(outcome.units.len(), 1);
        assert_eq!(outcome.units[0].status, ProviderStatus::Cleaned);
        assert!(outcome.reclaimed_bytes() > 0);
        assert_eq!(outcome.units[0].remaining_bytes, Some(0));
        assert!(!unit.exists(), "the emptied unit directory is removed");
        assert!(
            cargo_home(fixture.path()).join("registry/cache").exists(),
            "the store root stays where Cargo put it"
        );
    }

    /// A unit whose root moved out of the environment that planned it is
    /// refused before anything is touched.
    #[test]
    fn a_unit_outside_the_resolved_store_is_refused_at_execution() {
        let fixture = tempdir().unwrap();
        let stated = environment(fixture.path());
        let archive = fixture_archive(
            fixture.path(),
            "index.crates.io-1949cf8c6b5b557f",
            "good-1.0.0.crate",
            b"payload",
        );
        let unit = archive.parent().unwrap().to_path_buf();
        let provider = archive_provider(idle());
        let mut prepared = provider
            .prepare(
                &stated,
                &RunningProcessPolicy::none(),
                &[OwnerProviderSelection {
                    item_id: "dev.cargo.registry.cache.index".to_string(),
                    name: "Cargo registry".to_string(),
                    path: unit.clone(),
                    expected_bytes: 1,
                }],
            )
            .expect("the verified unit is authorized");

        // The environment now resolves a different Cargo home, so the plan
        // describes a store that is no longer here.
        let other = tempdir().unwrap();
        let other_environment = environment(other.path());
        let outcome = provider.execute(&other_environment, &prepared);
        assert_eq!(outcome.units[0].status, ProviderStatus::Blocked);
        assert_eq!(outcome.reclaimed_bytes(), 0);
        assert!(archive.exists(), "nothing was removed");

        // A replaced directory is refused as well.
        prepared.units[0].path = fixture.path().join("registry/cache/moved");
        fs::create_dir_all(&prepared.units[0].path).unwrap();
        let outcome = provider.execute(&stated, &prepared);
        assert_eq!(outcome.units[0].status, ProviderStatus::Blocked);
    }

    /// The advisory stores enumerate and measure what they own and refuse to
    /// remove anything: Cargo decides when their contents are stale.
    #[test]
    fn advisory_stores_enumerate_and_refuse_to_remove() {
        let fixture = tempdir().unwrap();
        let environment = environment(fixture.path());
        let source_unit = cargo_home(fixture.path())
            .join("registry/src/index.crates.io-1949cf8c6b5b557f/request-0.13.4");
        fs::create_dir_all(&source_unit).unwrap();
        fs::write(source_unit.join("Cargo.lock"), b"version = 3").unwrap();
        fs::write(source_unit.join("flake.lock"), b"locked").unwrap();
        let checkout = cargo_home(fixture.path())
            .join("git/checkouts/example-0123456789abcdef/abcdef0");
        fs::create_dir_all(&checkout).unwrap();
        fs::write(checkout.join("build.sh"), b"#!/bin/sh\n").unwrap();
        let database = cargo_home(fixture.path()).join("git/db/example-0123456789abcdef.git");
        fs::create_dir_all(&database).unwrap();
        fs::write(database.join("index.lock"), b"lock").unwrap();

        let source = CargoRegistrySourceProvider::new(idle(), measuring());
        let observation = source.scan(&environment, &RunningProcessPolicy::none());
        assert_eq!(observation.status, ProviderStatus::Ready);
        assert_eq!(observation.units.len(), 1);
        assert_eq!(observation.units[0].state, OwnerUnitState::Advisory);
        assert!(observation.units[0].allocated_bytes > 0);
        let refusal = source
            .prepare(
                &environment,
                &RunningProcessPolicy::none(),
                &[OwnerProviderSelection {
                    item_id: "dev.cargo.registry.src".to_string(),
                    name: "Cargo Registry Source".to_string(),
                    path: observation.units[0].path.clone(),
                    expected_bytes: 1,
                }],
            )
            .expect_err("an advisory store authorizes nothing");
        assert_eq!(
            refusal.refusals[0].reason,
            crate::models::CleanFailureReason::OwnerManaged
        );

        let git = CargoGitProvider::new(idle(), measuring());
        let observation = git.scan(&environment, &RunningProcessPolicy::none());
        assert_eq!(observation.units.len(), 2);
        assert_eq!(
            observation.units[0].unit_key, "checkouts/example-0123456789abcdef",
            "a multi-root store prefixes the unit key with its root"
        );
        assert_eq!(observation.units[1].unit_key, "db/example-0123456789abcdef.git");
        assert!(observation
            .units
            .iter()
            .all(|unit| unit.state == OwnerUnitState::Advisory));
        assert!(source_unit.join("flake.lock").exists());
        assert!(database.join("index.lock").exists());
    }

    /// The provider the catalog names is the one that runs, and each store's
    /// policy cannot authorize another's.
    #[test]
    fn the_catalog_binds_each_cargo_store_to_its_own_provider() {
        let fixture = tempdir().unwrap();
        let environment = environment(fixture.path());
        fixture_archive(
            fixture.path(),
            "index.crates.io-1949cf8c6b5b557f",
            "good-1.0.0.crate",
            b"payload",
        );
        let registry = SignatureRegistry::load_embedded_with(&environment).expect("catalog");
        let providers = crate::cleaner::owner_providers::OwnerProviderRegistry::new(vec![
            Arc::new(archive_provider(idle())),
            Arc::new(CargoRegistrySourceProvider::new(idle(), measuring())),
            Arc::new(CargoGitProvider::new(idle(), measuring())),
        ]);

        let items = providers.scan_items(
            &registry,
            Category::Developer,
            false,
            &[],
            &environment,
        );
        assert!(
            items
                .iter()
                .any(|item| item.signature_id == "dev.cargo.registry.cache"),
            "the archive store is enumerated through its provider"
        );
        assert!(
            items.iter().all(|item| item.signature_id != "dev.cargo.registry.src"),
            "an advisory store is not enumerated by the archive provider"
        );

        // The archive provider refuses a selection that names the source
        // store's root: a root is not a credential for another store.
        let source_root = cargo_home(fixture.path()).join("registry/src");
        let refusal = providers
            .prepare(
                &registry,
                "dev.cargo.registry.cache",
                &[OwnerProviderSelection {
                    item_id: "dev.cargo.registry.cache.index.crates.io".to_string(),
                    name: "Cargo Registry Source".to_string(),
                    path: source_root,
                    expected_bytes: 1,
                }],
                &environment,
            )
            .expect_err("the archive provider does not own the source store");
        assert_eq!(refusal.refusals.len(), 1);
        assert!(refusal.detail.contains("no longer part of the store"));
    }
}
