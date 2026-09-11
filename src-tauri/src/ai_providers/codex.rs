use super::registry::ProviderRegistry;
use super::support::{append_rate_windows, base_provider, u64_field};
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId};
use crate::tooling;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

#[derive(Default)]
pub struct CodexAdapter;

impl ProviderAdapter for CodexAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::Codex)
            .expect("Codex must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, _ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let mut provider = base_provider(ProviderId::Codex, "Codex", "ChatGPT OAuth");
        provider.action_url = Some("https://chatgpt.com/codex/settings/usage".into());

        let mut child = match tooling::command("codex")
            .args(["app-server", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ProviderError::CliNotInstalled(
                    "Codex CLI was not detected in PATH or known tool locations.".into(),
                ));
            }
            Err(error) => {
                return Err(ProviderError::CliFailed(format!(
                    "Could not start Codex: {error}"
                )));
            }
        };
        provider.installed = true;

        let stdout = child.stdout.take().expect("piped Codex stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(message) = serde_json::from_str::<Value>(&line) {
                    let _ = tx.send(message);
                }
            }
        });

        if let Some(stdin) = child.stdin.as_mut() {
            let requests = [
                json!({"method":"initialize","id":0,"params":{"clientInfo":{"name":"zenith","title":"Zenith","version":"0.1.0"}}}),
                json!({"method":"initialized","params":{}}),
                json!({"method":"account/read","id":1,"params":{"refreshToken":false}}),
                json!({"method":"account/rateLimits/read","id":2}),
                json!({"method":"account/usage/read","id":3}),
            ];
            for request in requests {
                let _ = writeln!(stdin, "{request}");
            }
            let _ = stdin.flush();
        }

        let mut received = 0;
        while received < 3 {
            let Ok(message) = rx.recv_timeout(Duration::from_secs(4)) else {
                break;
            };
            match message.get("id").and_then(Value::as_u64) {
                Some(1) => {
                    received += 1;
                    if let Some(account) = message.pointer("/result/account") {
                        provider.connected =
                            account.get("type").and_then(Value::as_str) == Some("chatgpt");
                        let plan = account
                            .get("planType")
                            .and_then(Value::as_str)
                            .unwrap_or("ChatGPT");
                        provider.auth_label = format!("{plan} · OAuth");
                        provider.status_message = if provider.connected {
                            "Live account limits from the official Codex app-server.".into()
                        } else {
                            "Sign in with ChatGPT using `codex login`.".into()
                        };
                    }
                }
                Some(2) => {
                    received += 1;
                    if let Some(limits) = message.pointer("/result/rateLimitsByLimitId") {
                        if let Some(map) = limits.as_object() {
                            for value in map.values() {
                                append_rate_windows(&mut provider.windows, value);
                            }
                        }
                    } else if let Some(limits) = message.pointer("/result/rateLimits") {
                        append_rate_windows(&mut provider.windows, limits);
                    }
                }
                Some(3) => {
                    received += 1;
                    let summary = message.pointer("/result/summary").unwrap_or(&Value::Null);
                    provider.summary.lifetime_tokens = u64_field(summary, "lifetimeTokens");
                    provider.summary.peak_daily_tokens = u64_field(summary, "peakDailyTokens");
                    provider.summary.current_streak_days = u64_field(summary, "currentStreakDays");
                    provider.summary.last_7d_tokens = message
                        .pointer("/result/dailyUsageBuckets")
                        .and_then(Value::as_array)
                        .map(|days| {
                            days.iter()
                                .rev()
                                .take(7)
                                .filter_map(|day| u64_field(day, "tokens"))
                                .sum()
                        });
                }
                _ => {}
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        Ok(provider)
    }
}
