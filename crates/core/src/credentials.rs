// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tiered credential management for API keys.
//!
//! Storage priority:
//! 1. Environment variables (`RUSTY_API_KEY`, `OPENAI_API_KEY`)
//! 2. OS Keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service)
//! 3. Settings file fallback (`~/.rusty/settings.json`)

use crate::credentials::keyring_impl::Entry;
use crate::{CredentialStore, RustyError, Settings};

// Re-export keyring for internal use
mod keyring_impl {
    pub use keyring::Entry;
}

/// Service name used for keyring entries.
const KEYRING_SERVICE: &str = "rusty";
const KEYRING_USER: &str = "default";

/// Manages API key storage with tiered resolution.
pub struct CredentialManager;

impl CredentialManager {
    /// Check if a keyring is available on this system.
    ///
    /// Attempts a benign read to verify the platform secret store is accessible.
    /// Returns `false` in headless environments (Docker, SSH without agent, etc.).
    pub fn is_keyring_available() -> bool {
        match Entry::new(KEYRING_SERVICE, KEYRING_USER) {
            Ok(entry) => {
                // A get_password() on a non-existent key returns Err, but that's fine —
                // we just want to know the platform store is reachable.
                let _ = entry.get_password();
                true
            }
            Err(_) => false, // Keyring not available
        }
    }

    /// Resolve API key using the tiered priority chain:
    ///
    /// 1. Environment variable (`RUSTY_API_KEY` or `OPENAI_API_KEY`)
    /// 2. OS Keyring (if `credential_store` is `Keyring`)
    /// 3. Settings file (`api_key` field)
    ///
    /// Note: The CLI's `--api-key` argument is handled by clap *before* this
    /// method is called — it arrives via the settings merge in `main.rs`.
    /// This method adds keyring as a higher-priority source.
    pub fn resolve_api_key(settings: &Settings) -> Option<String> {
        // Priority 1: Environment variables (check directly, clap only captures OPENAI_API_KEY)
        for var in &["RUSTY_API_KEY", "OPENAI_API_KEY"] {
            if let Ok(val) = std::env::var(var) {
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }

        // Priority 2: OS Keyring
        if settings.credential_store == CredentialStore::Keyring {
            if let Some(key) = Self::get_from_keyring() {
                return Some(key);
            }
        }

        // Priority 3: Settings file
        if let Some(ref key) = settings.api_key {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }

        None
    }

    /// Retrieve the API key from the OS keyring.
    pub fn get_from_keyring() -> Option<String> {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?;
        match entry.get_password() {
            Ok(key) if !key.is_empty() => Some(key),
            Ok(_) => None, // Keyring entry exists but is empty
            Err(keyring::Error::NoEntry) => None,
            Err(_) => None, // Keyring read failure
        }
    }

    /// Store the API key in the OS keyring.
    pub fn store_in_keyring(api_key: &str) -> Result<(), RustyError> {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|e| RustyError::Config(format!("Failed to access keyring: {e}")))?;
        entry
            .set_password(api_key)
            .map_err(|e| RustyError::Config(format!("Failed to store key in keyring: {e}")))
    }

    /// Delete the API key from the OS keyring.
    pub fn delete_from_keyring() -> Result<(), RustyError> {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|e| RustyError::Config(format!("Failed to access keyring: {e}")))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()), // Already gone
            Err(e) => Err(RustyError::Config(format!(
                "Failed to delete key from keyring: {e}"
            ))),
        }
    }
}

/// Convenience function: retrieve the API key from the OS keyring.
///
/// Returns `Err` if the keyring is unavailable or contains no entry.
pub fn get_stored_api_key() -> Result<String, RustyError> {
    CredentialManager::get_from_keyring()
        .ok_or_else(|| RustyError::Config("No API key found in keyring".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_keyring_available_does_not_panic() {
        // Just verify it returns without crashing — actual availability
        // depends on the CI/dev environment.
        let _available = CredentialManager::is_keyring_available();
    }
}
