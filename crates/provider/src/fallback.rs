//! A provider that prefers a cheaper model and falls back to the main one.

use crate::{LlmProvider, MessageRequest, MessageResponse, StreamEvent};
use async_trait::async_trait;
use futures::Stream;
use rusty_core::RustyError;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::warn;

/// Sends requests to `primary` (the fast model) and, if a request fails before
/// streaming starts, retries it on `fallback` (the main model). This covers a
/// fast model that is not available on the user's plan. After the first such
/// failure the primary is skipped for the rest of the session. The request's
/// model name is rewritten to match whichever provider handles it.
pub struct FallbackProvider {
    primary: Arc<dyn LlmProvider>,
    fallback: Arc<dyn LlmProvider>,
    primary_failed: AtomicBool,
}

impl FallbackProvider {
    pub fn new(primary: Arc<dyn LlmProvider>, fallback: Arc<dyn LlmProvider>) -> Self {
        Self {
            primary,
            fallback,
            primary_failed: AtomicBool::new(false),
        }
    }

    fn use_fallback(&self, err: &RustyError) -> bool {
        if matches!(err, RustyError::Cancelled) {
            return false;
        }
        if !self.primary_failed.swap(true, Ordering::Relaxed) {
            warn!(
                "Fast model {} failed ({err}); using {} for background work",
                self.primary.model(),
                self.fallback.model()
            );
        }
        true
    }

    fn for_fallback(&self, mut request: MessageRequest) -> MessageRequest {
        request.model = self.fallback.model().to_string();
        request
    }
}

#[async_trait]
impl LlmProvider for FallbackProvider {
    fn name(&self) -> &str {
        self.active().name()
    }

    fn model(&self) -> &str {
        self.active().model()
    }

    async fn create_message(&self, request: MessageRequest) -> Result<MessageResponse, RustyError> {
        if !self.primary_failed.load(Ordering::Relaxed) {
            match self.primary.create_message(request.clone()).await {
                Ok(r) => return Ok(r),
                Err(e) if !self.use_fallback(&e) => return Err(e),
                Err(_) => {}
            }
        }
        self.fallback.create_message(self.for_fallback(request)).await
    }

    async fn create_message_stream(
        &self,
        request: MessageRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, RustyError>> + Send>>, RustyError> {
        if !self.primary_failed.load(Ordering::Relaxed) {
            match self.primary.create_message_stream(request.clone()).await {
                Ok(s) => return Ok(s),
                Err(e) if !self.use_fallback(&e) => return Err(e),
                Err(_) => {}
            }
        }
        self.fallback.create_message_stream(self.for_fallback(request)).await
    }
}

impl FallbackProvider {
    fn active(&self) -> &dyn LlmProvider {
        if self.primary_failed.load(Ordering::Relaxed) {
            &*self.fallback
        } else {
            &*self.primary
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake {
        model: &'static str,
        fail: bool,
        seen: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl LlmProvider for Fake {
        fn name(&self) -> &str {
            "fake"
        }
        fn model(&self) -> &str {
            self.model
        }
        async fn create_message(&self, request: MessageRequest) -> Result<MessageResponse, RustyError> {
            self.seen.lock().unwrap().push(request.model.clone());
            if self.fail {
                Err(RustyError::Api("model not available".into()))
            } else {
                Err(RustyError::Other("ok".into())) // sentinel: reached this provider
            }
        }
        async fn create_message_stream(
            &self,
            request: MessageRequest,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, RustyError>> + Send>>, RustyError> {
            self.seen.lock().unwrap().push(request.model.clone());
            if self.fail {
                Err(RustyError::Api("model not available".into()))
            } else {
                Ok(Box::pin(futures::stream::empty()))
            }
        }
    }

    fn fake(model: &'static str, fail: bool) -> Arc<Fake> {
        Arc::new(Fake { model, fail, seen: Mutex::new(Vec::new()) })
    }

    fn request(model: &str) -> MessageRequest {
        MessageRequest {
            model: model.to_string(),
            system: None,
            messages: vec![],
            tools: vec![],
            max_tokens: 16,
            temperature: None,
            thinking_budget: None,
        }
    }

    #[tokio::test]
    async fn uses_primary_when_it_works() {
        let (fast, main) = (fake("fast", false), fake("main", false));
        let p = FallbackProvider::new(fast.clone(), main.clone());
        assert!(p.create_message_stream(request("fast")).await.is_ok());
        assert_eq!(*fast.seen.lock().unwrap(), vec!["fast"]);
        assert!(main.seen.lock().unwrap().is_empty());
        assert_eq!(p.model(), "fast");
    }

    #[tokio::test]
    async fn falls_back_with_rewritten_model_and_stays_there() {
        let (fast, main) = (fake("fast", true), fake("main", false));
        let p = FallbackProvider::new(fast.clone(), main.clone());
        assert!(p.create_message_stream(request("fast")).await.is_ok());
        assert!(p.create_message_stream(request("fast")).await.is_ok());
        // Primary tried once only; the fallback got the main model's name both times.
        assert_eq!(*fast.seen.lock().unwrap(), vec!["fast"]);
        assert_eq!(*main.seen.lock().unwrap(), vec!["main", "main"]);
        assert_eq!(p.model(), "main");
    }
}
