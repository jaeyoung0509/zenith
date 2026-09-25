//! Where the machine's power is coming from, and how much of it is left.
//!
//! Two independent questions live here. [`PowerSourceProvider`] answers which
//! source is supplying the machine, which Keep Awake needs; [`BatteryProvider`]
//! answers what the battery itself reports. Both are ports: the native
//! adapters are the only place platform APIs are called, and a test states a
//! reading instead of asking the host for one.
//!
//! No field is ever filled in from an adjacent one. A desktop machine reports
//! `Absent`, not a battery at zero percent; a platform without an adapter
//! reports `Unavailable` with a reason; and a percentage is only ever the one
//! the platform returned.

use crate::models::{BatteryChargeState, BatteryMetrics, BatteryPresence, PowerSourceType};

pub trait PowerSourceProvider: Send + Sync {
    fn current_power_source(&self) -> PowerSourceType;
}

#[derive(Default, Clone, Copy)]
pub struct SystemPowerSource;

impl SystemPowerSource {
    pub fn new() -> Self {
        Self
    }
}

impl PowerSourceProvider for SystemPowerSource {
    fn current_power_source(&self) -> PowerSourceType {
        #[cfg(target_os = "macos")]
        {
            let snapshot = unsafe { macos_ps::IOPSCopyPowerSourcesInfo() };
            if !snapshot.is_null() {
                let ps_type = unsafe { macos_ps::IOPSGetProvidingPowerSourceType(snapshot) };
                let source = unsafe { power_source_from_cf_string(ps_type) };
                unsafe { macos_ps::CFRelease(snapshot) };
                if source != PowerSourceType::Unknown {
                    return source;
                }
            }

            fallback_pmset_power_source()
        }

        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

            unsafe {
                let mut status: SYSTEM_POWER_STATUS = std::mem::zeroed();
                if GetSystemPowerStatus(&mut status) != 0 {
                    match status.ACLineStatus {
                        1 => PowerSourceType::Ac,
                        0 => PowerSourceType::Battery,
                        _ => PowerSourceType::Unknown,
                    }
                } else {
                    PowerSourceType::Unknown
                }
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            PowerSourceType::Unknown
        }
    }
}

/// The source name a platform prints, mapped to the tri-state.
///
/// Both IOKit and `pmset` name the source in words ("AC Power", "Battery
/// Power"), and neither is a number that could be read positionally.
#[cfg(target_os = "macos")]
fn power_source_from_text(text: &str) -> PowerSourceType {
    if text.contains("AC Power") {
        PowerSourceType::Ac
    } else if text.contains("Battery Power") {
        PowerSourceType::Battery
    } else {
        PowerSourceType::Unknown
    }
}

#[cfg(target_os = "macos")]
fn fallback_pmset_power_source() -> PowerSourceType {
    match pmset_battery_report() {
        Some(text) => power_source_from_text(&text),
        None => PowerSourceType::Unknown,
    }
}

/// One battery observation exactly as the platform reported it.
///
/// The fields are the platform's own facts, before Zenith decides anything:
/// `None` means the platform did not return that fact, and it is never derived
/// from a neighbouring field.
#[derive(Debug, Clone, PartialEq)]
pub struct BatteryReading {
    pub presence: BatteryPresence,
    pub power_source: PowerSourceType,
    /// What the platform said about charging, if it said anything.
    pub is_charging: Option<bool>,
    /// Charge percentage as the platform reported it.
    pub percent: Option<f32>,
    /// The platform's own estimate of runtime left, in seconds.
    pub time_to_empty_seconds: Option<u64>,
    /// Why a fact is missing or the read failed, when there is such a reason.
    pub reason: Option<String>,
}

impl BatteryReading {
    /// The platform answered that there is no battery.
    fn absent(power_source: PowerSourceType) -> Self {
        Self {
            presence: BatteryPresence::Absent,
            power_source,
            is_charging: None,
            percent: None,
            time_to_empty_seconds: None,
            reason: None,
        }
    }

    /// The platform could not answer.
    fn unavailable(power_source: PowerSourceType, reason: impl Into<String>) -> Self {
        Self {
            presence: BatteryPresence::Unavailable,
            power_source,
            is_charging: None,
            percent: None,
            time_to_empty_seconds: None,
            reason: Some(reason.into()),
        }
    }
}

/// Reads the machine's own battery.
pub trait BatteryProvider: Send + Sync {
    /// One read, never cached by the caller: a battery reading is only useful
    /// while it describes now.
    fn read(&self) -> BatteryReading;
}

/// The native adapter for the compiled platform.
#[derive(Default, Clone, Copy)]
pub struct SystemBatteryProvider;

impl SystemBatteryProvider {
    pub fn new() -> Self {
        Self
    }
}

impl BatteryProvider for SystemBatteryProvider {
    fn read(&self) -> BatteryReading {
        native_battery_reading()
    }
}

/// A provider that answers with a stated reading.
///
/// Tests and the browser-preview surfaces use it to describe a machine that is
/// not this one without reaching for a platform API.
pub struct MockBatteryProvider {
    reading: BatteryReading,
}

impl MockBatteryProvider {
    pub fn new(reading: BatteryReading) -> Self {
        Self { reading }
    }
}

impl BatteryProvider for MockBatteryProvider {
    fn read(&self) -> BatteryReading {
        self.reading.clone()
    }
}

/// Reads the battery through the given provider and stamps the observation.
pub fn observe_battery(provider: &dyn BatteryProvider) -> BatteryMetrics {
    battery_metrics_from_reading(provider.read(), Some(crate::metrics::unix_millis()))
}

/// Turns one platform reading into the IPC contract.
///
/// Pure, so every charge state and every refusal is reachable in a test on any
/// machine: the charge state comes from [`derive_charge_state`], a battery that
/// is not present carries no percentage, and a runtime estimate is only
/// reported when the battery is the thing supplying the machine.
pub fn battery_metrics_from_reading(
    reading: BatteryReading,
    sampled_at: Option<u64>,
) -> BatteryMetrics {
    let charge_state = if reading.presence == BatteryPresence::Present {
        derive_charge_state(reading.power_source, reading.is_charging, reading.percent)
    } else {
        // Nothing about a battery that is not there can be charging.
        BatteryChargeState::Unknown
    };

    let percent = match reading.presence {
        BatteryPresence::Present => reading.percent.map(|percent| percent.clamp(0.0, 100.0)),
        // A percentage beside `Absent` or `Unavailable` would be exactly the
        // synthesized value this contract exists to prevent.
        BatteryPresence::Absent | BatteryPresence::Unavailable => None,
    };

    let time_remaining_seconds = match (reading.presence, charge_state) {
        (BatteryPresence::Present, BatteryChargeState::Discharging) => reading
            .time_to_empty_seconds
            .map(|seconds| seconds.min(crate::ipc_numeric::MAX_SAFE_INTEGER)),
        // A "time to empty" describes a machine the battery is supplying; on
        // AC the same number would be a time to full and is not this field.
        _ => None,
    };

    BatteryMetrics {
        presence: reading.presence,
        charge_state,
        percent,
        time_remaining_seconds,
        power_source: reading.power_source,
        sampled_at,
        reason: reading.reason,
    }
}

/// The charge state a platform's own facts imply.
///
/// Kept a pure function of the three facts, so every state is testable on any
/// host and no platform adapter can invent its own rule. AC power alone is not
/// evidence of charging: the machine may be plugged in and holding at 91%.
pub fn derive_charge_state(
    power_source: PowerSourceType,
    is_charging: Option<bool>,
    percent: Option<f32>,
) -> BatteryChargeState {
    let full = percent == Some(100.0);
    match (power_source, is_charging) {
        // A battery that supplies the machine is by definition not charging.
        (PowerSourceType::Battery, _) => BatteryChargeState::Discharging,
        (PowerSourceType::Ac, Some(true)) => {
            if full {
                BatteryChargeState::Full
            } else {
                BatteryChargeState::Charging
            }
        }
        (PowerSourceType::Ac, Some(false)) => {
            if full {
                BatteryChargeState::Full
            } else {
                BatteryChargeState::PluggedInNotCharging
            }
        }
        // Plugged in, but the platform did not say whether the battery is
        // taking a charge.
        (PowerSourceType::Ac, None) => {
            if full {
                BatteryChargeState::Full
            } else {
                BatteryChargeState::Unknown
            }
        }
        (PowerSourceType::Unknown, _) => BatteryChargeState::Unknown,
    }
}

/// The IOKit power API, called by hand.
///
/// Zenith deliberately does not link a CoreFoundation or IOKit wrapper crate:
/// the handful of symbols below are the whole surface it needs, and the
/// reference-counting rules are stated at each call site instead of hidden
/// behind a binding.
#[cfg(target_os = "macos")]
mod macos_ps {
    use std::ffi::c_void;

    /// `kCFStringEncodingUTF8`.
    pub const K_CFSTRING_ENCODING_UTF8: u32 = 0x08000100;
    /// `kCFNumberDoubleType`: a battery description's integers convert into
    /// this without a range decision at each key.
    pub const K_CF_NUMBER_DOUBLE_TYPE: i64 = 13;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        pub fn IOPSCopyPowerSourcesInfo() -> *const c_void;
        pub fn IOPSGetProvidingPowerSourceType(snapshot: *const c_void) -> *const c_void;
        pub fn IOPSCopyPowerSourcesList(snapshot: *const c_void) -> *const c_void;
        pub fn IOPSGetPowerSourceDescription(
            snapshot: *const c_void,
            power_source: *const c_void,
        ) -> *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFStringGetCString(
            the_string: *const c_void,
            buffer: *mut libc::c_char,
            buffer_size: libc::c_long,
            encoding: u32,
        ) -> bool;
        pub fn CFStringCreateWithCString(
            allocator: *const c_void,
            c_str: *const libc::c_char,
            encoding: u32,
        ) -> *const c_void;
        pub fn CFStringGetTypeID() -> libc::c_ulong;
        pub fn CFArrayGetCount(array: *const c_void) -> libc::c_long;
        pub fn CFArrayGetValueAtIndex(array: *const c_void, index: libc::c_long) -> *const c_void;
        pub fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void)
            -> *const c_void;
        pub fn CFNumberGetValue(number: *const c_void, the_type: i64, value: *mut c_void) -> bool;
        pub fn CFBooleanGetValue(boolean: *const c_void) -> bool;
        pub fn CFGetTypeID(cf: *const c_void) -> libc::c_ulong;
        pub fn CFNumberGetTypeID() -> libc::c_ulong;
        pub fn CFBooleanGetTypeID() -> libc::c_ulong;
        pub fn CFRelease(cf: *const c_void);
    }
}

/// One CoreFoundation string dictionary key, owned for the length of a lookup.
///
/// The key is created per read rather than from a constant: `CFSTR` requires
/// the Objective-C constant-string symbol, which a hand-linked call cannot
/// reference. A battery read happens at most once per poll.
#[cfg(target_os = "macos")]
struct CfKey(*const std::ffi::c_void);

#[cfg(target_os = "macos")]
impl CfKey {
    fn new(name: &'static str) -> Self {
        let c_string = std::ffi::CString::new(name).expect("battery keys are literal ASCII");
        // `CFStringCreateWithCString` copies the bytes, so the C string only
        // has to outlive this call.
        let key = unsafe {
            macos_ps::CFStringCreateWithCString(
                std::ptr::null(),
                c_string.as_ptr(),
                macos_ps::K_CFSTRING_ENCODING_UTF8,
            )
        };
        Self(key)
    }

    fn as_ptr(&self) -> *const std::ffi::c_void {
        self.0
    }
}

#[cfg(target_os = "macos")]
impl Drop for CfKey {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { macos_ps::CFRelease(self.0) };
        }
    }
}

/// A dictionary entry, or null when the key is absent.
///
/// The returned value is borrowed from the dictionary: `CFDictionaryGetValue`
/// follows the "get" rule, so it must not be released.
#[cfg(target_os = "macos")]
unsafe fn value_for_key(
    description: *const std::ffi::c_void,
    key: &'static str,
) -> *const std::ffi::c_void {
    let key = CfKey::new(key);
    if key.as_ptr().is_null() {
        return std::ptr::null();
    }
    macos_ps::CFDictionaryGetValue(description, key.as_ptr())
}

/// The number a dictionary key holds, or `None` when the key is missing or
/// holds another type.
#[cfg(target_os = "macos")]
unsafe fn number_value(description: *const std::ffi::c_void, key: &'static str) -> Option<f64> {
    let value = value_for_key(description, key);
    if value.is_null() || macos_ps::CFGetTypeID(value) != macos_ps::CFNumberGetTypeID() {
        return None;
    }
    let mut number: f64 = 0.0;
    let read = macos_ps::CFNumberGetValue(
        value,
        macos_ps::K_CF_NUMBER_DOUBLE_TYPE,
        std::ptr::addr_of_mut!(number).cast(),
    );
    (read && number.is_finite()).then_some(number)
}

/// The boolean a dictionary key holds, or `None` when the key is missing or
/// holds another type.
#[cfg(target_os = "macos")]
unsafe fn bool_value(description: *const std::ffi::c_void, key: &'static str) -> Option<bool> {
    let value = value_for_key(description, key);
    if value.is_null() || macos_ps::CFGetTypeID(value) != macos_ps::CFBooleanGetTypeID() {
        return None;
    }
    Some(macos_ps::CFBooleanGetValue(value))
}

/// The string a dictionary key holds, or `None` when the key is missing or
/// holds another type.
#[cfg(target_os = "macos")]
unsafe fn string_value(description: *const std::ffi::c_void, key: &'static str) -> Option<String> {
    let value = value_for_key(description, key);
    if value.is_null() || macos_ps::CFGetTypeID(value) != macos_ps::CFStringGetTypeID() {
        return None;
    }
    cf_string_text(value)
}

/// The text of a CoreFoundation string.
#[cfg(target_os = "macos")]
unsafe fn cf_string_text(value: *const std::ffi::c_void) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let mut buffer = [0u8; 128];
    let read = macos_ps::CFStringGetCString(
        value,
        buffer.as_mut_ptr() as *mut libc::c_char,
        buffer.len() as libc::c_long,
        macos_ps::K_CFSTRING_ENCODING_UTF8,
    );
    if !read {
        return None;
    }
    Some(
        std::ffi::CStr::from_ptr(buffer.as_ptr() as *const libc::c_char)
            .to_string_lossy()
            .into_owned(),
    )
}

/// The source name `IOPSGetProvidingPowerSourceType` reports, when it reports
/// one.
#[cfg(target_os = "macos")]
unsafe fn power_source_from_cf_string(value: *const std::ffi::c_void) -> PowerSourceType {
    match cf_string_text(value) {
        Some(text) => power_source_from_text(&text),
        None => PowerSourceType::Unknown,
    }
}

/// The `kIOPSTimeToEmptyKey` value that means "no estimate".
#[cfg(target_os = "macos")]
const IOPS_TIME_TO_EMPTY_UNKNOWN: i64 = 65535;

/// The `kIOPSTypeKey` value of the machine's own battery.
#[cfg(target_os = "macos")]
const IOPS_INTERNAL_BATTERY: &str = "InternalBattery";

/// Whether a power-source description is the machine's own battery.
///
/// A UPS is a power source too. Reporting one as the machine's battery would be
/// a synthesized value, so the description's own `Type` decides; a description
/// that does not state a type is accepted, because the caller only reaches this
/// for a listed source.
#[cfg(target_os = "macos")]
unsafe fn is_internal_battery(description: *const std::ffi::c_void) -> bool {
    string_value(description, "Type").is_none_or(|kind| kind == IOPS_INTERNAL_BATTERY)
}

/// One IOKit battery description as a reading.
#[cfg(target_os = "macos")]
unsafe fn battery_from_description(
    description: *const std::ffi::c_void,
    system_source: PowerSourceType,
) -> BatteryReading {
    if bool_value(description, "Is Present") == Some(false) {
        return BatteryReading::absent(system_source);
    }

    // The IOPS dictionary states capacities as percentages of the maximum, so
    // the pair is a percentage and not a raw charge count.
    let percent = match (
        number_value(description, "Current Capacity"),
        number_value(description, "Max Capacity"),
    ) {
        (Some(current), Some(maximum)) if maximum > 0.0 => {
            Some(((current / maximum) * 100.0).clamp(0.0, 100.0) as f32)
        }
        _ => None,
    };

    let power_source = if system_source != PowerSourceType::Unknown {
        system_source
    } else {
        match string_value(description, "Power Source State") {
            Some(state) => power_source_from_text(&state),
            None => PowerSourceType::Unknown,
        }
    };

    let time_to_empty_seconds = number_value(description, "Time to Empty")
        .map(|minutes| minutes as i64)
        // 65535 is `kIOPSTimeToEmptyUnknown`, a negative value is not an
        // estimate, and zero means "not estimated" rather than "empty now".
        .filter(|minutes| *minutes > 0 && *minutes != IOPS_TIME_TO_EMPTY_UNKNOWN)
        .map(|minutes| (minutes as u64).saturating_mul(60));

    BatteryReading {
        presence: BatteryPresence::Present,
        power_source,
        is_charging: bool_value(description, "Is Charging"),
        percent,
        time_to_empty_seconds,
        reason: None,
    }
}

/// The battery reading for macOS.
#[cfg(target_os = "macos")]
fn macos_battery_reading() -> BatteryReading {
    let snapshot = unsafe { macos_ps::IOPSCopyPowerSourcesInfo() };
    if snapshot.is_null() {
        return macos_battery_fallback("IOKit returned no power-source snapshot.");
    }

    let system_source =
        unsafe { power_source_from_cf_string(macos_ps::IOPSGetProvidingPowerSourceType(snapshot)) };

    let list = unsafe { macos_ps::IOPSCopyPowerSourcesList(snapshot) };
    if list.is_null() {
        unsafe { macos_ps::CFRelease(snapshot) };
        return macos_battery_fallback("IOKit returned no power-source list.");
    }

    // `IOPSCopyPowerSourcesList` follows the "create" rule, so the array is
    // released here; each description is borrowed from the snapshot and must
    // not be.
    let count = unsafe { macos_ps::CFArrayGetCount(list) };
    let mut reading = None;
    let mut index = 0;
    while index < count {
        let power_source = unsafe { macos_ps::CFArrayGetValueAtIndex(list, index) };
        index += 1;
        if power_source.is_null() {
            continue;
        }
        let description =
            unsafe { macos_ps::IOPSGetPowerSourceDescription(snapshot, power_source) };
        if description.is_null() || !unsafe { is_internal_battery(description) } {
            continue;
        }
        reading = Some(unsafe { battery_from_description(description, system_source) });
        break;
    }
    unsafe {
        macos_ps::CFRelease(list);
        macos_ps::CFRelease(snapshot);
    }

    match reading {
        // The platform answered and no internal battery was among the sources
        // it listed: this machine has none. A percent would be an invention.
        None => BatteryReading::absent(system_source),
        Some(reading) => reading,
    }
}

/// The `pmset -g batt` report, or `None` when the command did not run.
#[cfg(target_os = "macos")]
fn pmset_battery_report() -> Option<String> {
    let mut cmd = std::process::Command::new("pmset");
    cmd.args(["-g", "batt"]);
    let output =
        zenith_platform::subprocess::run_with_timeout(cmd, std::time::Duration::from_secs(3))
            .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The reading for a machine whose IOKit query returned nothing usable.
#[cfg(target_os = "macos")]
fn macos_battery_fallback(io_kit_failure: &str) -> BatteryReading {
    match pmset_battery_report()
        .as_deref()
        .and_then(parse_pmset_battery)
    {
        Some(reading) => reading,
        None => BatteryReading::unavailable(
            PowerSourceType::Unknown,
            format!("{io_kit_failure} `pmset -g batt` reported no battery either."),
        ),
    }
}

/// Parses the `pmset -g batt` report.
///
/// The command prints one line per power source, for example:
///
/// ```text
/// Now drawing from 'AC Power'
///  -InternalBattery-0 (id=36569187)    91%; AC attached; not charging present: true
/// ```
///
/// Pure and separate from the subprocess, so the shapes macOS prints are cases
/// in a test rather than a description of one machine.
#[cfg(target_os = "macos")]
pub(crate) fn parse_pmset_battery(text: &str) -> Option<BatteryReading> {
    let header_source = power_source_from_text(text);

    for line in text.lines() {
        let Some((capacity, state)) = line.split_once('%') else {
            continue;
        };
        let Some(percent) = capacity
            .rsplit(['\t', ' '])
            .next()
            .and_then(|value| value.trim().parse::<f32>().ok())
        else {
            continue;
        };

        let segments: Vec<String> = state
            .split(';')
            .map(|segment| segment.trim().to_ascii_lowercase())
            .filter(|segment| !segment.is_empty())
            .collect();
        let state_name = segments.first().map(String::as_str).unwrap_or_default();

        if line.contains("present: false") {
            // The slot is listed and reports nothing in it.
            return Some(BatteryReading::absent(header_source));
        }

        let mut power_source = header_source;
        let mut is_charging = None;
        match state_name {
            "charging" | "finishing charge" => {
                power_source = PowerSourceType::Ac;
                is_charging = Some(true);
            }
            "discharging" => {
                power_source = PowerSourceType::Battery;
                is_charging = Some(false);
            }
            "charged" | "full" => {
                power_source = PowerSourceType::Ac;
                is_charging = Some(false);
            }
            "ac attached" => {
                power_source = PowerSourceType::Ac;
                if segments
                    .iter()
                    .any(|segment| segment.contains("not charging"))
                {
                    is_charging = Some(false);
                } else if segments.iter().any(|segment| segment.contains("charging")) {
                    is_charging = Some(true);
                }
            }
            _ => {}
        }

        // `pmset` prints the runtime estimate only while the battery supplies
        // the machine; on AC the same clock would be a time to full.
        let time_to_empty_seconds = match (power_source, is_charging) {
            (PowerSourceType::Battery, Some(false)) => parse_pmset_remaining_seconds(line),
            _ => None,
        };

        return Some(BatteryReading {
            presence: BatteryPresence::Present,
            power_source,
            is_charging,
            percent: Some(percent),
            time_to_empty_seconds,
            reason: None,
        });
    }

    None
}

/// The `H:MM remaining` estimate on a `pmset` line, in seconds.
///
/// A zero estimate is not an estimate, and `pmset` prints `(no estimate)` when
/// it has none, which has no clock to parse.
#[cfg(target_os = "macos")]
fn parse_pmset_remaining_seconds(line: &str) -> Option<u64> {
    let (clock, _) = line.split_once(" remaining")?;
    let (hours, minutes) = clock.rsplit_once(':')?;
    let hours = hours
        .rsplit(['\t', ' '])
        .next()?
        .trim()
        .parse::<u64>()
        .ok()?;
    let minutes = minutes.trim().parse::<u64>().ok()?;
    let total = hours
        .saturating_mul(3_600)
        .saturating_add(minutes.saturating_mul(60));
    (total > 0).then_some(total)
}

/// Windows battery flags from `SYSTEM_POWER_STATUS`.
///
/// `windows-sys` exposes `GetSystemPowerStatus` and the struct, but not these
/// values, so they are stated from the Win32 contract rather than guessed.
#[cfg(target_os = "windows")]
mod win_battery {
    /// `BATTERY_FLAG_CHARGING`: the battery is charging.
    pub const CHARGING: u8 = 8;
    /// `BATTERY_FLAG_NO_BATTERY`: the system has no battery.
    pub const NO_BATTERY: u8 = 128;
    /// `BATTERY_FLAG_UNKNOWN`: the battery state is unknown.
    pub const UNKNOWN: u8 = 255;
    /// `BATTERY_PERCENTAGE_UNKNOWN`.
    pub const PERCENTAGE_UNKNOWN: u8 = 255;
    /// `BATTERY_LIFE_TIME_UNKNOWN`.
    pub const LIFE_TIME_UNKNOWN: u32 = 0xFFFF_FFFF;
}

/// The battery reading for Windows.
#[cfg(target_os = "windows")]
fn windows_battery_reading() -> BatteryReading {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return BatteryReading::unavailable(
            PowerSourceType::Unknown,
            "Windows did not report the system power status.",
        );
    }

    let power_source = match status.ACLineStatus {
        1 => PowerSourceType::Ac,
        0 => PowerSourceType::Battery,
        _ => PowerSourceType::Unknown,
    };

    if status.BatteryFlag == win_battery::NO_BATTERY {
        return BatteryReading::absent(power_source);
    }
    if status.BatteryFlag == win_battery::UNKNOWN {
        return BatteryReading::unavailable(
            power_source,
            "Windows reported the battery state as unknown.",
        );
    }

    let percent = (status.BatteryLifePercent != win_battery::PERCENTAGE_UNKNOWN)
        .then(|| f32::from(status.BatteryLifePercent));
    let time_to_empty_seconds = (status.BatteryLifeTime != win_battery::LIFE_TIME_UNKNOWN)
        .then(|| u64::from(status.BatteryLifeTime))
        .filter(|seconds| *seconds > 0);

    BatteryReading {
        presence: BatteryPresence::Present,
        power_source,
        is_charging: Some(status.BatteryFlag & win_battery::CHARGING != 0),
        percent,
        time_to_empty_seconds,
        reason: percent
            .is_none()
            .then(|| "Windows did not report a battery charge percentage.".to_string()),
    }
}

/// The battery reading for the compiled platform.
fn native_battery_reading() -> BatteryReading {
    #[cfg(target_os = "macos")]
    {
        macos_battery_reading()
    }

    #[cfg(target_os = "windows")]
    {
        windows_battery_reading()
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        BatteryReading::unavailable(
            PowerSourceType::Unknown,
            "Zenith has no battery adapter for this platform.",
        )
    }
}

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct MockPowerSource {
    source: PowerSourceType,
    query_count: Arc<AtomicUsize>,
}

impl MockPowerSource {
    pub fn new(source: PowerSourceType) -> Self {
        Self {
            source,
            query_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn set_source(&mut self, source: PowerSourceType) {
        self.source = source;
    }

    pub fn query_count(&self) -> usize {
        self.query_count.load(Ordering::SeqCst)
    }
}

impl PowerSourceProvider for MockPowerSource {
    fn current_power_source(&self) -> PowerSourceType {
        self.query_count.fetch_add(1, Ordering::SeqCst);
        self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_power_source_reports_the_injected_source_on_every_query() {
        let mut mock = MockPowerSource::new(PowerSourceType::Battery);
        assert_eq!(mock.current_power_source(), PowerSourceType::Battery);
        assert_eq!(mock.current_power_source(), PowerSourceType::Battery);
        assert_eq!(mock.query_count(), 2, "each query must reach the provider");

        mock.set_source(PowerSourceType::Ac);
        assert_eq!(mock.current_power_source(), PowerSourceType::Ac);

        // An unidentifiable power source is a real answer, not a panic or a
        // silently assumed AC.
        mock.set_source(PowerSourceType::Unknown);
        assert_eq!(mock.current_power_source(), PowerSourceType::Unknown);
        assert_eq!(mock.query_count(), 4);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn system_power_source_is_unknown_without_a_native_adapter() {
        assert_eq!(
            SystemPowerSource::new().current_power_source(),
            PowerSourceType::Unknown
        );
    }

    fn present(
        power_source: PowerSourceType,
        is_charging: Option<bool>,
        percent: Option<f32>,
        time_to_empty_seconds: Option<u64>,
    ) -> BatteryReading {
        BatteryReading {
            presence: BatteryPresence::Present,
            power_source,
            is_charging,
            percent,
            time_to_empty_seconds,
            reason: None,
        }
    }

    /// The contract's table, one assertion per row: a state that a platform
    /// could report is a case here rather than a behaviour only one host can
    /// reach.
    #[test]
    fn power_facts_derive_the_charge_state_the_platform_implies() {
        // A battery supplying the machine is not charging, whatever the
        // platform's charging flag says.
        assert_eq!(
            derive_charge_state(PowerSourceType::Battery, Some(false), Some(64.0)),
            BatteryChargeState::Discharging
        );
        assert_eq!(
            derive_charge_state(PowerSourceType::Battery, Some(true), Some(64.0)),
            BatteryChargeState::Discharging
        );
        assert_eq!(
            derive_charge_state(PowerSourceType::Battery, Some(true), Some(100.0)),
            BatteryChargeState::Discharging
        );

        assert_eq!(
            derive_charge_state(PowerSourceType::Ac, Some(true), Some(64.0)),
            BatteryChargeState::Charging
        );
        assert_eq!(
            derive_charge_state(PowerSourceType::Ac, Some(true), Some(100.0)),
            BatteryChargeState::Full
        );

        assert_eq!(
            derive_charge_state(PowerSourceType::Ac, Some(false), Some(91.0)),
            BatteryChargeState::PluggedInNotCharging
        );
        assert_eq!(
            derive_charge_state(PowerSourceType::Ac, Some(false), Some(100.0)),
            BatteryChargeState::Full
        );

        // Plugged in with no charging answer is not evidence of charging.
        assert_eq!(
            derive_charge_state(PowerSourceType::Ac, None, Some(64.0)),
            BatteryChargeState::Unknown
        );
        assert_eq!(
            derive_charge_state(PowerSourceType::Ac, None, Some(100.0)),
            BatteryChargeState::Full
        );

        assert_eq!(
            derive_charge_state(PowerSourceType::Unknown, Some(true), Some(100.0)),
            BatteryChargeState::Unknown
        );
        assert_eq!(
            derive_charge_state(PowerSourceType::Unknown, None, None),
            BatteryChargeState::Unknown
        );
    }

    #[test]
    fn a_stated_battery_reading_reaches_the_contract_through_the_provider() {
        let provider = MockBatteryProvider::new(present(
            PowerSourceType::Battery,
            Some(false),
            Some(72.0),
            Some(4_200),
        ));

        let metrics = observe_battery(&provider);

        assert_eq!(metrics.presence, BatteryPresence::Present);
        assert_eq!(metrics.charge_state, BatteryChargeState::Discharging);
        assert_eq!(metrics.percent, Some(72.0));
        assert_eq!(metrics.time_remaining_seconds, Some(4_200));
        assert_eq!(metrics.power_source, PowerSourceType::Battery);
        assert!(
            metrics.sampled_at.is_some_and(|sampled_at| sampled_at > 0),
            "an observation carries the instant it was taken"
        );
        assert_eq!(metrics.reason, None);
    }

    #[test]
    fn an_absent_battery_is_reported_absent_rather_than_at_zero_percent() {
        let provider = MockBatteryProvider::new(BatteryReading::absent(PowerSourceType::Ac));

        let metrics = observe_battery(&provider);

        assert_eq!(metrics.presence, BatteryPresence::Absent);
        assert_eq!(
            metrics.percent, None,
            "a machine without a battery has no charge to report"
        );
        assert_eq!(metrics.charge_state, BatteryChargeState::Unknown);
        assert_eq!(metrics.time_remaining_seconds, None);
        assert_eq!(metrics.power_source, PowerSourceType::Ac);
    }

    /// A platform fact that contradicts itself must not become a percentage
    /// beside a presence the platform did not confirm.
    #[test]
    fn a_reading_that_is_not_present_carries_no_percentage() {
        let contradictory = BatteryReading {
            presence: BatteryPresence::Absent,
            power_source: PowerSourceType::Ac,
            is_charging: Some(true),
            percent: Some(87.0),
            time_to_empty_seconds: Some(600),
            reason: None,
        };

        let metrics = battery_metrics_from_reading(contradictory, None);

        assert_eq!(metrics.presence, BatteryPresence::Absent);
        assert_eq!(metrics.percent, None);
        assert_eq!(metrics.charge_state, BatteryChargeState::Unknown);
        assert_eq!(metrics.time_remaining_seconds, None);
    }

    #[test]
    fn a_runtime_estimate_is_only_reported_while_the_battery_supplies_the_machine() {
        let discharging = battery_metrics_from_reading(
            present(PowerSourceType::Battery, Some(false), Some(40.0), Some(600)),
            None,
        );
        assert_eq!(discharging.time_remaining_seconds, Some(600));

        for reading in [
            present(PowerSourceType::Ac, Some(true), Some(40.0), Some(600)),
            present(PowerSourceType::Ac, Some(false), Some(40.0), Some(600)),
            present(PowerSourceType::Ac, None, Some(40.0), Some(600)),
            present(PowerSourceType::Unknown, None, Some(40.0), Some(600)),
        ] {
            let metrics = battery_metrics_from_reading(reading, None);
            assert_eq!(
                metrics.time_remaining_seconds, None,
                "a time to empty only describes a machine the battery supplies: {metrics:?}"
            );
        }
    }

    #[test]
    fn a_platform_without_an_adapter_reports_unavailable_with_its_reason() {
        let metrics = battery_metrics_from_reading(
            BatteryReading::unavailable(
                PowerSourceType::Unknown,
                "Zenith has no battery adapter for this platform.",
            ),
            None,
        );

        assert_eq!(metrics.presence, BatteryPresence::Unavailable);
        assert_eq!(metrics.percent, None);
        assert_eq!(
            metrics.reason.as_deref(),
            Some("Zenith has no battery adapter for this platform.")
        );
    }

    #[test]
    fn a_reading_rounds_a_percentage_onto_the_contract_range() {
        let metrics = battery_metrics_from_reading(
            present(PowerSourceType::Battery, Some(false), Some(100.4), None),
            None,
        );
        assert_eq!(metrics.percent, Some(100.0));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pmset_output_maps_to_the_battery_state_it_reports() {
        // The line this project's development machine prints while plugged in
        // and holding: the battery is not the power source, and AC alone does
        // not make it charging.
        let attached = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=36569187)\t91%; AC attached; not charging present: true\n";
        let reading = parse_pmset_battery(attached).expect("the battery line must parse");
        assert_eq!(reading.presence, BatteryPresence::Present);
        assert_eq!(reading.percent, Some(91.0));
        assert_eq!(reading.is_charging, Some(false));
        assert_eq!(reading.power_source, PowerSourceType::Ac);
        assert_eq!(reading.time_to_empty_seconds, None);
        let metrics = battery_metrics_from_reading(reading, Some(0));
        assert_eq!(
            metrics.charge_state,
            BatteryChargeState::PluggedInNotCharging
        );
        assert_eq!(metrics.percent, Some(91.0));
        assert_eq!(metrics.time_remaining_seconds, None);

        let discharging = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t64%; discharging; 2:12 remaining present: true\n";
        let reading = parse_pmset_battery(discharging).expect("the battery line must parse");
        assert_eq!(reading.power_source, PowerSourceType::Battery);
        assert_eq!(reading.is_charging, Some(false));
        assert_eq!(reading.percent, Some(64.0));
        assert_eq!(reading.time_to_empty_seconds, Some(7_920));
        let metrics = battery_metrics_from_reading(reading, Some(0));
        assert_eq!(metrics.charge_state, BatteryChargeState::Discharging);
        assert_eq!(metrics.time_remaining_seconds, Some(7_920));

        // Charging reports a clock too, but it is a time to full.
        let charging = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t20%; charging; 1:30 remaining present: true\n";
        let reading = parse_pmset_battery(charging).expect("the battery line must parse");
        assert_eq!(reading.is_charging, Some(true));
        assert_eq!(reading.time_to_empty_seconds, None);
        let metrics = battery_metrics_from_reading(reading, Some(0));
        assert_eq!(metrics.charge_state, BatteryChargeState::Charging);
        assert_eq!(metrics.time_remaining_seconds, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pmset_output_without_a_battery_line_has_no_reading() {
        // A desktop Mac prints the source line only.
        assert_eq!(parse_pmset_battery("Now drawing from 'AC Power'\n"), None);

        let empty_slot = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t0%; AC attached; not charging present: false\n";
        let reading = parse_pmset_battery(empty_slot).expect("the line describes a slot");
        assert_eq!(reading.presence, BatteryPresence::Absent);
        assert_eq!(reading.percent, None);
    }
}
