// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

pub mod openai;
pub mod types;

use async_trait::async_trait;
use futures::Stream;
use rusty_core::{Message, RustyError, ToolDefinition, UsageInfo};
use std::pin::Pin;

pub use openai::OpenAiProvider;
pub use types::*;

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub api_key: String,
    pub api_base: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub thinking_budget: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct MessageRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct MessageResponse {
    pub content: Vec<rusty_core::ContentBlock>,
    pub usage: UsageInfo,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    Usage(UsageInfo),
    Done {
        stop_reason: Option<String>,
    },
    Error(String),
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    fn model(&self) -> &str;

    async fn create_message(&self, request: MessageRequest) -> Result<MessageResponse, RustyError>;

    async fn create_message_stream(
        &self,
        request: MessageRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, RustyError>> + Send>>, RustyError>;
}
