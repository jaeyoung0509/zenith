use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

/// A secret string that redacts its content in Debug and Display implementations
/// to prevent accidental logging or leakage into error messages.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    StorageUnavailable(String),
    NotFound,
    OperationFailed(String),
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StorageUnavailable(msg) => {
                write!(f, "Secure credential storage is unavailable: {msg}")
            }
            Self::NotFound => write!(f, "Credential not found"),
            Self::OperationFailed(msg) => write!(f, "Credential operation failed: {msg}"),
        }
    }
}

impl std::error::Error for CredentialError {}

/// Abstract provider-keyed credential storage.
pub trait CredentialStore: Send + Sync {
    fn get(&self, provider_id: &str) -> Result<Option<SecretString>, CredentialError>;
    fn set(&self, provider_id: &str, secret: SecretString) -> Result<(), CredentialError>;
    fn remove(&self, provider_id: &str) -> Result<(), CredentialError>;
}

/// In-memory credential store, useful for tests, ephemeral sessions, and fallback mocking.
#[derive(Default, Clone)]
pub struct InMemoryCredentialStore {
    store: Arc<Mutex<HashMap<String, SecretString>>>,
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn get(&self, provider_id: &str) -> Result<Option<SecretString>, CredentialError> {
        let guard = self.store.lock().unwrap_or_else(|p| p.into_inner());
        Ok(guard.get(provider_id).cloned())
    }

    fn set(&self, provider_id: &str, secret: SecretString) -> Result<(), CredentialError> {
        let mut guard = self.store.lock().unwrap_or_else(|p| p.into_inner());
        guard.insert(provider_id.to_string(), secret);
        Ok(())
    }

    fn remove(&self, provider_id: &str) -> Result<(), CredentialError> {
        let mut guard = self.store.lock().unwrap_or_else(|p| p.into_inner());
        guard.remove(provider_id);
        Ok(())
    }
}

/// OS-backed secure credential store with in-memory session caching.
///
/// On macOS, uses `/usr/bin/security`.
/// On Windows, uses Windows Credential Manager / DPAPI.
#[derive(Default)]
pub struct OsCredentialStore {
    session_cache: Mutex<HashMap<String, SecretString>>,
}

impl OsCredentialStore {
    pub fn new() -> Self {
        Self {
            session_cache: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(target_os = "macos")]
    fn service_name(provider_id: &str) -> String {
        format!("app.zenith.ai.{provider_id}")
    }

    #[cfg(target_os = "macos")]
    fn get_os(&self, provider_id: &str) -> Result<Option<SecretString>, CredentialError> {
        let service = Self::service_name(provider_id);
        let mut cmd = std::process::Command::new("/usr/bin/security");
        cmd.args([
            "find-generic-password",
            "-a",
            "zenith",
            "-s",
            &service,
            "-w",
        ]);
        let output = cmd.output().map_err(|err| {
            CredentialError::StorageUnavailable(format!("Could not run security tool: {err}"))
        })?;

        if output.status.success() {
            let secret = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if secret.is_empty() {
                Ok(None)
            } else {
                Ok(Some(SecretString::new(secret)))
            }
        } else {
            // Exit code 44 indicates item not found
            let code = output.status.code().unwrap_or(-1);
            if code == 44 {
                Ok(None)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                Err(CredentialError::StorageUnavailable(format!(
                    "security tool exited with {code}: {stderr}"
                )))
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn set_os(&self, provider_id: &str, secret: &SecretString) -> Result<(), CredentialError> {
        let service = Self::service_name(provider_id);
        let mut cmd = std::process::Command::new("/usr/bin/security");
        cmd.args([
            "add-generic-password",
            "-U",
            "-a",
            "zenith",
            "-s",
            &service,
            "-w",
            secret.expose_secret(),
        ]);
        let output = cmd.output().map_err(|err| {
            CredentialError::StorageUnavailable(format!("Could not run security tool: {err}"))
        })?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(CredentialError::OperationFailed(format!(
                "Failed to save credential to macOS Keychain: {stderr}"
            )))
        }
    }

    #[cfg(target_os = "macos")]
    fn remove_os(&self, provider_id: &str) -> Result<(), CredentialError> {
        let service = Self::service_name(provider_id);
        let mut cmd = std::process::Command::new("/usr/bin/security");
        cmd.args(["delete-generic-password", "-a", "zenith", "-s", &service]);
        let output = cmd.output().map_err(|err| {
            CredentialError::StorageUnavailable(format!("Could not run security tool: {err}"))
        })?;

        let code = output.status.code().unwrap_or(-1);
        if output.status.success() || code == 44 {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(CredentialError::OperationFailed(format!(
                "Failed to remove credential from macOS Keychain: {stderr}"
            )))
        }
    }

    #[cfg(target_os = "windows")]
    fn target_name(provider_id: &str) -> String {
        format!("ZenithAI:{provider_id}")
    }

    #[cfg(target_os = "windows")]
    fn get_os(&self, provider_id: &str) -> Result<Option<SecretString>, CredentialError> {
        let target = Self::target_name(provider_id);
        let script = format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
             $cred = cmdkey /list:{} 2>&1; \
             if ($LASTEXITCODE -eq 0) {{ 'EXISTS' }} else {{ 'NOT_FOUND' }}",
            target
        );
        let mut cmd = crate::tooling::command("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        let output = cmd.output().map_err(|err| {
            CredentialError::StorageUnavailable(format!("PowerShell execution failed: {err}"))
        })?;
        let text = String::from_utf8_lossy(&output.stdout);
        if text.contains("EXISTS") {
            let guard = self.session_cache.lock().unwrap_or_else(|p| p.into_inner());
            Ok(guard.get(provider_id).cloned())
        } else {
            Ok(None)
        }
    }

    #[cfg(target_os = "windows")]
    fn set_os(&self, provider_id: &str, secret: &SecretString) -> Result<(), CredentialError> {
        let target = Self::target_name(provider_id);
        let mut cmd = crate::tooling::command("cmdkey");
        cmd.args([
            &format!("/generic:{target}"),
            "/user:zenith",
            &format!("/pass:{}", secret.expose_secret()),
        ]);
        let output = cmd.output().map_err(|err| {
            CredentialError::StorageUnavailable(format!("cmdkey execution failed: {err}"))
        })?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(CredentialError::OperationFailed(format!(
                "Failed to store credential in Windows Credential Manager: {stderr}"
            )))
        }
    }

    #[cfg(target_os = "windows")]
    fn remove_os(&self, provider_id: &str) -> Result<(), CredentialError> {
        let target = Self::target_name(provider_id);
        let mut cmd = crate::tooling::command("cmdkey");
        cmd.args([&format!("/delete:{target}")]);
        let output = cmd.output().map_err(|err| {
            CredentialError::StorageUnavailable(format!("cmdkey execution failed: {err}"))
        })?;
        if output.status.success() || output.status.code() == Some(1) {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(CredentialError::OperationFailed(format!(
                "Failed to remove credential from Windows Credential Manager: {stderr}"
            )))
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn get_os(&self, _provider_id: &str) -> Result<Option<SecretString>, CredentialError> {
        Ok(None)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn set_os(&self, _provider_id: &str, _secret: &SecretString) -> Result<(), CredentialError> {
        Err(CredentialError::StorageUnavailable(
            "OS secure storage not supported on this platform".into(),
        ))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn remove_os(&self, _provider_id: &str) -> Result<(), CredentialError> {
        Ok(())
    }
}

impl CredentialStore for OsCredentialStore {
    fn get(&self, provider_id: &str) -> Result<Option<SecretString>, CredentialError> {
        {
            let guard = self.session_cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(cached) = guard.get(provider_id) {
                return Ok(Some(cached.clone()));
            }
        }

        match self.get_os(provider_id)? {
            Some(secret) => {
                let mut guard = self.session_cache.lock().unwrap_or_else(|p| p.into_inner());
                guard.insert(provider_id.to_string(), secret.clone());
                Ok(Some(secret))
            }
            None => Ok(None),
        }
    }

    fn set(&self, provider_id: &str, secret: SecretString) -> Result<(), CredentialError> {
        self.set_os(provider_id, &secret)?;
        let mut guard = self.session_cache.lock().unwrap_or_else(|p| p.into_inner());
        guard.insert(provider_id.to_string(), secret);
        Ok(())
    }

    fn remove(&self, provider_id: &str) -> Result<(), CredentialError> {
        let _ = self.remove_os(provider_id);
        let mut guard = self.session_cache.lock().unwrap_or_else(|p| p.into_inner());
        guard.remove(provider_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_string_redacts_in_debug_and_display() {
        let secret = SecretString::new("super_secret_api_key_12345");
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
        assert_eq!(format!("{secret}"), "[REDACTED]");
        assert_eq!(secret.expose_secret(), "super_secret_api_key_12345");
    }

    #[test]
    fn in_memory_credential_store_crud() {
        let store = InMemoryCredentialStore::new();
        assert_eq!(store.get("openai-api").unwrap(), None);

        let secret = SecretString::new("sk-test-123");
        store.set("openai-api", secret.clone()).unwrap();
        assert_eq!(store.get("openai-api").unwrap(), Some(secret));

        store.remove("openai-api").unwrap();
        assert_eq!(store.get("openai-api").unwrap(), None);
    }
}
