//! The settings authority every service reads, and the reactions a save
//! triggers in the services whose caches are keyed to preferences.
//!
//! Settings are shared authority rather than one workflow's state, so the
//! snapshot lives in one place and is reached through typed accessors instead
//! of a mutex each handler locks for itself. Authority state fails closed: a
//! poisoned lock means an interrupted settings transaction, and resuming a
//! partially written snapshot is not a safe recovery.
//!
//! Persistence stays with the service that owns the surrounding use case; this
//! module only holds what the process currently believes.

use std::sync::Mutex;

use crate::models::ZenithSettings;
use crate::services::desktop_notifications::DesktopNotifications;

/// The process-wide settings snapshot.
pub struct SettingsAuthority {
    settings: Mutex<ZenithSettings>,
}

impl SettingsAuthority {
    pub(crate) fn new(initial: ZenithSettings) -> Self {
        Self {
            settings: Mutex::new(initial),
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
        let mut guard = self.settings.lock().map_err(|_| settings_unavailable())?;
        *guard = next;
        Ok(())
    }

    /// Mutates the snapshot in place and returns the result.
    ///
    /// The caller owns persistence: an edit that is not saved is only the
    /// process's belief, which is why every caller pairs this with
    /// [`crate::settings_store::save`] before publishing it to the interface.
    pub fn edit(&self, edit: impl FnOnce(&mut ZenithSettings)) -> Result<ZenithSettings, String> {
        let mut guard = self.settings.lock().map_err(|_| settings_unavailable())?;
        edit(&mut guard);
        Ok(guard.clone())
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
    fn edit_returns_the_published_snapshot() {
        let authority = SettingsAuthority::new(ZenithSettings::default());

        let edited = authority
            .edit(|settings| settings.clean_developer_tools = false)
            .expect("authority is healthy");

        assert!(!edited.clean_developer_tools);
        assert!(
            !authority
                .snapshot()
                .expect("authority is healthy")
                .clean_developer_tools,
            "the edit must be visible to the next reader"
        );
    }

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
    fn poisoned_authority_refuses_to_answer() {
        use std::sync::Arc;

        let authority = Arc::new(SettingsAuthority::new(ZenithSettings::default()));
        let poisoner = authority.clone();
        let _ = std::thread::spawn(move || {
            let _ = poisoner.edit(|settings| {
                settings.intensive_cleanup = true;
                panic!("interrupted settings transaction");
            });
        })
        .join();

        let error = authority
            .snapshot()
            .expect_err("a poisoned authority fails closed");
        assert!(error.contains("Settings state is unavailable"), "{error}");
    }
}
