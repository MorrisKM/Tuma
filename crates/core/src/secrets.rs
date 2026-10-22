//! Secret variable values live in the OS keychain (macOS Keychain, Windows Credential Manager, the Secret Service on
//! Linux), never in workspace files, so committing a workspace is safe.

use std::collections::HashMap;
use std::sync::Mutex;

pub trait SecretStore: Send + Sync {
    /// `Ok(None)` when no secret is stored under `key`.
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    /// Deleting a key that does not exist is not an error.
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// The OS keychain. Every entry is under the service name `tuma`.
pub struct Keychain;

const SERVICE: &str = "tuma";

impl SecretStore for Keychain {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(SERVICE, key).and_then(|e| e.get_password()) {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, key)
            .and_then(|e| e.set_password(value))
            .map_err(|e| e.to_string())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, key).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// For the CLI and CI: a secret can come from an environment variable named `TUMA_SECRET_` plus the variable's name in
/// upper case (`apiKey` → `TUMA_SECRET_APIKEY`), which wins over the keychain. When there is no keychain, as on most CI
/// machines, a secret without an environment variable reads as missing instead of failing.
pub struct EnvThenKeychain;

impl EnvThenKeychain {
    fn env_name(key: &str) -> String {
        let name = key.rsplit('/').next().unwrap_or(key);
        let name: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        format!("TUMA_SECRET_{name}")
    }
}

impl SecretStore for EnvThenKeychain {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        if let Ok(value) = std::env::var(Self::env_name(key)) {
            return Ok(Some(value));
        }
        Ok(Keychain.get(key).unwrap_or(None))
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        Keychain.set(key, value)
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        Keychain.delete(key)
    }
}

/// Keeps secrets in memory, for tests and machines without a keychain.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}
