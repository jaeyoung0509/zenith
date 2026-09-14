mod ai_service;
pub mod desktop_notifications;
mod plan_store;
pub mod progress;
mod scan_service;
mod scan_store;
mod settings_service;
mod storage_service;
mod system_service;

pub use ai_service::AiService;
pub use cleanup_service::{select_quick_clean_safe_candidates, CleanupService};
pub use plan_store::{OneShotPlan, PlanLifecycle, PlanStore};
pub(crate) use scan_service::ScanService;
pub(crate) use scan_store::ScanStore;
pub use settings_service::{SettingsAuthority, SettingsChange, SettingsChangeReaction};
pub use storage_service::StorageService;
pub use system_service::{DockerStatusCache, SystemService};

pub mod cleanup_service;
