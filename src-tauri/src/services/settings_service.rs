//! The settings authority every service reads, and the reactions a save
//! triggers in the services whose caches are keyed to preferences.
//!
//! Settings are shared authority rather than one workflow's state, so the
//! snapshot lives in one place and is reached through typed accessors instead
//! of a mutex each handler locks for itself. Authority state fails closed: a
//! poisoned lock means an interrupted settings transaction, and resuming a
//! partially written snapshot is not a safe recovery.
//!
//! Whole-file persistence is serialized here; each service supplies only the
//! mutation its use case owns. This keeps concurrent commands from publishing
//! two snapshots derived from the same stale value.

use std::path::Path;
use std::sync::Mutex;

use crate::models::ZenithSettings;
use crate::services::desktop_notifications::DesktopNotifications;

/// The process-wide settings snapshot.
pub struct SettingsAuthority {
    settings: Mutex<ZenithSettings>,
    write_transaction: Mutex<()>,
}

impl SettingsAuthority {
    pub(crate) fn new(initial: ZenithSettings) -> Self {
        Self {
            settings: Mutex::new(initial),
            write_transaction: Mutex::new(()),
        }
    }

    /// A copy of the current preferences.
    pub fn snapshot(&self) -> Result<ZenithSettings, String> {
        self.settings
            .lock()
            .map(|guard| guard.clone())
            .map_err(|_| settings_unavailable())
    }

    /// Replaces the snapshot after the caller persisted it.
    pub fn replace(&self, next: ZenithSettings) -> Result<(), String> {
        let _transaction = self
            .write_transaction
            .lock()
            .map_err(|_| settings_unavailable())?;
        self.publish(next)
    }

    /// Serializes a whole-file settings update from its latest snapshot through
    /// atomic persistence and publication.
    ///
    /// Writers must mutate through this boundary rather than composing
    /// `snapshot -> save -> replace` themselves. That sequence loses unrelated
    /// fields when two async commands start from the same snapshot. The
    /// transaction mutex keeps reads cheap while ensuring every writer derives
    /// its update from the last successfully persisted value.
    pub fn update_and_persist<R>(
        &self,
        config_dir: &Path,
        update: impl FnOnce(&mut ZenithSettings) -> R,
    ) -> Result<(ZenithSettings, R), String> {
        let _transaction = self
            .write_transaction
            .lock()
            .map_err(|_| settings_unavailable())?;
        let mut next = self.snapshot()?;
        let result = update(&mut next);
        crate::settings_store::save(config_dir, &next)?;
        self.publish(next.clone())?;
        Ok((next, result))
    }

    fn publish(&self, next: ZenithSettings) -> Result<(), String> {
        let mut guard = self.settings.lock().map_err(|_| settings_unavailable())?;
        *guard = next;
        Ok(())
    }
}

fn settings_unavailable() -> String {
    "Settings state is unavailable due to an unexpected previous error".to_string()
}

/// What a saved snapshot changed, for the services that cache derived data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsChange {
    /// The AI quota-provider selection changed, so a usage snapshot collected
    /// for the previous selection no longer answers for the interface.
    pub provider_selection_changed: bool,
    /// The inactivity threshold changed, so a cached activity registry was
    /// classified under a different question than the current one.
    pub inactivity_threshold_changed: bool,
}

/// The reaction a settings save triggers in other bounded services.
///
/// A settings save is one use case that spans domains: the service that
/// persists preferences cannot own what every other domain must do about the
/// change. The composition root injects the implementations, so the
/// persistence service never names the AI runtime and the AI runtime never
/// writes settings.
pub trait SettingsChangeReaction: Send + Sync {
    /// Runs before the snapshot is persisted or published, so a refusal (for
    /// example an operating-system notification permission that cannot be
    /// requested) leaves both the file and the shared snapshot untouched.
    fn before_save(
        &self,
        next: &ZenithSettings,
        notifications: &dyn DesktopNotifications,
    ) -> Result<(), String>;

    /// Runs after the snapshot is persisted and published.
    fn saved(&self, change: &SettingsChange);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_publishes_the_persisted_snapshot() {
        let authority = SettingsAuthority::new(ZenithSettings::default());
        let next = ZenithSettings {
            intensive_cleanup: true,
            ..ZenithSettings::default()
        };

        authority.replace(next).expect("authority is healthy");

        assert!(
            authority
                .snapshot()
                .expect("authority is healthy")
                .intensive_cleanup
        );
    }

    #[test]
    fn concurrent_update_transactions_preserve_unrelated_fields() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};
        use std::time::Duration;

        let config_dir = tempfile::tempdir().expect("temp config dir");
        let authority = Arc::new(SettingsAuthority::new(ZenithSettings::default()));
        let start = Arc::new(Barrier::new(3));
        let active_transactions = Arc::new(AtomicUsize::new(0));
        let max_active_transactions = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();

        for intensive in [false, true] {
            let authority = authority.clone();
            let config_dir = config_dir.path().to_path_buf();
            let start = start.clone();
            let active_transactions = active_transactions.clone();
            let max_active_transactions = max_active_transactions.clone();
            workers.push(std::thread::spawn(move || {
                start.wait();
                authority
                    .update_and_persist(&config_dir, |settings| {
                        let active = active_transactions.fetch_add(1, Ordering::SeqCst) + 1;
                        max_active_transactions.fetch_max(active, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(25));
                        if intensive {
                            settings.intensive_cleanup = true;
                        } else {
                            settings.clean_developer_tools = false;
                        }
                        active_transactions.fetch_sub(1, Ordering::SeqCst);
                    })
                    .expect("concurrent update succeeds");
            }));
        }
        start.wait();
        for worker in workers {
            worker.join().expect("settings worker completes");
        }

        let persisted = crate::settings_store::load(config_dir.path());
        assert!(!persisted.clean_developer_tools);
        assert!(persisted.intensive_cleanup);
        assert_eq!(max_active_transactions.load(Ordering::SeqCst), 1);
        assert_eq!(
            authority.snapshot().expect("authority is healthy"),
            persisted
        );
    }

    #[test]
    fn poisoned_authority_refuses_to_answer() {
        use std::sync::Arc;

        let authority = Arc::new(SettingsAuthority::new(ZenithSettings::default()));
        let poisoner = authority.clone();
        let _ = std::thread::spawn(move || {
            let mut settings = poisoner.settings.lock().expect("authority starts healthy");
            settings.intensive_cleanup = true;
            panic!("interrupted settings transaction");
        })
        .join();

        let error = authority
            .snapshot()
            .expect_err("a poisoned authority fails closed");
        assert!(error.contains("Settings state is unavailable"), "{error}");
    }
}
