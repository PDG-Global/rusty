// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Action dispatcher: registers named handlers and invokes them by action name.

use std::collections::HashMap;

/// Signature for action handler functions.
pub type ActionFn = Box<dyn FnMut(&str)>;

/// Registry of action names to handler functions.
///
/// Handlers are `FnMut` so they can capture and mutate state (e.g. UI model).
/// An optional `default_action` is invoked for unrecognised action names.
pub struct Dispatcher {
    handlers: HashMap<String, ActionFn>,
    default_action: Option<ActionFn>,
}

impl Dispatcher {
    pub fn new() -> Self {
        Dispatcher {
            handlers: HashMap::new(),
            default_action: None,
        }
    }

    /// Register a handler for the given action name.
    pub fn register(&mut self, action: impl Into<String>, handler: ActionFn) {
        self.handlers.insert(action.into(), handler);
    }

    /// Set a default handler invoked when no specific handler matches.
    pub fn set_default_action(&mut self, handler: ActionFn) {
        self.default_action = Some(handler);
    }

    /// Dispatch to the handler registered for `action`.
    /// Falls back to `default_action` if no specific handler is found.
    /// Returns `true` if any handler was called, `false` otherwise.
    pub fn dispatch(&mut self, action: &str) -> bool {
        if let Some(handler) = self.handlers.get_mut(action) {
            handler(action);
            true
        } else if let Some(ref mut fallback) = self.default_action {
            fallback(action);
            true
        } else {
            false
        }
    }

    /// Returns true if a handler is registered for the given action.
    pub fn has_action(&self, action: &str) -> bool {
        self.handlers.contains_key(action)
    }

    /// Returns the number of registered actions.
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn dispatch_calls_handler() {
        let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let log_clone = log.clone();

        let mut d = Dispatcher::new();
        d.register(
            "test",
            Box::new(move |action: &str| {
                log_clone.borrow_mut().push(action.to_string());
            }),
        );

        assert!(d.dispatch("test"));
        assert_eq!(&*log.borrow(), &["test".to_string()]);
    }

    #[test]
    fn dispatch_missing_returns_false() {
        let mut d = Dispatcher::new();
        assert!(!d.dispatch("nope"));
    }

    #[test]
    fn default_action_fires_for_unknown() {
        let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let log_clone = log.clone();

        let mut d = Dispatcher::new();
        d.set_default_action(Box::new(move |action: &str| {
            log_clone.borrow_mut().push(action.to_string());
        }));

        // No specific handler registered, so default_action fires.
        assert!(d.dispatch("unknown-action"));
        assert_eq!(&*log.borrow(), &["unknown-action".to_string()]);
    }

    #[test]
    fn default_action_does_not_fire_when_specific_exists() {
        let default_log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let specific_log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let dl = default_log.clone();
        let sl = specific_log.clone();

        let mut d = Dispatcher::new();
        d.set_default_action(Box::new(move |action: &str| {
            dl.borrow_mut().push(action.to_string());
        }));
        d.register(
            "quit",
            Box::new(move |action: &str| {
                sl.borrow_mut().push(action.to_string());
            }),
        );

        assert!(d.dispatch("quit"));
        assert_eq!(&*specific_log.borrow(), &["quit".to_string()]);
        assert!(default_log.borrow().is_empty());
    }

    #[test]
    fn default_action_still_returns_false_without_either() {
        let mut d = Dispatcher::new();
        assert!(!d.dispatch("nothing"));
    }

    #[test]
    fn has_action() {
        let mut d = Dispatcher::new();
        d.register("quit", Box::new(|_| {}));
        assert!(d.has_action("quit"));
        assert!(!d.has_action("other"));
    }
}
