pub mod app_picker;
pub mod assertion;
pub mod source;
pub mod watcher;
#[cfg(windows)]
mod windows_request;

pub use app_picker::ApplicationPicker;
pub use assertion::{NativeAssertionProvider, PowerAssertion, PowerAssertionProvider};
pub use source::{
    battery_metrics_from_reading, derive_charge_state, observe_battery, BatteryProvider,
    BatteryReading, MockBatteryProvider, MockPowerSource, PowerSourceProvider,
    SystemBatteryProvider, SystemPowerSource,
};
pub use watcher::{KeepAwakeManager, MAX_AWAKE_RULES};
