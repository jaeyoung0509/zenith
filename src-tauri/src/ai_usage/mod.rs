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
        Self::collect_parallel(openrouter_key, |_| {})
    }

    pub fn collect_parallel<F>(openrouter_key: Option<String>, on_provider: F) -> AiUsageSnapshot
    where
        F: Fn(AiProviderUsage) + Send + Sync,
    {
        let on_p = &on_provider;
        let (codex, claude, opencode, openrouter, antigravity) = std::thread::scope(|s| {
            let h_codex = s.spawn(move || {
                let p = Self::collect_codex();
                on_p(p.clone());
                p
            });
            let h_claude = s.spawn(move || {
                let p = Self::collect_claude();
                on_p(p.clone());
                p
            });
            let h_opencode = s.spawn(move || {
                let p = Self::collect_opencode();
                on_p(p.clone());
                p
            });
            let h_openrouter = s.spawn(move || {
                let p = Self::collect_openrouter(openrouter_key.as_deref());
                on_p(p.clone());
                p
            });
            let h_antigravity = s.spawn(move || {
                let p = Self::collect_antigravity();
                on_p(p.clone());
                p
            });

            (
                h_codex
                    .join()
                    .unwrap_or_else(|_| Self::failed_provider("codex", "Codex")),
                h_claude
                    .join()
                    .unwrap_or_else(|_| Self::failed_provider("claude", "Claude Code")),
                h_opencode
                    .join()
                    .unwrap_or_else(|_| Self::failed_provider("opencode", "OpenCode")),
                h_openrouter
                    .join()
                    .unwrap_or_else(|_| Self::failed_provider("openrouter", "OpenRouter")),
                h_antigravity
                    .join()
                    .unwrap_or_else(|_| Self::failed_provider("antigravity", "Antigravity")),
            )
        });

        AiUsageSnapshot {
            providers: vec![codex, claude, opencode, openrouter, antigravity],
            fetched_at: now_secs(),
        }
    }

    fn failed_provider(id: &str, name: &str) -> AiProviderUsage {
        let mut provider = base_provider(id, name, "Unknown");
        provider.support = UsageSupport::Manual;
        provider.status_message = "Collector thread failed.".into();
        provider
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

        let client = match reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(err) => {
                let msg = format!("Failed to create OpenRouter HTTP client: {err}");
                crate::diagnostics::log_error("ai_usage", &msg);
                provider.status_message = msg;
                return provider;
            }
        };

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
                let msg = format!("OpenRouter usage request failed: {error}");
                crate::diagnostics::log_error("ai_usage", &msg);
                provider.status_message = msg;
            }
        }
        provider
    }

    fn collect_antigravity() -> AiProviderUsage {
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
        let output = match tooling::run_with_timeout(cmd, Duration::from_secs(8)) {
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
        .map_err(|err| {
            let msg = format!("Failed to create OpenRouter HTTP client: {err}");
            crate::diagnostics::log_error("ai_usage", &msg);
            msg
        })?;

    let response = client
        .post("https://openrouter.ai/api/v1/auth/keys")
        .json(&json!({
            "code": code,
            "code_verifier": verifier,
            "code_challenge_method": "S256"
        }))
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| {
            let msg = format!("OpenRouter token exchange failed: {error}");
            crate::diagnostics::log_error("ai_usage", &msg);
            msg
        })?
        .json::<Value>()
        .map_err(|error| {
            let msg = error.to_string();
            crate::diagnostics::log_error("ai_usage", &msg);
            msg
        })?;

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

fn parse_antigravity_usage_json(value: &Value, provider: &mut AiProviderUsage) {
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

    // Sort windows so shorter windows (5h) appear before longer ones (Weekly)
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

pub(crate) fn parse_rfc3339_to_unix_secs(s: &str) -> Option<u64> {
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

fn is_leap_year(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_rfc3339_correctly() {
        assert_eq!(
            parse_rfc3339_to_unix_secs("2026-09-02T03:13:59Z"),
            Some(1788318839)
        );
        assert_eq!(
            parse_rfc3339_to_unix_secs("2026-08-29T17:05:20Z"),
            Some(1788023120)
        );
        assert_eq!(
            parse_rfc3339_to_unix_secs("2026-09-02T03:13:59.123456Z"),
            Some(1788318839)
        );
        assert_eq!(parse_rfc3339_to_unix_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_to_unix_secs("2026-09-02T12:13:59+09:00"),
            Some(1788318839)
        );
        assert_eq!(parse_rfc3339_to_unix_secs("invalid"), None);
        assert_eq!(parse_rfc3339_to_unix_secs("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_rfc3339_to_unix_secs("2026-02-29T00:00:00Z"), None);
        assert_eq!(parse_rfc3339_to_unix_secs("2026-09-02 03:13:59Z"), None);
        assert_eq!(parse_rfc3339_to_unix_secs("2026-09-02T03:13:59"), None);
        assert_eq!(parse_rfc3339_to_unix_secs("2026-09-02T03:13:59.Z"), None);
        assert_eq!(parse_rfc3339_to_unix_secs("2026-09-02T03:13:59Zjunk"), None);
        assert_eq!(
            parse_rfc3339_to_unix_secs("2026-09-02T03:13:59+24:00"),
            None
        );
        assert_eq!(parse_rfc3339_to_unix_secs("2026-09-02T03:13:59+09"), None);
    }

    #[test]
    fn parses_antigravity_usage_json_buckets() {
        let sample = json!({
            "status": "SUCCESS",
            "command": {
                "name": "usage",
                "data": {
                    "groups": [
                        {
                            "name": "Gemini Models",
                            "buckets": [
                                {
                                    "id": "gemini-weekly",
                                    "name": "Weekly Limit Remaining",
                                    "window": "weekly",
                                    "remaining_fraction": 0.792,
                                    "reset_time": "2026-09-02T03:13:59Z"
                                },
                                {
                                    "id": "gemini-5h",
                                    "name": "Five Hour Limit Remaining",
                                    "window": "5h",
                                    "remaining_fraction": 0.988,
                                    "reset_time": "2026-08-29T17:05:20Z"
                                },
                                {
                                    "id": "malformed",
                                    "name": "Bucket without remaining fraction",
                                    "window": "5h",
                                    "reset_time": "2026-08-29T17:05:20Z"
                                }
                            ]
                        },
                        {
                            "name": "Claude and GPT models",
                            "buckets": [
                                {
                                    "id": "3p-weekly",
                                    "name": "Weekly Limit Remaining",
                                    "window": "weekly",
                                    "remaining_fraction": 1.0,
                                    "reset_time": "2026-09-05T12:07:16Z"
                                }
                            ]
                        }
                    ]
                }
            }
        });

        let mut provider = base_provider("antigravity", "Antigravity", "Google OAuth");
        parse_antigravity_usage_json(&sample, &mut provider);

        assert!(provider.connected);
        assert!(matches!(provider.support, UsageSupport::Live));
        assert_eq!(provider.windows.len(), 3);
        assert_eq!(provider.windows[0].label, "Gemini · 5h");
        assert!((provider.windows[0].used_percent - 1.2).abs() < 0.1);
        assert_eq!(provider.windows[0].resets_at, Some(1788023120));

        assert_eq!(provider.windows[1].label, "Gemini · Weekly");
        assert!((provider.windows[1].used_percent - 20.8).abs() < 0.1);
        assert_eq!(provider.windows[1].resets_at, Some(1788318839));

        assert_eq!(provider.windows[2].label, "Claude/GPT · Weekly");
        assert_eq!(provider.windows[2].used_percent, 0.0);
    }
}
