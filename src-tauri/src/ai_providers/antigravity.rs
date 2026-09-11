use super::credentials::CredentialStore;
use super::support::{base_provider, command_exists, parse_rfc3339_to_unix_secs};
use super::ProviderAdapter;
use crate::models::{AiProviderUsage, UsageSupport, UsageWindow};
use crate::tooling;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

pub const ANTIGRAVITY_USAGE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct AntigravityAdapter;

impl ProviderAdapter for AntigravityAdapter {
    fn id(&self) -> &'static str {
        "antigravity"
    }

    fn collect(&self, _credentials: &dyn CredentialStore) -> AiProviderUsage {
        let has_cli = command_exists("agy") || command_exists("antigravity");
        let installed = has_cli || Path::new("/Applications/Antigravity.app").exists();
        let mut provider = base_provider("antigravity", "Antigravity", "Google OAuth");
        provider.installed = installed;
        provider.action_url = None;

        if !installed {
            provider.support = UsageSupport::Manual;
            provider.status_message = "Antigravity is not installed.".into();
            return provider;
        }

        if !has_cli {
            provider.support = UsageSupport::Manual;
            provider.status_message = "Antigravity CLI (agy) is not available in PATH.".into();
            return provider;
        }

        let bin = if command_exists("agy") {
            "agy"
        } else {
            "antigravity"
        };
        let mut cmd = tooling::command(bin);
        cmd.args(["-p", "/usage", "--output-format", "json"]);
        let output = match tooling::run_with_timeout(cmd, ANTIGRAVITY_USAGE_TIMEOUT) {
            Ok(output) => output,
            Err(error) => {
                provider.support = UsageSupport::Manual;
                provider.status_message = format!("Could not inspect Antigravity: {error}");
                return provider;
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            provider.support = UsageSupport::Manual;
            provider.status_message = if stderr.contains("login") || stderr.contains("auth") {
                "Sign in to Antigravity using `agy`.".into()
            } else {
                format!("Antigravity /usage exited with status {}", output.status)
            };
            return provider;
        }

        match serde_json::from_slice::<Value>(&output.stdout) {
            Ok(json_value) => {
                parse_antigravity_usage_json(&json_value, &mut provider);
            }
            Err(error) => {
                provider.support = UsageSupport::Manual;
                provider.status_message = format!("Failed to parse Antigravity output: {error}");
            }
        }

        provider
    }
}

pub fn parse_antigravity_usage_json(value: &Value, provider: &mut AiProviderUsage) {
    let Some(groups) = value
        .pointer("/command/data/groups")
        .and_then(Value::as_array)
    else {
        provider.support = UsageSupport::Manual;
        provider.status_message = "Antigravity returned unexpected usage format.".into();
        return;
    };

    let mut windows = Vec::new();
    for group in groups {
        let group_name = group.get("name").and_then(Value::as_str).unwrap_or("");
        let group_prefix = if group_name.contains("Gemini") {
            "Gemini"
        } else if group_name.contains("Claude") || group_name.contains("GPT") {
            "Claude/GPT"
        } else if !group_name.is_empty() {
            group_name
        } else {
            "Models"
        };

        if let Some(buckets) = group.get("buckets").and_then(Value::as_array) {
            for bucket in buckets {
                let window_type = bucket.get("window").and_then(Value::as_str).unwrap_or("");
                let window_label = match window_type {
                    "weekly" => "Weekly",
                    "5h" => "5h",
                    _ => bucket
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Limit"),
                };
                let label = format!("{group_prefix} · {window_label}");
                let Some(remaining) = bucket.get("remaining_fraction").and_then(Value::as_f64)
                else {
                    continue;
                };
                let used_percent = ((1.0 - remaining) * 100.0).clamp(0.0, 100.0);
                let resets_at = bucket
                    .get("reset_time")
                    .and_then(Value::as_str)
                    .and_then(parse_rfc3339_to_unix_secs);

                windows.push(UsageWindow {
                    label,
                    used_percent,
                    resets_at,
                });
            }
        }
    }

    windows.sort_by_key(|w| {
        let is_gemini = w.label.contains("Gemini");
        let group_order = if is_gemini { 0 } else { 1 };
        let window_order = if w.label.contains("5h") { 0 } else { 1 };
        (group_order, window_order)
    });

    if !windows.is_empty() {
        provider.connected = true;
        provider.support = UsageSupport::Live;
        provider.status_message = "Live limits from Antigravity CLI (/usage).".into();
        provider.windows = windows;
    } else {
        provider.support = UsageSupport::Manual;
        provider.status_message = "No rate limit buckets found in Antigravity response.".into();
    }
}
