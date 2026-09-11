use super::registry::ProviderRegistry;
use super::support::base_provider;
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId, UsageSupport};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::Duration;
use url::Url;
use uuid::Uuid;

#[derive(Default)]
pub struct OpenRouterAdapter;

impl ProviderAdapter for OpenRouterAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::OpenRouter
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::OpenRouter)
            .expect("OpenRouter must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let mut provider = base_provider(ProviderId::OpenRouter, "OpenRouter", "OAuth PKCE");
        provider.installed = true;
        provider.connected = false;
        provider.support = UsageSupport::Live;
        provider.status_message = "No Zenith OAuth session is connected yet.".into();
        provider.action_url = Some("https://openrouter.ai/activity".into());

        let key = match ctx.credentials.get(ProviderId::OpenRouter) {
            Ok(Some(secret)) => secret,
            _ => return Ok(provider),
        };

        let response = ctx
            .http_client
            .get("https://openrouter.ai/api/v1/key")
            .bearer_auth(key.expose_secret())
            .send()
            .map_err(|err| ProviderError::Network(err.to_string()))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ProviderError::AuthenticationFailed(
                "OpenRouter session expired or invalid.".into(),
            ));
        }
        if !status.is_success() {
            return Err(ProviderError::Network(format!(
                "API returned HTTP status {status}"
            )));
        }

        let data = response
            .json::<Value>()
            .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;

        provider.connected = true;
        provider.status_message = "Live key usage from OpenRouter OAuth.".into();
        provider.summary.usage_usd = data.pointer("/data/usage").and_then(Value::as_f64);
        provider.summary.limit_remaining_usd = data
            .pointer("/data/limit_remaining")
            .and_then(Value::as_f64);

        Ok(provider)
    }
}

const OAUTH_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_CALLBACK_BYTES: u64 = 8 * 1024;

enum CallbackOutcome {
    Authorized(String),
    Rejected,
}

/// The callback URL passed to OpenRouter. The CSRF `state` travels as a query
/// parameter of the callback URL, which is the shape OpenRouter echoes back on
/// redirect; it is not a top-level parameter of the authorization URL.
fn callback_url(port: u16, state: &str) -> Result<Url, String> {
    let mut callback =
        Url::parse(&format!("http://127.0.0.1:{port}/callback")).map_err(|e| e.to_string())?;
    callback.query_pairs_mut().append_pair("state", state);
    Ok(callback)
}

fn authorization_url(callback: &Url, challenge: &str) -> Result<Url, String> {
    let mut auth_url =
        Url::parse("https://openrouter.ai/auth").map_err(|error| error.to_string())?;
    auth_url
        .query_pairs_mut()
        .append_pair("callback_url", callback.as_str())
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(auth_url)
}

/// Validates the loopback callback without touching the network. The `state`
/// value must match exactly, and a callback without one is rejected.
fn parse_callback(request_line: &str, expected_state: &str) -> CallbackOutcome {
    let target = if request_line.trim_start().starts_with('/') {
        request_line.trim()
    } else {
        match request_line.split_whitespace().nth(1) {
            Some(target) => target,
            None => return CallbackOutcome::Rejected,
        }
    };
    let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        return CallbackOutcome::Rejected;
    };
    let mut code = None;
    let mut state = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            _ => {}
        }
    }
    match (code, state) {
        (Some(code), Some(state)) if !code.is_empty() && state == expected_state => {
            CallbackOutcome::Authorized(code)
        }
        _ => CallbackOutcome::Rejected,
    }
}

fn write_callback_response(stream: &mut std::net::TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

/// Reads one bounded request line with a deadline. The listener is
/// non-blocking, but an accepted `TcpStream` inherits blocking reads, so a
/// local process that connects without sending anything must be bounded here.
fn read_callback_line(
    stream: &mut std::net::TcpStream,
    timeout: Duration,
) -> std::io::Result<String> {
    stream.set_read_timeout(Some(timeout))?;
    let mut request_line = String::new();
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut limited = (&mut reader).take(MAX_CALLBACK_BYTES);
    limited.read_line(&mut request_line)?;
    Ok(request_line)
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
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    // High-entropy CSRF state: two UUIDv4 values keep the callback bound to this
    // specific authorization attempt.
    let state = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let callback = callback_url(port, &state)?;
    let auth_url = authorization_url(&callback, &challenge)?;

    open_browser(auth_url.as_str())?;

    let started = std::time::Instant::now();
    let code = loop {
        if started.elapsed() > OAUTH_TIMEOUT {
            return Err("OpenRouter sign-in timed out.".into());
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    write_callback_response(
                        &mut stream,
                        "403 Forbidden",
                        "The OpenRouter callback must come from this machine.",
                    );
                    continue;
                }
                let remaining = OAUTH_TIMEOUT.saturating_sub(started.elapsed());
                let connection_timeout = remaining.min(Duration::from_secs(5));
                match read_callback_line(&mut stream, connection_timeout) {
                    Ok(request_line) => match parse_callback(&request_line, &state) {
                        CallbackOutcome::Authorized(code) => {
                            write_callback_response(
                                &mut stream,
                                "200 OK",
                                "OpenRouter connected to Zenith. You can close this tab.",
                            );
                            break code;
                        }
                        CallbackOutcome::Rejected => {
                            // An unsolicited or stale request must not abort a
                            // legitimate flow that is still pending.
                            write_callback_response(
                                &mut stream,
                                "400 Bad Request",
                                "OpenRouter authorization was not accepted.",
                            );
                        }
                    },
                    Err(_) => {
                        // A silent, slow, or malformed connection must not
                        // block the flow: drop it and keep accepting until the
                        // overall deadline.
                        write_callback_response(
                            &mut stream,
                            "408 Request Timeout",
                            "The callback request timed out.",
                        );
                    }
                }
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
            crate::diagnostics::log_error("ai_providers", &msg);
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
            crate::diagnostics::log_error("ai_providers", &msg);
            msg
        })?
        .json::<Value>()
        .map_err(|error| {
            let msg = error.to_string();
            crate::diagnostics::log_error("ai_providers", &msg);
            msg
        })?;

    response
        .get("key")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "OpenRouter did not return an OAuth key.".into())
}

fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        crate::tooling::command("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Could not open the OAuth page: {error}"))
    }
    #[cfg(target_os = "windows")]
    {
        // ShellExecuteW hands the URL to the default browser directly. Going
        // through `cmd /C start` would re-parse the `&` separators that every
        // OAuth authorization URL contains.
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let operation = std::ffi::OsStr::new("open")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let target = std::ffi::OsStr::new(url)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize > 32 {
            Ok(())
        } else {
            Err(format!(
                "Could not open the OAuth page (ShellExecuteW returned {})",
                result as isize
            ))
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        crate::tooling::command("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Could not open the OAuth page: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_requires_a_matching_state() {
        assert!(matches!(
            parse_callback("/callback?code=abc&state=expected", "expected"),
            CallbackOutcome::Authorized(code) if code == "abc"
        ));
        assert!(matches!(
            parse_callback("/callback?code=abc&state=other", "expected"),
            CallbackOutcome::Rejected
        ));
        assert!(matches!(
            parse_callback("/callback?code=abc", "expected"),
            CallbackOutcome::Rejected
        ));
        assert!(matches!(
            parse_callback("/callback?code=&state=expected", "expected"),
            CallbackOutcome::Rejected
        ));
        assert!(matches!(
            parse_callback("GET /callback", "expected"),
            CallbackOutcome::Rejected
        ));
    }

    #[test]
    fn callback_url_carries_state_and_the_auth_url_does_not() {
        let callback = callback_url(12345, "state-value").unwrap();
        let callback_pairs = callback
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            callback_pairs.get("state").map(String::as_str),
            Some("state-value")
        );

        let auth = authorization_url(&callback, "challenge-value").unwrap();
        let auth_pairs = auth
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<std::collections::HashMap<_, _>>();
        assert!(
            !auth_pairs.contains_key("state"),
            "state must travel inside callback_url, not as a top-level auth parameter"
        );
        let forwarded = auth_pairs.get("callback_url").unwrap();
        assert!(forwarded.contains("state=state-value"));
        assert!(forwarded.starts_with("http://127.0.0.1:12345/callback"));
        assert_eq!(
            auth_pairs.get("code_challenge").map(String::as_str),
            Some("challenge-value")
        );
        assert_eq!(
            auth_pairs.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
    }

    #[test]
    fn only_loopback_peers_are_accepted() {
        assert!(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST).is_loopback());
        assert!(!std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 10)).is_loopback());
    }

    #[test]
    fn a_silent_connection_times_out_and_a_later_callback_is_still_accepted() {
        use std::io::Write as _;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        // A local process connects and sends nothing. The read must return
        // within the bounded timeout instead of blocking the OAuth flow.
        let _silent = std::net::TcpStream::connect(addr).unwrap();
        let (mut accepted, _) = listener.accept().unwrap();
        let started = std::time::Instant::now();
        let result = read_callback_line(&mut accepted, Duration::from_millis(100));
        assert!(result.is_err(), "a silent connection must time out");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the read deadline must bound the wait"
        );

        // The loop can still accept and parse the real callback afterwards.
        let mut client = std::net::TcpStream::connect(addr).unwrap();
        client
            .write_all(b"GET /callback?code=ok&state=expected HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let (mut accepted, _) = listener.accept().unwrap();
        let line = read_callback_line(&mut accepted, Duration::from_secs(2)).unwrap();
        assert!(matches!(
            parse_callback(&line, "expected"),
            CallbackOutcome::Authorized(code) if code == "ok"
        ));
    }
}
