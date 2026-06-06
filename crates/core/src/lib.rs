// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

pub mod cancel;
pub mod config;
pub mod context;
pub mod cost;
pub mod credentials;
pub mod error;
pub mod history;
pub mod memory;
pub mod permissions;
pub mod plan;
pub mod setup_wizard;
pub mod types;

pub use cancel::*;
pub use config::*;
pub use credentials::*;
pub use error::*;
pub use history::*;
pub use memory::*;
pub use permissions::*;
pub use types::*;

/// User-Agent string for all Rusty HTTP requests.
///
/// Format: `Rusty/{cargo_package_version}`
pub fn rusty_user_agent() -> &'static str {
    concat!("Rusty/", env!("CARGO_PKG_VERSION"))
}
