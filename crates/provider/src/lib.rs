// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

pub mod openai;
pub mod types;

use async_trait::async_trait;
use futures::Stream;
use rusty_core::{ModelEntry, ProviderType, Message, RustyError, ToolDefinition, UsageInfo};
use std::pin::Pin;
use std::sync::Arc;

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
    /// Extra HTTP headers to send with every request to this provider.
    /// Used by providers like Kimi that require custom headers for routing.
    pub extra_headers: Option<std::collections::HashMap<String, String>>,
}

impl ProviderConfig {
    /// Build a `ProviderConfig` from a model registry entry and an API key.
    pub fn from_model_entry(entry: &ModelEntry, api_key: String) -> Self {
        Self {
            api_key,
            api_base: entry.api_base.clone(),
            model: entry.model.clone(),
            max_tokens: entry.max_tokens,
            temperature: entry.temperature,
            thinking_budget: entry.thinking_budget,
            extra_headers: entry.extra_headers.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MessageRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub thinking_budget: Option<u32>,
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

/// Create a provider instance for the given provider type and configuration.
///
/// This is the central factory that maps `ProviderType` → concrete `LlmProvider`
/// implementation. Currently all providers speak the OpenAI-compatible protocol.
pub fn create_provider(
    provider_type: ProviderType,
    config: ProviderConfig,
) -> Result<Arc<dyn LlmProvider>, RustyError> {
    match provider_type {
        ProviderType::OpenAI => {
            let provider = OpenAiProvider::new(config)?;
            Ok(Arc::new(provider))
        }
    }
}

/// Convenience: build a provider directly from a model registry entry.
pub fn create_provider_from_entry(
    entry: &ModelEntry,
    api_key: String,
) -> Result<Arc<dyn LlmProvider>, RustyError> {
    let config = ProviderConfig::from_model_entry(entry, api_key);
    create_provider(entry.provider, config)
}
