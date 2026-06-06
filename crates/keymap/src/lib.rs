// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Terminal keymapping library.
//!
//! Provides JSON-configurable keybindings with support for modifier keys,
//! multi-key sequences (prefix keys), and terminal-specific escape overrides.
//!
//! # Architecture
//!
//! 1. **Config loading** - `BindingConfig` is deserialized from JSON.
//! 2. **Key parsing** - raw terminal bytes are parsed into `KeyEvent` structs.
//! 3. **Binding lookup** - `KeyMap` resolves a `KeyEvent` (or sequence) to an action name.
//! 4. **Dispatch** - `Dispatcher` invokes the registered handler for that action.
//!
//! The caller owns the read loop and is responsible for putting the terminal in
//! raw mode and reading bytes from stdin.

pub mod dispatcher;
pub mod keymap;
pub mod parse;
pub mod types;

// Re-export the main public API at crate root for convenience.
pub use dispatcher::Dispatcher;
pub use keymap::KeyMap;
pub use parse::parse_key_bytes;
pub use types::{BindingConfig, BindingEntry, KeyEvent, KeyCombo, Modifier};
