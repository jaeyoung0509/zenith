use crate::models::{AiProviderUsage, AiUsageSnapshot, UsageSummary, UsageSupport, UsageWindow};
use crate::tooling;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, SystemTime};
use url::Url;
use uuid::Uuid;

pub struct AiUsageCollector;

impl AiUsageCollector {
    pub fn collect(openrouter_key: Option<String>) -> AiUsageSnapshot {
        AiUsageSnapshot {
            providers: vec![
                Self::collect_codex(),
                Self::collect_claude(),
                Self::collect_opencode(),
                Self::collect_openrouter(openrouter_key.as_deref()),
                Self::collect_antigravity(),
            ],
            fetched_at: now_secs(),
        }
    }

    fn collect_codex() -> AiProviderUsage {
        let mut provider = base_provider("codex", "Codex", "ChatGPT OAuth");
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
                provider.status_message = "Codex CLI is not installed.".into();
                return provider;
            }
            Err(error) => {
                provider.installed = true;
                provider.status_message = format!("Could not start Codex: {error}");
                return provider;
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
        provider
    }

    fn collect_claude() -> AiProviderUsage {
        let installed = command_exists("claude");
        AiProviderUsage {
            id: "claude".into(),
            name: "Claude Code".into(),
            installed,
            connected: false,
            auth_label: "Claude.ai OAuth".into(),
            status_message: if installed {
                "Claude exposes subscription limits inside `/usage`; no external OAuth usage API is public.".into()
            } else {
                "Claude Code is not installed.".into()
            },
            support: UsageSupport::Manual,
            windows: vec![],
            summary: UsageSummary::default(),
            action_url: Some("https://claude.ai/settings/usage".into()),
        }
    }

    fn collect_opencode() -> AiProviderUsage {
        let mut provider = base_provider("opencode", "OpenCode", "Provider OAuth");
        provider.support = UsageSupport::Local;
        provider.action_url = Some("https://opencode.ai/docs/providers".into());

        let mut auth_cmd = tooling::command("opencode");
        auth_cmd.args(["auth", "list"]);
        let auth = match tooling::run_with_timeout(auth_cmd, Duration::from_secs(4)) {
            Ok(output) => output,
            Err(error) => {
                let error_str = error.to_string();
                if error_str.contains("No such file") || error_str.contains("not found") {
                    provider.status_message = "OpenCode is not installed.".into();
                } else {
                    provider.installed = true;
                    provider.status_message = format!("Could not inspect OpenCode: {error}");
                }
                return provider;
            }
        };
        provider.installed = true;
        let auth_output = strip_ansi(&String::from_utf8_lossy(&auth.stdout));
        let oauth_count = auth_output
            .lines()
            .filter(|line| line.to_ascii_lowercase().contains("oauth"))
            .count();
        provider.connected = oauth_count > 0;
        provider.auth_label = format!(
            "{oauth_count} OAuth provider{}",
            if oauth_count == 1 { "" } else { "s" }
        );
        provider.status_message =
            "Local activity from `opencode stats`; quotas remain provider-owned.".into();

        let mut stats_cmd = tooling::command("opencode");
        stats_cmd.args(["stats", "--days", "7"]);
        if let Ok(output) = tooling::run_with_timeout(stats_cmd, Duration::from_secs(4)) {
            let stats = strip_ansi(&String::from_utf8_lossy(&output.stdout));
            provider.summary.local_sessions = parse_stat_u64(&stats, "Sessions");
            provider.summary.local_cost_usd = parse_stat_f64(&stats, "Total Cost");
        }
        provider
    }

    fn collect_openrouter(key: Option<&str>) -> AiProviderUsage {
        let mut provider = AiProviderUsage {
            id: "openrouter".into(),
            name: "OpenRouter".into(),
            installed: true,
            connected: false,
            auth_label: "OAuth PKCE".into(),
            status_message: "No Zenith OAuth session is connected yet.".into(),
            support: UsageSupport::Live,
            windows: vec![],
            summary: UsageSummary::default(),
            action_url: Some("https://openrouter.ai/activity".into()),
        };

        let Some(key) = key else {
            return provider;
        };

        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());

        match client
            .get("https://openrouter.ai/api/v1/key")
            .bearer_auth(key)
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json::<Value>())
        {
            Ok(response) => {
                provider.connected = true;
                provider.status_message = "Live key usage from OpenRouter OAuth.".into();
                provider.summary.usage_usd =
                    response.pointer("/data/usage").and_then(Value::as_f64);
                provider.summary.limit_remaining_usd = response
                    .pointer("/data/limit_remaining")
                    .and_then(Value::as_f64);
            }
            Err(error) => {
                provider.status_message = format!("OpenRouter usage request failed: {error}");
            }
        }
        provider
    }

    fn collect_antigravity() -> AiProviderUsage {
        let installed = Path::new("/Applications/Antigravity.app").exists();
        AiProviderUsage {
            id: "antigravity".into(),
            name: "Antigravity".into(),
            installed,
            connected: false,
            auth_label: "Google OAuth".into(),
            status_message: "Google does not currently publish an Antigravity account-usage API."
                .into(),
            support: UsageSupport::Manual,
            windows: vec![],
            summary: UsageSummary::default(),
            action_url: None,
        }
    }
}

pub fn connect_openrouter() -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let callback = format!("http://localhost:{port}/callback");
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    let mut auth_url =
        Url::parse("https://openrouter.ai/auth").map_err(|error| error.to_string())?;
    auth_url
        .query_pairs_mut()
        .append_pair("callback_url", &callback)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");

    tooling::command("open")
        .arg(auth_url.as_str())
        .spawn()
        .map_err(|error| format!("Could not open the OAuth page: {error}"))?;

    let started = std::time::Instant::now();
    let code = loop {
        if started.elapsed() > Duration::from_secs(180) {
            return Err("OpenRouter sign-in timed out.".into());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut first_line = String::new();
                BufReader::new(stream.try_clone().map_err(|error| error.to_string())?)
                    .read_line(&mut first_line)
                    .map_err(|error| error.to_string())?;
                let path = first_line.split_whitespace().nth(1).unwrap_or("/");
                let callback_url = Url::parse(&format!("http://localhost{path}"))
                    .map_err(|error| error.to_string())?;
                let oauth_code = callback_url
                    .query_pairs()
                    .find(|(key, _)| key == "code")
                    .map(|(_, value)| value.into_owned());
                let (status, body) = if oauth_code.is_some() {
                    (
                        "200 OK",
                        "OpenRouter connected to Zenith. You can close this tab.",
                    )
                } else {
                    ("400 Bad Request", "OpenRouter authorization was cancelled.")
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                if let Some(code) = oauth_code {
                    break code;
                }
                return Err("OpenRouter authorization was cancelled.".into());
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error.to_string()),
        }
    };

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new());

    let response = client
        .post("https://openrouter.ai/api/v1/auth/keys")
        .json(&json!({
            "code": code,
            "code_verifier": verifier,
            "code_challenge_method": "S256"
        }))
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("OpenRouter token exchange failed: {error}"))?
        .json::<Value>()
        .map_err(|error| error.to_string())?;

    response
        .get("key")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "OpenRouter did not return an OAuth key.".into())
}

fn base_provider(id: &str, name: &str, auth_label: &str) -> AiProviderUsage {
    AiProviderUsage {
        id: id.into(),
        name: name.into(),
        installed: false,
        connected: false,
        auth_label: auth_label.into(),
        status_message: "Not connected".into(),
        support: UsageSupport::Live,
        windows: vec![],
        summary: UsageSummary::default(),
        action_url: None,
    }
}

fn append_rate_windows(target: &mut Vec<UsageWindow>, limits: &Value) {
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

fn u64_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn command_exists(command: &str) -> bool {
    tooling::resolve(command).is_some()
}

fn parse_stat_u64(output: &str, label: &str) -> Option<u64> {
    output.lines().find_map(|line| {
        let (_, value) = line.split_once(label)?;
        value
            .trim_matches(|character: char| !character.is_ascii_digit())
            .parse()
            .ok()
    })
}

fn parse_stat_f64(output: &str, label: &str) -> Option<f64> {
    output.lines().find_map(|line| {
        let (_, value) = line.split_once(label)?;
        value
            .trim_matches(|character: char| !character.is_ascii_digit() && character != '.')
            .parse()
            .ok()
    })
}

fn strip_ansi(input: &str) -> String {
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

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
