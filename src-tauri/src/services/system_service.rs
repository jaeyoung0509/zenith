//! System runtime application service.
//!
//! Everything the dashboard's system surfaces need that is not a window, a
//! tray, or an IPC shape: memory and disk observations, Docker status and
//! pruning, local-model inventory and deletion, Keep Awake, development-port
//! leases, diagnostics, and the persisted preferences whose save is one use
//! case rather than a handler's sequence of writes.
//!
//! The service owns the raw workflow state those surfaces share — the memory
//! sampler and its termination leases, the development-port store, and the
//! Docker status observation — so no handler can hold a lease store or decide
//! when a mutation may start. Capability gates, execution budgets, and the
//! storage operation gate are applied here, which is what keeps them
//! independent of the Tauri capability files.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zenith_platform::{PlatformCapabilitiesProvider, PlatformEnvironment};

use crate::docker::adapter::ContainerHost;
use crate::docker::DockerAdapter;
use crate::execution_budget::ExecutionBudgets;
use crate::metrics::{
    CpuSampler, DiskMetricsCollector, MemoryInspector, MemorySampler, MemoryTerminationStore,
};
use crate::models::{
    AwakeBehavior, AwakeRule, AwakeState, BatteryMetrics, CapabilityAccess, CpuMetrics,
    DevelopmentListener, DiagnosticsSnapshot, DiskMetrics, DiskVolume, DockerStatus,
    LocalModelInventory, MemoryMetrics, MemoryTerminationMode, MemoryTerminationResult,
    PlatformCapabilities, PlatformFeature, ReleaseDevelopmentListenerResult, ReleaseMode,
    ZenithSettings,
};
use crate::models_inventory::{LocalModelManager, LocalModelScanner};
use crate::operation_gate::StorageOperationGate;
use crate::power::{BatteryProvider, KeepAwakeManager};
use crate::services::desktop_notifications::DesktopNotifications;
use crate::services::settings_service::{
    SettingsAuthority, SettingsChange, SettingsChangeReaction,
};

/// How long a Docker status observation answers before it is re-read.
const DOCKER_STATUS_TTL: Duration = Duration::from_secs(3);

/// The shared Docker status observation.
///
/// A storage mutation can change the runtime's state, so the pruning paths
/// invalidate the observation: the next read fetches instead of answering from
/// before the mutation. Both the status command and the cleanup workflow hold
/// the same handle, which is why the type exists rather than a bare mutex.
pub struct DockerStatusCache {
    entry: Mutex<Option<(DockerStatus, Instant)>>,
}

impl DockerStatusCache {
    pub(crate) fn new() -> Self {
        Self {
            entry: Mutex::new(None),
        }
    }

    /// The observation, when it was taken less than `ttl` ago.
    pub fn fresh(&self, ttl: Duration) -> Option<DockerStatus> {
        let guard = self
            .entry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .as_ref()
            .filter(|(_, fetched_at)| fetched_at.elapsed() < ttl)
            .map(|(status, _)| status.clone())
    }

    pub fn publish(&self, status: DockerStatus) {
        let mut guard = self
            .entry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some((status, Instant::now()));
    }

    /// Drops the observation, so the next read cannot answer from a state the
    /// caller may have just changed.
    pub fn invalidate(&self) {
        let mut guard = self
            .entry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = None;
    }
}

pub struct SystemService {
    environment: Arc<PlatformEnvironment>,
    container_host: ContainerHost,
    platform_capabilities: Arc<dyn PlatformCapabilitiesProvider>,
    operation_gate: StorageOperationGate,
    budgets: Arc<ExecutionBudgets>,
    memory_sampler: Arc<MemorySampler>,
    memory_leases: Arc<Mutex<MemoryTerminationStore>>,
    cpu_sampler: Arc<CpuSampler>,
    battery_provider: Arc<dyn BatteryProvider>,
    awake: Arc<KeepAwakeManager>,
    docker_status: Arc<DockerStatusCache>,
    dev_ports: Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
    settings: Arc<SettingsAuthority>,
    settings_reaction: Arc<dyn SettingsChangeReaction>,
}

impl SystemService {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        environment: Arc<PlatformEnvironment>,
        container_host: ContainerHost,
        platform_capabilities: Arc<dyn PlatformCapabilitiesProvider>,
        operation_gate: StorageOperationGate,
        budgets: Arc<ExecutionBudgets>,
        memory_sampler: Arc<MemorySampler>,
        memory_leases: Arc<Mutex<MemoryTerminationStore>>,
        cpu_sampler: Arc<CpuSampler>,
        battery_provider: Arc<dyn BatteryProvider>,
        awake: Arc<KeepAwakeManager>,
        docker_status: Arc<DockerStatusCache>,
        dev_ports: Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
        settings: Arc<SettingsAuthority>,
        settings_reaction: Arc<dyn SettingsChangeReaction>,
    ) -> Self {
        Self {
            environment,
            container_host,
            platform_capabilities,
            operation_gate,
            budgets,
            memory_sampler,
            memory_leases,
            cpu_sampler,
            battery_provider,
            awake,
            docker_status,
            dev_ports,
            settings,
            settings_reaction,
        }
    }

    /// The platform capability snapshot the interface renders from.
    pub fn capabilities(&self) -> PlatformCapabilities {
        self.platform_capabilities.capabilities()
    }

    /// A mutating or inspecting capability gate, owned here so a JSON
    /// capability file is never the only thing standing in front of a native
    /// action.
    fn require(&self, feature: PlatformFeature, access: CapabilityAccess) -> Result<(), String> {
        self.platform_capabilities
            .capabilities()
            .require(feature, access)
            .map_err(|error| error.to_string())
    }

    pub async fn memory_metrics(&self) -> Result<MemoryMetrics, String> {
        let observation = self
            .memory_sampler
            .get_observation(Duration::from_millis(800))
            .await?;
        Ok(observation.mint_metrics_with_leases(&self.memory_leases))
    }

    /// One observation of the system-wide CPU share.
    ///
    /// The platform read is synchronous, so it runs on a blocking worker like
    /// the other native probes. The sampler owns the interval state between
    /// observations, so nothing here caches a reading or invents a second one.
    pub async fn cpu_metrics(&self) -> Result<CpuMetrics, String> {
        let sampler = self.cpu_sampler.clone();
        crate::blocking::run_blocking(move || Ok(sampler.observe()), "CPU metrics worker panicked")
            .await
    }

    /// One observation of the machine's battery.
    ///
    /// The provider answers with what the platform reported; the absence of a
    /// battery, and the absence of an adapter for the platform, are both
    /// answers the observation carries rather than errors.
    pub async fn battery_metrics(&self) -> Result<BatteryMetrics, String> {
        let provider = self.battery_provider.clone();
        crate::blocking::run_blocking(
            move || Ok(crate::power::observe_battery(provider.as_ref())),
            "Battery metrics worker panicked",
        )
        .await
    }

    /// Terminates one allowlisted process group. The lease was minted against
    /// a fresh process-table snapshot, and consuming it is what makes the
    /// request one-shot.
    pub async fn terminate_memory_group(
        &self,
        lease_id: &str,
        mode: MemoryTerminationMode,
    ) -> Result<MemoryTerminationResult, String> {
        self.require(
            PlatformFeature::ProcessTermination,
            CapabilityAccess::Mutate,
        )?;
        let lease_id = lease_id.to_string();
        let lease_store = self.memory_leases.clone();
        crate::blocking::run_blocking(
            move || {
                let system = crate::metrics::memory::RealMemorySystem::default();
                MemoryInspector::execute_termination(&lease_id, mode, &lease_store, &system)
            },
            "Process termination worker panicked",
        )
        .await
    }

    pub async fn disk_metrics(&self) -> Result<DiskMetrics, String> {
        let environment = self.environment.clone();
        crate::blocking::run_blocking(
            move || {
                DiskMetricsCollector::get_primary_disk(&environment)
                    .map_err(|error| error.to_string())
            },
            "Disk metrics worker panicked",
        )
        .await
    }

    pub async fn disk_volumes(&self) -> Result<Vec<DiskVolume>, String> {
        let environment = self.environment.clone();
        crate::blocking::run_blocking(
            move || Ok(DiskMetricsCollector::get_volumes(&environment)),
            "Disk volume worker panicked",
        )
        .await
    }

    /// Reads the container runtime's status through the shared observation.
    ///
    /// A cache miss re-checks inside the storage read gate: a Docker mutation
    /// that completed while this request waited would otherwise publish a
    /// status from before it.
    pub async fn docker_status(&self) -> Result<DockerStatus, String> {
        if let Some(status) = self.docker_status.fresh(DOCKER_STATUS_TTL) {
            return Ok(status);
        }

        let _permit = self.budgets.acquire_subprocess().await?;
        let cache = self.docker_status.clone();
        let operation_gate = self.operation_gate.clone();
        let environment = self.environment.clone();
        let container_host = self.container_host.clone();
        crate::blocking::run_blocking(
            move || {
                operation_gate.run_read(|| {
                    if let Some(status) = cache.fresh(DOCKER_STATUS_TTL) {
                        return Ok(status);
                    }

                    let fresh = DockerAdapter::get_status(&environment, &container_host);
                    cache.publish(fresh.clone());
                    Ok(fresh)
                })
            },
            "Docker status worker panicked",
        )
        .await
    }

    /// Prunes one reviewed Docker target and reports the bytes it reclaimed.
    pub async fn prune_docker_target(&self, signature_id: &str) -> Result<u64, String> {
        self.require(PlatformFeature::Docker, CapabilityAccess::Mutate)?;

        let _permit = self.budgets.acquire_subprocess().await?;
        let cache = self.docker_status.clone();
        let operation_gate = self.operation_gate.clone();
        let environment = self.environment.clone();
        let signature_id = signature_id.to_string();
        crate::blocking::run_blocking(
            move || {
                operation_gate.run_write(|| {
                    cache.invalidate();
                    let result = DockerAdapter::prune_category(&environment, &signature_id)
                        .map_err(|error| error.to_string());
                    // The command may have changed Docker state even if its
                    // final status/delta query failed, so never retain a
                    // pre-prune snapshot.
                    cache.invalidate();
                    result
                })
            },
            "Docker cleanup worker panicked",
        )
        .await
    }

    pub async fn local_models(&self) -> Result<LocalModelInventory, String> {
        self.require(PlatformFeature::LocalModels, CapabilityAccess::Inspect)?;

        let _permit = self.budgets.acquire_storage_read().await?;
        let operation_gate = self.operation_gate.clone();
        let environment = self.environment.clone();
        crate::blocking::run_blocking(
            move || {
                operation_gate.run_read(|| Ok(LocalModelScanner::scan_all_models(&environment)))
            },
            "Local model scan worker panicked",
        )
        .await
    }

    /// Deletes one local model and reports the bytes it reclaimed.
    ///
    /// `None` means the model was deleted but the reclaimed amount could not be
    /// measured completely; the interface must not present that as a number.
    pub async fn delete_local_model(
        &self,
        model_id: &str,
    ) -> Result<crate::ipc_numeric::IpcOptionalU64, String> {
        self.require(PlatformFeature::LocalModels, CapabilityAccess::Mutate)?;

        let operation_gate = self.operation_gate.clone();
        let environment = self.environment.clone();
        let model_id = model_id.to_string();
        crate::blocking::run_blocking(
            move || {
                operation_gate.run_write(|| {
                    LocalModelManager::delete_by_id(&environment, &model_id)
                        .map_err(|error| error.to_string())
                })
            },
            "Local model deletion worker panicked",
        )
        .await
        .map(Into::into)
    }

    /// The native Keep Awake manager, for the watcher thread the desktop shell
    /// drives.
    pub fn awake_manager(&self) -> Arc<KeepAwakeManager> {
        self.awake.clone()
    }

    /// Applies the persisted Keep Awake policy at startup.
    ///
    /// The settings file is the only source for the policy, so restoring it is
    /// part of loading settings rather than a separate decision. The rule
    /// bound is enforced by the manager itself; a stored list beyond it is
    /// reported here so the desktop shell can log that the stored rules were
    /// not applied.
    pub fn apply_startup_policy(&self, settings: &ZenithSettings) -> Result<(), String> {
        self.awake.set_rules(settings.awake_rules.clone())?;
        self.awake.set_control_center_awake_policy(
            settings
                .ai_control
                .autopilot
                .keep_awake_for_verified_sessions,
            settings.ai_control.autopilot.keep_awake_ac_only,
        );
        Ok(())
    }

    pub fn awake_state(&self) -> AwakeState {
        self.awake.get_state()
    }

    pub async fn set_awake_rules(&self, rules: Vec<AwakeRule>) -> Result<(), String> {
        self.require(PlatformFeature::KeepAwake, CapabilityAccess::Mutate)?;
        let awake = self.awake.clone();
        crate::blocking::run_blocking(
            move || awake.set_rules(rules),
            "Keep Awake rule worker panicked",
        )
        .await
    }

    pub async fn set_manual_awake(
        &self,
        duration_secs: Option<u64>,
        behavior: AwakeBehavior,
    ) -> Result<(), String> {
        self.require(PlatformFeature::KeepAwake, CapabilityAccess::Mutate)?;
        let awake = self.awake.clone();
        crate::blocking::run_blocking(
            move || {
                awake
                    .set_manual(duration_secs, behavior)
                    .map_err(|error| error.to_string())
            },
            "Manual Keep Awake worker panicked",
        )
        .await
    }

    pub async fn disable_manual_awake(&self) -> Result<(), String> {
        self.require(PlatformFeature::KeepAwake, CapabilityAccess::Mutate)?;
        let awake = self.awake.clone();
        crate::blocking::run_blocking(
            move || {
                awake.disable_manual();
                Ok(())
            },
            "Manual Keep Awake worker panicked",
        )
        .await
    }

    pub fn settings(&self) -> Result<ZenithSettings, String> {
        self.settings.snapshot()
    }

    /// Persists a settings snapshot and publishes it.
    ///
    /// The reaction that runs before the write can refuse (an operating-system
    /// permission that cannot be requested), and the reaction that runs after
    /// it invalidates the caches keyed to the previous snapshot, so the file,
    /// the shared snapshot, and every derived answer move together.
    pub async fn save_settings(
        &self,
        config_dir: &Path,
        settings: ZenithSettings,
        notifications: &dyn DesktopNotifications,
    ) -> Result<(), String> {
        let next = settings.sanitize();
        // The rule bound is checked before anything persists: a save that
        // would leave the file and the shared authority holding a list the
        // runtime refuses is a split brain, so validation happens while the
        // only thing that can refuse is still the save itself.
        KeepAwakeManager::validate_rules(&next.awake_rules)?;
        self.settings_reaction.before_save(&next, notifications)?;

        let config_dir = config_dir.to_path_buf();
        let authority = self.settings.clone();
        let awake = self.awake.clone();
        let change = crate::blocking::run_blocking(
            move || {
                let (published, change) =
                    authority.update_and_persist(&config_dir, |previous| {
                        let change = SettingsChange {
                            provider_selection_changed: previous.ai_accounts_quota_providers
                                != next.ai_accounts_quota_providers,
                            inactivity_threshold_changed: previous
                                .agent_notifications
                                .inactivity_threshold_minutes
                                != next.agent_notifications.inactivity_threshold_minutes,
                        };
                        // AI Control owns this subtree through its dedicated
                        // save/dismissal use cases. A general settings payload
                        // may have been captured before one of those writes,
                        // so preserve the latest authoritative value instead
                        // of reintroducing the stale copy carried by the UI.
                        let ai_control = previous.ai_control.clone();
                        *previous = next;
                        previous.ai_control = ai_control;
                        change
                    })?;
                // The published list was already validated before the write,
                // so this application step cannot refuse it anymore.
                awake.set_rules(published.awake_rules)?;
                Ok::<_, String>(change)
            },
            "Settings save worker panicked",
        )
        .await?;
        self.settings_reaction.saved(&change);
        Ok(())
    }

    pub async fn reveal_path(&self, path: &str) -> Result<(), String> {
        self.require(PlatformFeature::SystemActions, CapabilityAccess::Inspect)?;
        let environment = self.environment.clone();
        let path = path.to_string();
        crate::blocking::run_blocking(
            move || {
                use zenith_platform::SystemActionProvider;
                let path_buf = expand_display_path(&path, &environment)?;
                zenith_platform::NativeSystemActions::new().reveal_path(&path_buf)
            },
            "File manager worker panicked",
        )
        .await
    }

    /// Runs the `--doctor` self-check against the running environment.
    pub async fn environment_self_check(
        &self,
    ) -> Result<crate::diagnostics::doctor::EnvironmentReport, String> {
        let environment = self.environment.clone();
        crate::blocking::run_blocking(
            move || Ok(crate::diagnostics::doctor::self_check(&environment)),
            "Environment self-check worker panicked",
        )
        .await
    }

    pub async fn diagnostics(&self, config_dir: &Path) -> Result<DiagnosticsSnapshot, String> {
        let settings = self.settings.snapshot()?;
        let config_dir = config_dir.to_path_buf();
        crate::blocking::run_blocking(
            move || Ok(crate::diagnostics::get_snapshot(&settings, &config_dir)),
            "Diagnostics worker panicked",
        )
        .await
    }

    pub async fn development_listeners(&self) -> Result<Vec<DevelopmentListener>, String> {
        self.require(PlatformFeature::DevelopmentPorts, CapabilityAccess::Inspect)?;
        let store = self.dev_ports.clone();
        let flavor = self.environment.flavor();
        let home = self.environment.user_home();
        crate::blocking::run_blocking(
            move || {
                crate::dev_ports::list_listeners(
                    &store,
                    &crate::dev_ports::RealDevPortSystem::new(flavor),
                    home.as_deref(),
                )
            },
            "Development listener worker panicked",
        )
        .await
    }

    /// Releases one leased listener by exact process identity.
    pub async fn release_development_listener(
        &self,
        id: &str,
        mode: ReleaseMode,
    ) -> Result<ReleaseDevelopmentListenerResult, String> {
        self.require(PlatformFeature::DevelopmentPorts, CapabilityAccess::Mutate)?;
        let store = self.dev_ports.clone();
        let flavor = self.environment.flavor();
        let home = self.environment.user_home();
        let id = id.to_string();
        crate::blocking::run_blocking(
            move || {
                crate::dev_ports::release_listener(
                    &store,
                    &crate::dev_ports::RealDevPortSystem::new(flavor),
                    &id,
                    mode,
                    home.as_deref(),
                )
            },
            "Development listener release worker panicked",
        )
        .await
    }
}

/// Expands the display form of a path and refuses one that no longer exists.
///
/// The interface hands back what a scan showed it, including `~` notation; the
/// canonical form is what the native reveal and terminal actions receive, so a
/// path that disappeared between review and action fails with a reason instead
/// of opening something unrelated.
fn expand_display_path(path: &str, environment: &PlatformEnvironment) -> Result<PathBuf, String> {
    let path_obj = Path::new(path);
    let normalized = zenith_platform::NativePlatformPaths::normalize_verbatim_path(path_obj);
    let path_str = normalized.to_string_lossy();
    let expanded = if let Some(relative) = path_str
        .strip_prefix("~/")
        .or_else(|| path_str.strip_prefix("~\\"))
    {
        let home = environment
            .user_home()
            .ok_or_else(|| "Home environment variable is not set.".to_string())?;
        home.join(relative)
    } else {
        normalized
    };
    let canonical = expanded
        .canonicalize()
        .map_err(|error| format!("Path is no longer available: {error}"))?;
    Ok(zenith_platform::NativePlatformPaths::normalize_verbatim_path(&canonical))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCapabilitiesProvider(PlatformCapabilities);

    impl PlatformCapabilitiesProvider for TestCapabilitiesProvider {
        fn capabilities(&self) -> PlatformCapabilities {
            self.0.clone()
        }
    }

    /// Records the order a save reports to the other bounded services.
    #[derive(Default)]
    struct RecordingReaction {
        calls: Mutex<Vec<&'static str>>,
        refuse: bool,
    }

    impl SettingsChangeReaction for RecordingReaction {
        fn before_save(
            &self,
            _next: &ZenithSettings,
            _notifications: &dyn DesktopNotifications,
        ) -> Result<(), String> {
            self.calls.lock().unwrap().push("before_save");
            if self.refuse {
                return Err("notification permission was refused".to_string());
            }
            Ok(())
        }

        fn saved(&self, _change: &SettingsChange) {
            self.calls.lock().unwrap().push("saved");
        }
    }

    struct TestNotifications {
        grant: bool,
    }

    impl DesktopNotifications for TestNotifications {
        fn request_permission(&self) -> Result<(), String> {
            if self.grant {
                Ok(())
            } else {
                Err("notification permission was refused".to_string())
            }
        }

        fn emit_recommendations(
            &self,
            _recommendations: &[crate::models::Recommendation],
        ) -> Vec<String> {
            Vec::new()
        }

        fn emit_process_advisories(
            &self,
            _snapshot: &crate::models::AgentActivitySnapshot,
            _preferences: &crate::models::AgentNotificationPreferences,
            _filter: &mut crate::agent_activity::notifications::NotificationFilter,
        ) -> Vec<String> {
            Vec::new()
        }
    }

    fn service_with_reaction(reaction: Arc<dyn SettingsChangeReaction>) -> SystemService {
        service_with_capabilities(reaction, PlatformCapabilities::current())
    }

    fn service_with_capabilities(
        reaction: Arc<dyn SettingsChangeReaction>,
        capabilities: PlatformCapabilities,
    ) -> SystemService {
        SystemService::new(
            Arc::new(PlatformEnvironment::native()),
            ContainerHost::unstated(),
            Arc::new(TestCapabilitiesProvider(capabilities)),
            StorageOperationGate::default(),
            Arc::new(ExecutionBudgets::new()),
            Arc::new(MemorySampler::new()),
            Arc::new(Mutex::new(MemoryTerminationStore::default())),
            Arc::new(CpuSampler::new()),
            Arc::new(crate::power::SystemBatteryProvider::new()),
            Arc::new(KeepAwakeManager::new()),
            Arc::new(DockerStatusCache::new()),
            Arc::new(Mutex::new(crate::dev_ports::DevelopmentPortStore::default())),
            Arc::new(SettingsAuthority::new(ZenithSettings::default())),
            reaction,
        )
    }

    fn service_with_battery(provider: Arc<dyn BatteryProvider>) -> SystemService {
        SystemService::new(
            Arc::new(PlatformEnvironment::native()),
            ContainerHost::unstated(),
            Arc::new(TestCapabilitiesProvider(PlatformCapabilities::current())),
            StorageOperationGate::default(),
            Arc::new(ExecutionBudgets::new()),
            Arc::new(MemorySampler::new()),
            Arc::new(Mutex::new(MemoryTerminationStore::default())),
            Arc::new(CpuSampler::new()),
            provider,
            Arc::new(KeepAwakeManager::new()),
            Arc::new(DockerStatusCache::new()),
            Arc::new(Mutex::new(crate::dev_ports::DevelopmentPortStore::default())),
            Arc::new(SettingsAuthority::new(ZenithSettings::default())),
            Arc::new(RecordingReaction::default()),
        )
    }

    /// The battery the interface renders is the provider's reading, and a
    /// machine the platform says has no battery never reports a charge.
    #[test]
    fn battery_metrics_answer_with_the_provider_reading() {
        let provider = Arc::new(crate::power::MockBatteryProvider::new(
            crate::power::BatteryReading {
                presence: crate::models::BatteryPresence::Present,
                power_source: crate::models::PowerSourceType::Battery,
                is_charging: Some(false),
                percent: Some(72.0),
                time_to_empty_seconds: Some(4_200),
                reason: None,
            },
        ));
        let present =
            tauri::async_runtime::block_on(service_with_battery(provider).battery_metrics())
                .expect("the observation is an answer, not an error");
        assert_eq!(present.presence, crate::models::BatteryPresence::Present);
        assert_eq!(
            present.charge_state,
            crate::models::BatteryChargeState::Discharging
        );
        assert_eq!(present.percent, Some(72.0));
        assert_eq!(present.time_remaining_seconds, Some(4_200));

        let absent_provider = Arc::new(crate::power::MockBatteryProvider::new(
            crate::power::BatteryReading {
                presence: crate::models::BatteryPresence::Absent,
                power_source: crate::models::PowerSourceType::Ac,
                is_charging: None,
                percent: None,
                time_to_empty_seconds: None,
                reason: None,
            },
        ));
        let absent =
            tauri::async_runtime::block_on(service_with_battery(absent_provider).battery_metrics())
                .expect("an absent battery is an answer");
        assert_eq!(absent.presence, crate::models::BatteryPresence::Absent);
        assert_eq!(
            absent.percent, None,
            "an absent battery must not be reported at zero percent"
        );
    }

    /// The CPU surface answers with the sampler's own state machine: a fresh
    /// sampler has not differenced two platform reads yet, so it reports the
    /// warm-up state rather than a share it never measured.
    #[test]
    fn cpu_metrics_report_the_sampler_state_rather_than_a_guess() {
        let service = service_with_capabilities(
            Arc::new(RecordingReaction::default()),
            PlatformCapabilities::current(),
        );

        let first = tauri::async_runtime::block_on(service.cpu_metrics())
            .expect("the observation is an answer");
        assert_eq!(first.state, crate::models::CpuSampleState::Warmup);
        assert_eq!(first.usage_percent, None);
        assert_eq!(first.stale_after_ms, crate::metrics::CPU_STALE_AFTER_MS);
    }

    #[test]
    fn native_system_mutations_require_their_service_capabilities() {
        let service = service_with_capabilities(
            Arc::new(RecordingReaction::default()),
            PlatformCapabilities::unsupported(crate::models::PlatformKind::Linux),
        );

        let keep_awake_errors = [
            tauri::async_runtime::block_on(service.set_awake_rules(Vec::new()))
                .expect_err("rules require Keep Awake"),
            tauri::async_runtime::block_on(
                service.set_manual_awake(None, AwakeBehavior::PreventSystemSleep),
            )
            .expect_err("manual awake requires Keep Awake"),
            tauri::async_runtime::block_on(service.disable_manual_awake())
                .expect_err("disabling manual awake requires Keep Awake"),
        ];
        assert!(
            keep_awake_errors
                .iter()
                .all(|error| error.contains("KeepAwake")),
            "every Keep Awake mutation must fail at the same service gate: {keep_awake_errors:?}"
        );

        // The project-scoped terminal opener moved to the AI surfaces, where
        // the observed project state lives; its capability gate is covered by
        // ai_service's tests.
    }

    fn bound_rule(id: &str) -> crate::models::AwakeRule {
        crate::models::AwakeRule {
            id: id.to_string(),
            app_name: "Bound test".to_string(),
            executable_pattern: "non_existent_bound_test_process".to_string(),
            requires_process_pattern: None,
            application: None,
            agent_ids: Vec::new(),
            behavior: crate::models::AwakeBehavior::PreventSystemSleep,
            power_condition: crate::models::PowerCondition::Always,
            enabled: true,
        }
    }

    fn settings_with_notifications_enabled() -> ZenithSettings {
        ZenithSettings {
            agent_notifications: crate::models::AgentNotificationPreferences {
                enabled: true,
                ..Default::default()
            },
            ..ZenithSettings::default()
        }
    }

    /// A save that cannot arrange its side effects must not publish: the file
    /// and the shared snapshot stay on the previous preferences, and no
    /// service is told a change happened.
    #[test]
    fn a_refused_settings_save_publishes_nothing() {
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let reaction = Arc::new(RecordingReaction {
            refuse: true,
            ..RecordingReaction::default()
        });
        let service = service_with_reaction(reaction.clone());

        let refused = tauri::async_runtime::block_on(service.save_settings(
            config_dir.path(),
            settings_with_notifications_enabled(),
            &TestNotifications { grant: false },
        ));

        assert!(refused.is_err(), "the refusal must reach the caller");
        assert!(
            !crate::settings_store::settings_path(config_dir.path()).exists(),
            "a refused save must not leave a settings file behind"
        );
        assert!(
            !service
                .settings()
                .expect("authority is healthy")
                .agent_notifications
                .enabled,
            "a refused save must not publish the snapshot"
        );
        assert_eq!(
            reaction.calls.lock().unwrap().as_slice(),
            ["before_save"],
            "a refused save must not report completion"
        );
    }

    /// A save whose rule list the runtime refuses must leave everything on
    /// the previous snapshot: the persisted file, the shared authority, and
    /// the rules the Keep Awake manager is actually evaluating.
    #[test]
    fn a_save_with_too_many_rules_persists_nothing() {
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let service = service_with_reaction(Arc::new(RecordingReaction::default()));

        let baseline = bound_rule("rule.baseline");
        let mut accepted = service.settings().expect("authority is healthy");
        accepted.awake_rules = vec![baseline.clone()];
        tauri::async_runtime::block_on(service.save_settings(
            config_dir.path(),
            accepted,
            &TestNotifications { grant: true },
        ))
        .expect("the baseline save succeeds");
        assert_eq!(
            crate::settings_store::load(config_dir.path())
                .awake_rules
                .len(),
            1,
            "the baseline is on disk before the oversized attempt"
        );

        let mut oversized = service.settings().expect("authority is healthy");
        oversized.awake_rules = (0..crate::power::MAX_AWAKE_RULES + 1)
            .map(|index| bound_rule(&format!("rule.overflow_{index}")))
            .collect();
        let refused = tauri::async_runtime::block_on(service.save_settings(
            config_dir.path(),
            oversized,
            &TestNotifications { grant: true },
        ));
        assert!(refused.is_err(), "the oversized list must refuse the save");

        // Nothing moved: the file, the shared authority, and the running rules.
        assert_eq!(
            crate::settings_store::load(config_dir.path()).awake_rules,
            vec![baseline.clone()],
            "the persisted file keeps the baseline"
        );
        assert_eq!(
            service
                .settings()
                .expect("authority is healthy")
                .awake_rules,
            vec![baseline.clone()],
            "the shared authority keeps the baseline"
        );
        assert_eq!(
            service.awake_state().rule_evaluations.len(),
            1,
            "the running manager keeps the baseline"
        );
    }

    /// The accepted path persists, publishes, and then reports the change, in
    /// that order.
    #[test]
    fn an_accepted_settings_save_publishes_then_reports() {
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let reaction = Arc::new(RecordingReaction::default());
        let service = service_with_reaction(reaction.clone());

        tauri::async_runtime::block_on(service.save_settings(
            config_dir.path(),
            settings_with_notifications_enabled(),
            &TestNotifications { grant: true },
        ))
        .expect("the save succeeds");

        assert!(
            crate::settings_store::settings_path(config_dir.path()).exists(),
            "an accepted save persists the snapshot"
        );
        assert!(
            service
                .settings()
                .expect("authority is healthy")
                .agent_notifications
                .enabled,
            "an accepted save publishes the snapshot"
        );
        assert_eq!(
            reaction.calls.lock().unwrap().as_slice(),
            ["before_save", "saved"],
            "the reaction is told after the snapshot is published"
        );
    }

    #[test]
    fn a_general_save_preserves_a_newer_ai_control_transaction() {
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let service = service_with_reaction(Arc::new(RecordingReaction::default()));
        let mut stale_general_snapshot = service.settings().expect("authority is healthy");
        stale_general_snapshot.theme = "dark".to_string();

        service
            .settings
            .update_and_persist(config_dir.path(), |settings| {
                settings
                    .ai_control
                    .dismissed_findings
                    .push("finding.concurrent".to_string());
            })
            .expect("AI-owned transaction succeeds");

        tauri::async_runtime::block_on(service.save_settings(
            config_dir.path(),
            stale_general_snapshot,
            &TestNotifications { grant: true },
        ))
        .expect("general save succeeds");

        let published = service.settings().expect("authority is healthy");
        assert_eq!(published.theme, "dark");
        assert_eq!(
            published.ai_control.dismissed_findings,
            ["finding.concurrent"]
        );
        assert_eq!(crate::settings_store::load(config_dir.path()), published);
    }

    #[test]
    fn docker_status_observation_expires_and_can_be_invalidated() {
        let cache = DockerStatusCache::new();
        assert!(cache.fresh(DOCKER_STATUS_TTL).is_none());

        cache.publish(DockerStatus {
            is_available: false,
            is_running: false,
            version: None,
            error_message: None,
            overview: None,
            images: Vec::new(),
            containers: Vec::new(),
            volumes: Vec::new(),
        });
        assert!(
            cache.fresh(DOCKER_STATUS_TTL).is_some(),
            "a just-observed status answers"
        );

        cache.invalidate();
        assert!(
            cache.fresh(DOCKER_STATUS_TTL).is_none(),
            "a mutation invalidates the observation so the next read cannot answer from before it"
        );
    }

    #[test]
    fn expand_display_path_refuses_a_path_that_disappeared() {
        let environment = PlatformEnvironment::native();
        let missing = environment
            .user_home()
            .map(|home| home.join("zenith-200-missing-path-fixture"))
            .unwrap_or_else(|| PathBuf::from("/zenith-200-missing-path-fixture"));

        let error = expand_display_path(&missing.to_string_lossy(), &environment)
            .expect_err("a path that no longer exists must fail closed");

        assert!(error.starts_with("Path is no longer available"), "{error}");
    }
}
