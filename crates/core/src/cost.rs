// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::sync::atomic::{AtomicU64, Ordering};

use crate::UsageInfo;

pub struct CostTracker {
    input_tokens: AtomicU64,
    output_tokens: AtomicU64,
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl CostTracker {
    pub fn new() -> Self {
        Self {
            input_tokens: AtomicU64::new(0),
            output_tokens: AtomicU64::new(0),
        }
    }

    pub fn add_usage(&self, usage: &UsageInfo) {
        self.input_tokens
            .fetch_add(usage.input_tokens as u64, Ordering::Relaxed);
        self.output_tokens
            .fetch_add(usage.output_tokens as u64, Ordering::Relaxed);
    }

    pub fn total_input(&self) -> u64 {
        self.input_tokens.load(Ordering::Relaxed)
    }

    pub fn total_output(&self) -> u64 {
        self.output_tokens.load(Ordering::Relaxed)
    }

    pub fn summary(&self) -> String {
        format!(
            "tokens: {} in / {} out",
            self.total_input(),
            self.total_output()
        )
    }
}
