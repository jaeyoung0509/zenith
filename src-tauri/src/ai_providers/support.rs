use crate::ai_providers::registry::ProviderRegistry;
use crate::models::{AiProviderUsage, ProviderId, UsageSummary, UsageSupport, UsageWindow};
use crate::tooling;
use serde_json::Value;
use std::time::SystemTime;

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn command_exists(command: &str) -> bool {
    tooling::resolve(command).is_some()
}

pub fn base_provider(id: ProviderId, name: &str, auth_label: &str) -> AiProviderUsage {
    let descriptor = ProviderRegistry::find(id);
    let (vendor, model) = if let Some(d) = descriptor {
        (
            d.model_vendor.map(String::from),
            d.model_identity.map(String::from),
        )
    } else {
        (None, None)
    };
    AiProviderUsage {
        id,
        name: name.into(),
        installed: false,
        connected: false,
        auth_label: auth_label.into(),
        status_message: "Not connected".into(),
        support: UsageSupport::Live,
        windows: vec![],
        summary: UsageSummary::default(),
        action_url: None,
        model_vendor: vendor,
        model_identity: model,
    }
}

pub fn failed_provider(id: ProviderId, name: &str, message: &str) -> AiProviderUsage {
    let mut provider = base_provider(id, name, "Unknown");
    provider.support = UsageSupport::Manual;
    provider.status_message = message.into();
    provider
}

pub fn append_rate_windows(target: &mut Vec<UsageWindow>, limits: &Value) {
    let limit_name = limits
        .get("limitName")
        .and_then(Value::as_str)
        .or_else(|| limits.get("limitId").and_then(Value::as_str))
        .unwrap_or("Usage");
    for (key, fallback) in [("primary", "Primary"), ("secondary", "Secondary")] {
        let Some(window) = limits.get(key).filter(|value| !value.is_null()) else {
            continue;
        };
        let duration = u64_field(window, "windowDurationMins").unwrap_or(0);
        let label = if duration == 10080 {
            "Weekly".into()
        } else if duration >= 60 {
            format!("{}h", duration / 60)
        } else if duration > 0 {
            format!("{duration}m")
        } else {
            format!("{limit_name} {fallback}")
        };
        target.push(UsageWindow {
            label,
            used_percent: window
                .get("usedPercent")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            resets_at: u64_field(window, "resetsAt"),
        });
    }
}

pub fn u64_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

pub fn parse_stat_u64(output: &str, label: &str) -> Option<u64> {
    output.lines().find_map(|line| {
        let (_, value) = line.split_once(label)?;
        value
            .trim_matches(|character: char| !character.is_ascii_digit())
            .parse()
            .ok()
    })
}

pub fn parse_stat_f64(output: &str, label: &str) -> Option<f64> {
    output.lines().find_map(|line| {
        let (_, value) = line.split_once(label)?;
        value
            .trim_matches(|character: char| !character.is_ascii_digit() && character != '.')
            .parse()
            .ok()
    })
}

pub fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            output.push(character);
        }
    }
    output
}

pub fn parse_rfc3339_to_unix_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.len() < 20 {
        return None;
    }
    let year: u64 = s.get(0..4)?.parse().ok()?;
    if s.as_bytes().get(4) != Some(&b'-') {
        return None;
    }
    let month: u64 = s.get(5..7)?.parse().ok()?;
    if s.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    let day: u64 = s.get(8..10)?.parse().ok()?;
    let sep = *s.as_bytes().get(10)?;
    if sep != b'T' && sep != b't' {
        return None;
    }
    let hour: u64 = s.get(11..13)?.parse().ok()?;
    if s.as_bytes().get(13) != Some(&b':') {
        return None;
    }
    let min: u64 = s.get(14..16)?.parse().ok()?;
    if s.as_bytes().get(16) != Some(&b':') {
        return None;
    }
    let sec: u64 = s.get(17..19)?.parse().ok()?;

    if year < 1970
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || min > 59
        || sec > 60
    {
        return None;
    }

    let is_leap = is_leap_year(year);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap {
                29
            } else {
                28
            }
        }
        _ => return None,
    };
    if day > days_in_month {
        return None;
    }

    let mut days = 0u64;
    for y in 1970..year {
        let y_leap = is_leap_year(y);
        days += if y_leap { 366 } else { 365 };
    }
    let month_days = [
        31,
        if is_leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    for m in 1..month {
        days += month_days[(m - 1) as usize];
    }
    days += day - 1;

    let epoch_secs = days * 86400 + hour * 3600 + min * 60 + sec;

    let mut rest = &s[19..];
    if rest.starts_with('.') {
        let fraction = &rest[1..];
        let end_digits = fraction
            .find(|c: char| !c.is_ascii_digit())
            .map(|idx| idx + 1)
            .unwrap_or(rest.len());
        if end_digits == 1 {
            return None;
        }
        rest = &rest[end_digits..];
    }

    if rest == "Z" || rest == "z" {
        return Some(epoch_secs);
    }

    if rest.len() == 6
        && (rest.starts_with('+') || rest.starts_with('-'))
        && rest.as_bytes().get(3) == Some(&b':')
    {
        let sign = if rest.starts_with('+') { -1i64 } else { 1i64 };
        let off_h = rest.get(1..3)?.parse::<i64>().ok()?;
        let off_m = rest.get(4..6)?.parse::<i64>().ok()?;
        if off_h > 23 || off_m > 59 {
            return None;
        }
        let offset_secs = sign * (off_h * 3600 + off_m * 60);
        let adjusted = epoch_secs as i64 + offset_secs;
        if adjusted < 0 {
            return None;
        }
        return Some(adjusted as u64);
    }

    None
}

pub fn is_leap_year(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}
