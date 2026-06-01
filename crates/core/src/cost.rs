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

#[cfg(test)]
mod tests {
    use super::*;

    // ── CostTracker::new ─────────────────────────────────────────────

    #[test]
    fn new_tracker_starts_at_zero() {
        let tracker = CostTracker::new();
        assert_eq!(tracker.total_input(), 0);
        assert_eq!(tracker.total_output(), 0);
    }

    #[test]
    fn default_tracker_starts_at_zero() {
        let tracker = CostTracker::default();
        assert_eq!(tracker.total_input(), 0);
        assert_eq!(tracker.total_output(), 0);
    }

    // ── CostTracker::add_usage ───────────────────────────────────────

    #[test]
    fn add_usage_accumulates() {
        let tracker = CostTracker::new();
        tracker.add_usage(&UsageInfo {
            input_tokens: 100,
            output_tokens: 50,
        });
        assert_eq!(tracker.total_input(), 100);
        assert_eq!(tracker.total_output(), 50);
    }

    #[test]
    fn add_usage_multiple_calls_sum() {
        let tracker = CostTracker::new();
        tracker.add_usage(&UsageInfo {
            input_tokens: 100,
            output_tokens: 50,
        });
        tracker.add_usage(&UsageInfo {
            input_tokens: 200,
            output_tokens: 75,
        });
        assert_eq!(tracker.total_input(), 300);
        assert_eq!(tracker.total_output(), 125);
    }

    #[test]
    fn add_usage_with_zero_tokens() {
        let tracker = CostTracker::new();
        tracker.add_usage(&UsageInfo {
            input_tokens: 0,
            output_tokens: 0,
        });
        assert_eq!(tracker.total_input(), 0);
        assert_eq!(tracker.total_output(), 0);
    }

    // ── CostTracker::summary ─────────────────────────────────────────

    #[test]
    fn summary_zero() {
        let tracker = CostTracker::new();
        assert_eq!(tracker.summary(), "tokens: 0 in / 0 out");
    }

    #[test]
    fn summary_with_usage() {
        let tracker = CostTracker::new();
        tracker.add_usage(&UsageInfo {
            input_tokens: 1234,
            output_tokens: 5678,
        });
        assert_eq!(tracker.summary(), "tokens: 1234 in / 5678 out");
    }
}
