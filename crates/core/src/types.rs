// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    Thinking {
        thinking: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::Text(text.into()),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: MessageContent::Text(text.into()),
        }
    }

    pub fn user_blocks(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::Blocks(blocks),
        }
    }

    pub fn assistant_blocks(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::Assistant,
            content: MessageContent::Blocks(blocks),
        }
    }

    pub fn get_text(&self) -> Option<&str> {
        match &self.content {
            MessageContent::Text(s) => Some(s),
            MessageContent::Blocks(blocks) => blocks.iter().find_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            }),
        }
    }

    pub fn get_all_text(&self) -> String {
        match &self.content {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    pub fn content_blocks(&self) -> &[ContentBlock] {
        match &self.content {
            MessageContent::Blocks(blocks) => blocks,
            _ => &[],
        }
    }

    pub fn get_tool_use_blocks(&self) -> Vec<&ContentBlock> {
        self.content_blocks()
            .iter()
            .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
            .collect()
    }

    pub fn has_tool_use(&self) -> bool {
        self.content_blocks()
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageInfo {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Number of input tokens served from the provider's prompt cache.
    /// Providers report this differently (OpenAI: prompt_tokens_details.cached_tokens,
    /// DeepSeek: prompt_cache_hit_tokens). 0 when not reported or not cached.
    #[serde(default)]
    pub cached_tokens: u32,
}

impl UsageInfo {
    pub fn total(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Message constructors ─────────────────────────────────────────

    #[test]
    fn user_constructor() {
        let msg = Message::user("hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.get_text(), Some("hello"));
    }

    #[test]
    fn assistant_constructor() {
        let msg = Message::assistant("hi there");
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.get_text(), Some("hi there"));
    }

    #[test]
    fn user_blocks_constructor() {
        let blocks = vec![ContentBlock::Text { text: "a".into() }];
        let msg = Message::user_blocks(blocks);
        assert_eq!(msg.role, Role::User);
        assert!(matches!(msg.content, MessageContent::Blocks(_)));
    }

    #[test]
    fn assistant_blocks_constructor() {
        let blocks = vec![ContentBlock::Text { text: "b".into() }];
        let msg = Message::assistant_blocks(blocks);
        assert_eq!(msg.role, Role::Assistant);
    }

    // ── get_text() ───────────────────────────────────────────────────

    #[test]
    fn get_text_from_simple_text() {
        let msg = Message::user("hello world");
        assert_eq!(msg.get_text(), Some("hello world"));
    }

    #[test]
    fn get_text_from_blocks_finds_first_text() {
        let msg = Message::user_blocks(vec![
            ContentBlock::ToolUse {
                id: "t1".into(),
                name: "bash".into(),
                input: json!({}),
            },
            ContentBlock::Text { text: "found me".into() },
        ]);
        assert_eq!(msg.get_text(), Some("found me"));
    }

    #[test]
    fn get_text_from_blocks_no_text_returns_none() {
        let msg = Message::user_blocks(vec![ContentBlock::ToolUse {
            id: "t1".into(),
            name: "bash".into(),
            input: json!({}),
        }]);
        assert_eq!(msg.get_text(), None);
    }

    // ── get_all_text() ───────────────────────────────────────────────

    #[test]
    fn get_all_text_from_simple_text() {
        let msg = Message::user("hello");
        assert_eq!(msg.get_all_text(), "hello");
    }

    #[test]
    fn get_all_text_concatenates_all_text_blocks() {
        let msg = Message::assistant_blocks(vec![
            ContentBlock::Text { text: "first ".into() },
            ContentBlock::ToolUse {
                id: "t1".into(),
                name: "bash".into(),
                input: json!({}),
            },
            ContentBlock::Text { text: "second".into() },
        ]);
        assert_eq!(msg.get_all_text(), "first second");
    }

    #[test]
    fn get_all_text_empty_when_no_text_blocks() {
        let msg = Message::assistant_blocks(vec![ContentBlock::ToolUse {
            id: "t1".into(),
            name: "bash".into(),
            input: json!({}),
        }]);
        assert_eq!(msg.get_all_text(), "");
    }

    // ── content_blocks() ─────────────────────────────────────────────

    #[test]
    fn content_blocks_simple_text_returns_empty() {
        let msg = Message::user("hello");
        assert!(msg.content_blocks().is_empty());
    }

    #[test]
    fn content_blocks_returns_slice() {
        let msg = Message::assistant_blocks(vec![
            ContentBlock::Text { text: "a".into() },
            ContentBlock::Text { text: "b".into() },
        ]);
        assert_eq!(msg.content_blocks().len(), 2);
    }

    // ── get_tool_use_blocks() ────────────────────────────────────────

    #[test]
    fn get_tool_use_blocks_filters_correctly() {
        let msg = Message::assistant_blocks(vec![
            ContentBlock::Text { text: "thinking".into() },
            ContentBlock::ToolUse {
                id: "t1".into(),
                name: "bash".into(),
                input: json!({"command": "ls"}),
            },
            ContentBlock::ToolUse {
                id: "t2".into(),
                name: "read".into(),
                input: json!({"path": "/tmp/f"}),
            },
        ]);
        let tool_blocks = msg.get_tool_use_blocks();
        assert_eq!(tool_blocks.len(), 2);
    }

    #[test]
    fn get_tool_use_blocks_empty_for_text_only() {
        let msg = Message::assistant("no tools here");
        assert!(msg.get_tool_use_blocks().is_empty());
    }

    // ── has_tool_use() ───────────────────────────────────────────────

    #[test]
    fn has_tool_use_true() {
        let msg = Message::assistant_blocks(vec![ContentBlock::ToolUse {
            id: "t1".into(),
            name: "bash".into(),
            input: json!({}),
        }]);
        assert!(msg.has_tool_use());
    }

    #[test]
    fn has_tool_use_false_for_text() {
        let msg = Message::assistant("just text");
        assert!(!msg.has_tool_use());
    }

    // ── UsageInfo ────────────────────────────────────────────────────

    #[test]
    fn usage_info_total() {
        let usage = UsageInfo {
            input_tokens: 1000,
            output_tokens: 500,
            cached_tokens: 0,
        };
        assert_eq!(usage.total(), 1500);
    }

    #[test]
    fn usage_info_total_with_cached() {
        let usage = UsageInfo {
            input_tokens: 1000,
            output_tokens: 500,
            cached_tokens: 800,
        };
        // cached_tokens is a subset of input_tokens, not additive
        assert_eq!(usage.total(), 1500);
    }

    #[test]
    fn usage_info_default_is_zero() {
        let usage = UsageInfo::default();
        assert_eq!(usage.total(), 0);
    }
}
