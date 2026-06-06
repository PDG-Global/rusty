// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! KeyMap: maps key events to action names, with optional prefix-key support.

use std::collections::HashMap;

use crate::types::{BindingConfig, KeyCombo, KeyEvent};

/// A KeyMap built from a `BindingConfig`. Supports simple bindings and
/// prefix-key bindings (like tmux: press prefix, then a second key).
pub struct KeyMap {
    /// Simple bindings: KeyEvent -> action name.
    simple: HashMap<KeyEvent, String>,
    /// Prefix bindings: KeyEvent -> action name (active only after prefix).
    prefixed: HashMap<KeyEvent, String>,
    /// The prefix key itself (if configured).
    prefix: Option<KeyEvent>,
    /// Raw escape sequence overrides for a specific terminal.
    overrides: HashMap<Vec<u8>, KeyEvent>,
}

impl KeyMap {
    /// Build a KeyMap from a loaded config.
    pub fn from_config(config: &BindingConfig) -> Self {
        let mut simple = HashMap::new();
        let mut prefixed = HashMap::new();

        for entry in &config.bindings {
            let event = key_combo_to_event(&entry.combo);
            if entry.combo.after_prefix {
                prefixed.insert(event, entry.action.clone());
            } else {
                simple.insert(event, entry.action.clone());
            }
        }

        let prefix = config.prefix.as_ref().map(key_combo_to_event);

        // Build raw-escape overrides (used when a terminal sends non-standard sequences).
        let mut overrides = HashMap::new();
        for ovs in config.terminal_overrides.values() {
            for ov in ovs {
                let event = key_combo_to_event(&ov.maps_to);
                overrides.insert(ov.raw_escape.as_bytes().to_vec(), event);
            }
        }

        KeyMap {
            simple,
            prefixed,
            prefix,
            overrides,
        }
    }

    /// Look up an action for the given key event.
    ///
    /// If `prefix_active` is true, prefixed bindings are checked first.
    /// Returns `Some((action, consumed_prefix))` on match.
    pub fn lookup(&self, event: &KeyEvent, prefix_active: bool) -> Option<(String, bool)> {
        if prefix_active {
            if let Some(action) = self.prefixed.get(event) {
                return Some((action.clone(), true));
            }
        }
        if let Some(action) = self.simple.get(event) {
            return Some((action.clone(), false));
        }
        None
    }

    /// Try to resolve a raw escape sequence through the overrides table.
    /// Returns the mapped KeyEvent if found.
    pub fn resolve_override(&self, bytes: &[u8]) -> Option<KeyEvent> {
        self.overrides.get(bytes).cloned()
    }

    /// Returns true if the event is the configured prefix key.
    pub fn is_prefix(&self, event: &KeyEvent) -> bool {
        self.prefix.as_ref() == Some(event)
    }

    /// Returns true if a prefix key is configured.
    pub fn has_prefix(&self) -> bool {
        self.prefix.is_some()
    }

    /// Returns the configured prefix key, if any.
    pub fn prefix_key(&self) -> Option<&KeyEvent> {
        self.prefix.as_ref()
    }
}

/// Convert a config `KeyCombo` into a `KeyEvent` for internal use.
fn key_combo_to_event(combo: &KeyCombo) -> KeyEvent {
    KeyEvent {
        key: combo.key.clone(),
        modifiers: combo.modifiers.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BindingEntry, KeyCombo, Modifier};

    fn make_config() -> BindingConfig {
        BindingConfig {
            prefix: Some(KeyCombo {
                key: "b".into(),
                modifiers: vec![Modifier::Ctrl],
                after_prefix: false,
            }),
            bindings: vec![
                BindingEntry {
                    combo: KeyCombo {
                        key: "q".into(),
                        modifiers: vec![],
                        after_prefix: false,
                    },
                    action: "quit".into(),
                },
                BindingEntry {
                    combo: KeyCombo {
                        key: "c".into(),
                        modifiers: vec![],
                        after_prefix: true,
                    },
                    action: "new_pane".into(),
                },
            ],
            terminal_overrides: HashMap::new(),
        }
    }

    #[test]
    fn simple_lookup() {
        let km = KeyMap::from_config(&make_config());
        let result = km.lookup(&KeyEvent::plain("q"), false);
        assert_eq!(result, Some(("quit".to_string(), false)));
    }

    #[test]
    fn prefix_key_detected() {
        let km = KeyMap::from_config(&make_config());
        assert!(km.is_prefix(&KeyEvent::new("b", vec![Modifier::Ctrl])));
        assert!(!km.is_prefix(&KeyEvent::plain("b")));
    }

    #[test]
    fn prefixed_binding() {
        let km = KeyMap::from_config(&make_config());
        let result = km.lookup(&KeyEvent::plain("c"), true);
        assert_eq!(result, Some(("new_pane".to_string(), true)));
    }

    #[test]
    fn prefixed_binding_not_active() {
        let km = KeyMap::from_config(&make_config());
        // Without prefix_active, the prefixed binding should not match.
        let result = km.lookup(&KeyEvent::plain("c"), false);
        assert_eq!(result, None);
    }
}
