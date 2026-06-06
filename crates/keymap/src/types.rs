// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Types for key combos, key events, and binding configuration.

use serde::Deserialize;
use std::fmt;

/// Modifier keys that can be held alongside a regular key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Super,
}

impl fmt::Display for Modifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Modifier::Ctrl => write!(f, "Ctrl"),
            Modifier::Alt => write!(f, "Alt"),
            Modifier::Shift => write!(f, "Shift"),
            Modifier::Super => write!(f, "Super"),
        }
    }
}

/// A key combination as specified in a config file (key + modifiers + optional flag).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct KeyCombo {
    pub key: String,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    #[serde(default)]
    pub after_prefix: bool,
}

/// A parsed key event from terminal input (key + active modifiers).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub key: String,
    pub modifiers: Vec<Modifier>,
}

impl KeyEvent {
    pub fn new(key: impl Into<String>, modifiers: Vec<Modifier>) -> Self {
        KeyEvent {
            key: key.into(),
            modifiers,
        }
    }

    /// Create a key event with no modifiers.
    pub fn plain(key: impl Into<String>) -> Self {
        KeyEvent {
            key: key.into(),
            modifiers: Vec::new(),
        }
    }
}

impl fmt::Display for KeyEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for m in &self.modifiers {
            write!(f, "{}-", m)?;
        }
        write!(f, "{}", self.key)
    }
}

/// A single binding entry: a key combo maps to an action name.
#[derive(Debug, Clone, Deserialize)]
pub struct BindingEntry {
    #[serde(flatten)]
    pub combo: KeyCombo,
    pub action: String,
}

/// Override: map a raw escape sequence to a synthetic KeyEvent for a specific terminal.
#[derive(Debug, Clone, Deserialize)]
pub struct TerminalOverride {
    pub raw_escape: String,
    pub maps_to: KeyCombo,
}

/// Top-level binding configuration loaded from JSON.
#[derive(Debug, Clone, Deserialize)]
pub struct BindingConfig {
    #[serde(default)]
    pub bindings: Vec<BindingEntry>,
    #[serde(default)]
    pub prefix: Option<KeyCombo>,
    #[serde(default)]
    pub terminal_overrides: std::collections::HashMap<String, Vec<TerminalOverride>>,
}
