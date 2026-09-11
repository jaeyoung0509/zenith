use super::credentials::CredentialStore;
use super::support::base_provider;
use super::ProviderAdapter;
use crate::models::{AiProviderUsage, UsageSupport};
use crate::tooling;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::Duration;
use url::Url;
use uuid::Uuid;

#[derive(Default)]
pub struct OpenRouterAdapter;

impl ProviderAdapter for OpenRouterAdapter {
    fn id(&self) -> &'static str {
        "openrouter"
    }

    fn collect(&self, credentials: &dyn CredentialStore) -> AiProviderUsage {
        let mut provider = base_provider("openrouter", "OpenRouter", "OAuth PKCE");
        provider.installed = true;
        provider.connected = false;
        provider.support = UsageSupport::Live;
        provider.status_message = "No Zenith OAuth session is connected yet.".into();
        provider.action_url = Some("https://openrouter.ai/activity".into());

        let key = match credentials.get("openrouter") {
            Ok(Some(secret)) => secret,
            _ => return provider,
        };

        let client = match reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(err) => {
                let msg = format!("Failed to create OpenRouter HTTP client: {err}");
                crate::diagnostics::log_error("ai_providers", &msg);
                provider.status_message = msg;
                return provider;
            }
        };

        match client
            .get("https://openrouter.ai/api/v1/key")
            .bearer_auth(key.expose_secret())
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
                crate::diagnostics::log_error("ai_providers", &msg);
                provider.status_message = msg;
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
