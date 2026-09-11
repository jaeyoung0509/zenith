use crate::models::ProviderId;
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

/// Abstract provider-keyed credential storage using type-safe ProviderId.
pub trait CredentialStore: Send + Sync {
    fn get(&self, provider: ProviderId) -> Result<Option<SecretString>, CredentialError>;
    fn set(&self, provider: ProviderId, secret: SecretString) -> Result<(), CredentialError>;
    fn remove(&self, provider: ProviderId) -> Result<(), CredentialError>;
}

/// In-memory credential store, useful for tests, ephemeral sessions, and fallback mocking.
#[derive(Default, Clone)]
pub struct InMemoryCredentialStore {
    store: Arc<Mutex<HashMap<ProviderId, SecretString>>>,
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn get(&self, provider: ProviderId) -> Result<Option<SecretString>, CredentialError> {
        let guard = self.store.lock().unwrap_or_else(|p| p.into_inner());
        Ok(guard.get(&provider).cloned())
    }

    fn set(&self, provider: ProviderId, secret: SecretString) -> Result<(), CredentialError> {
        let mut guard = self.store.lock().unwrap_or_else(|p| p.into_inner());
        guard.insert(provider, secret);
        Ok(())
    }

    fn remove(&self, provider: ProviderId) -> Result<(), CredentialError> {
        let mut guard = self.store.lock().unwrap_or_else(|p| p.into_inner());
        guard.remove(&provider);
        Ok(())
    }
}

/// OS-backed secure credential store with in-memory session caching.
///
/// On macOS, uses Keychain via `/usr/bin/security`.
/// On Windows, uses native Windows Credentials API (CredReadW, CredWriteW, CredDeleteW).
#[derive(Default)]
pub struct OsCredentialStore {
    session_cache: Mutex<HashMap<ProviderId, SecretString>>,
}

impl OsCredentialStore {
    pub fn new() -> Self {
        Self {
            session_cache: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(target_os = "macos")]
    fn service_name(provider: ProviderId) -> String {
        format!("app.zenith.ai.{provider}")
    }

    #[cfg(target_os = "macos")]
    fn get_os(&self, provider: ProviderId) -> Result<Option<SecretString>, CredentialError> {
        let service = Self::service_name(provider);
        match security_framework::passwords::get_generic_password(&service, "zenith") {
            Ok(bytes) => {
                let secret = String::from_utf8_lossy(&bytes).trim().to_string();
                if secret.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(SecretString::new(secret)))
                }
            }
            Err(err) => {
                let code = err.code();
                // errSecItemNotFound = -25300
                if code == -25300 {
                    Ok(None)
                } else {
                    Err(CredentialError::StorageUnavailable(format!(
                        "macOS Keychain error ({code}): {err}"
                    )))
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn set_os(&self, provider: ProviderId, secret: &SecretString) -> Result<(), CredentialError> {
        let service = Self::service_name(provider);
        security_framework::passwords::set_generic_password(
            &service,
            "zenith",
            secret.expose_secret().as_bytes(),
        )
        .map_err(|err| {
            CredentialError::OperationFailed(format!("Failed to save to macOS Keychain: {err}"))
        })
    }

    #[cfg(target_os = "macos")]
    fn remove_os(&self, provider: ProviderId) -> Result<(), CredentialError> {
        let service = Self::service_name(provider);
        match security_framework::passwords::delete_generic_password(&service, "zenith") {
            Ok(()) => Ok(()),
            Err(err) => {
                let code = err.code();
                // errSecItemNotFound = -25300
                if code == -25300 {
                    Ok(())
                } else {
                    Err(CredentialError::OperationFailed(format!(
                        "Failed to remove credential from macOS Keychain ({code}): {err}"
                    )))
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn target_name(provider: ProviderId) -> Vec<u16> {
        format!("ZenithAI:{provider}\0").encode_utf16().collect()
    }

    #[cfg(target_os = "windows")]
    fn get_os(&self, provider: ProviderId) -> Result<Option<SecretString>, CredentialError> {
        use std::ptr;
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{
            CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
        };

        let target_name = Self::target_name(provider);
        let mut cred_ptr: *mut CREDENTIALW = ptr::null_mut();

        let success =
            unsafe { CredReadW(target_name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred_ptr) };

        if success == 0 {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            if err == ERROR_NOT_FOUND {
                return Ok(None);
            }
            return Err(CredentialError::StorageUnavailable(format!(
                "CredReadW failed with Win32 error code {err}"
            )));
        }

        if cred_ptr.is_null() {
            return Ok(None);
        }

        let cred = unsafe { &*cred_ptr };
        let slice = unsafe {
            std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize)
        };
        let secret = String::from_utf8(slice.to_vec()).map_err(|e| {
            unsafe { CredFree(cred_ptr as _) };
            CredentialError::OperationFailed(format!("Invalid UTF-8 in credential blob: {e}"))
        })?;

        unsafe { CredFree(cred_ptr as _) };
        Ok(Some(SecretString::new(secret)))
    }

    #[cfg(target_os = "windows")]
    fn set_os(&self, provider: ProviderId, secret: &SecretString) -> Result<(), CredentialError> {
        use std::ptr;
        use windows_sys::Win32::Security::Credentials::{
            CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        };

        let target_name = Self::target_name(provider);
        let user_name: Vec<u16> = "zenith\0".encode_utf16().collect();
        let secret_bytes = secret.expose_secret().as_bytes();

        let cred = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: target_name.as_ptr() as *mut u16,
            Comment: ptr::null_mut(),
            LastWritten: unsafe { std::mem::zeroed() },
            CredentialBlobSize: secret_bytes.len() as u32,
            CredentialBlob: secret_bytes.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: ptr::null_mut(),
            TargetAlias: ptr::null_mut(),
            UserName: user_name.as_ptr() as *mut u16,
        };

        let success = unsafe { CredWriteW(&cred, 0) };
        if success == 0 {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            return Err(CredentialError::OperationFailed(format!(
                "CredWriteW failed with Win32 error code {err}"
            )));
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn remove_os(&self, provider: ProviderId) -> Result<(), CredentialError> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};

        let target_name = Self::target_name(provider);
        let success = unsafe { CredDeleteW(target_name.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if success == 0 {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            if err == ERROR_NOT_FOUND {
                return Ok(());
            }
            return Err(CredentialError::OperationFailed(format!(
                "CredDeleteW failed with Win32 error code {err}"
            )));
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn get_os(&self, _provider: ProviderId) -> Result<Option<SecretString>, CredentialError> {
        Ok(None)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn set_os(&self, _provider: ProviderId, _secret: &SecretString) -> Result<(), CredentialError> {
        Err(CredentialError::StorageUnavailable(
            "OS secure storage not supported on this platform".into(),
        ))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn remove_os(&self, _provider: ProviderId) -> Result<(), CredentialError> {
        Ok(())
    }
}

impl CredentialStore for OsCredentialStore {
    fn get(&self, provider: ProviderId) -> Result<Option<SecretString>, CredentialError> {
        {
            let guard = self.session_cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(cached) = guard.get(&provider) {
                return Ok(Some(cached.clone()));
            }
        }

        match self.get_os(provider)? {
            Some(secret) => {
                let mut guard = self.session_cache.lock().unwrap_or_else(|p| p.into_inner());
                guard.insert(provider, secret.clone());
                Ok(Some(secret))
            }
            None => Ok(None),
        }
    }

    fn set(&self, provider: ProviderId, secret: SecretString) -> Result<(), CredentialError> {
        self.set_os(provider, &secret)?;
        let mut guard = self.session_cache.lock().unwrap_or_else(|p| p.into_inner());
        guard.insert(provider, secret);
        Ok(())
    }

    fn remove(&self, provider: ProviderId) -> Result<(), CredentialError> {
        self.remove_os(provider)?;
        let mut guard = self.session_cache.lock().unwrap_or_else(|p| p.into_inner());
        guard.remove(&provider);
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
        assert_eq!(store.get(ProviderId::OpenAiApi).unwrap(), None);

        let secret = SecretString::new("sk-test-123");
        store.set(ProviderId::OpenAiApi, secret.clone()).unwrap();
        assert_eq!(store.get(ProviderId::OpenAiApi).unwrap(), Some(secret));

        store.remove(ProviderId::OpenAiApi).unwrap();
        assert_eq!(store.get(ProviderId::OpenAiApi).unwrap(), None);
    }
}
