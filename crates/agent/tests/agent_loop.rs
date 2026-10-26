// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Integration tests for `Agent::run`, driven by a scripted fake provider.

use async_trait::async_trait;
use futures::Stream;
use rusty_agent::{Agent, AgentCallbacks};
use rusty_core::task::TaskRegistry;
use rusty_core::{
    Config, ContentBlock, Message, PermissionLevel, PermissionMode, Role, RustyError,
};
use rusty_provider::{LlmProvider, MessageRequest, MessageResponse, StreamEvent};
use rusty_tools::{Tool, ToolContext, ToolResult};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------

struct FakeProvider {
    model: String,
    script: Mutex<VecDeque<Vec<StreamEvent>>>,
    requests: Arc<Mutex<Vec<MessageRequest>>>,
}

impl FakeProvider {
    fn new(script: Vec<Vec<StreamEvent>>) -> (Arc<Self>, Arc<Mutex<Vec<MessageRequest>>>) {
        Self::named("fake-model", script)
    }

    /// A fake provider reporting a custom `model()` name.
    fn named(
        model: &str,
        script: Vec<Vec<StreamEvent>>,
    ) -> (Arc<Self>, Arc<Mutex<Vec<MessageRequest>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let p = Arc::new(Self {
            model: model.to_string(),
            script: Mutex::new(script.into()),
            requests: requests.clone(),
        });
        (p, requests)
    }
}

#[async_trait]
impl LlmProvider for FakeProvider {
    fn name(&self) -> &str {
        "fake"
    }
    fn model(&self) -> &str {
        &self.model
    }
    async fn create_message(&self, r: MessageRequest) -> Result<MessageResponse, RustyError> {
        self.requests.lock().unwrap().push(r);
        Ok(MessageResponse {
            content: vec![ContentBlock::Text { text: "run the tests".into() }],
            usage: Default::default(),
            stop_reason: Some("end_turn".into()),
        })
    }
    async fn create_message_stream(
        &self,
        request: MessageRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, RustyError>> + Send>>, RustyError>
    {
        let n = {
            let mut reqs = self.requests.lock().unwrap();
            reqs.push(request);
            reqs.len()
        };
        let events = self.script.lock().unwrap().pop_front().unwrap_or_else(|| {
            panic!("FakeProvider script exhausted: unexpected extra API call #{n}")
        });
        Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
    }
}

struct FakeTool {
    name: &'static str,
    level: PermissionLevel,
    inputs: Arc<Mutex<Vec<Value>>>,
    /// Optional side effect run on execute.
    on_exec: Option<Box<dyn Fn() + Send + Sync>>,
    /// When true, execute returns an error result.
    fail: bool,
}

impl FakeTool {
    fn new(name: &'static str, level: PermissionLevel) -> (Box<dyn Tool>, Arc<Mutex<Vec<Value>>>) {
        let inputs = Arc::new(Mutex::new(Vec::new()));
        (
            Box::new(Self { name, level, inputs: inputs.clone(), on_exec: None, fail: false }),
            inputs,
        )
    }

    fn failing(name: &'static str, level: PermissionLevel) -> Box<dyn Tool> {
        Box::new(Self { name, level, inputs: Arc::default(), on_exec: None, fail: true })
    }
}

#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "fake tool"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn permission_level(&self) -> PermissionLevel {
        self.level
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        self.inputs.lock().unwrap().push(input);
        if let Some(f) = &self.on_exec {
            f();
        }
        if self.fail {
            return Ok(ToolResult::error("boom"));
        }
        Ok(ToolResult::success("ok"))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn text(t: &str, stop: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::TextDelta(t.to_string()),
        StreamEvent::Done { stop_reason: Some(stop.to_string()) },
    ]
}

fn end(t: &str) -> Vec<StreamEvent> {
    text(t, "end_turn")
}

fn call(name: &str, args: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolCallDelta {
            index: 0,
            id: Some("call_1".into()),
            name: Some(name.into()),
            arguments_delta: args.to_string(),
        },
        StreamEvent::Done { stop_reason: Some("tool_calls".into()) },
    ]
}

fn make_agent(provider: Arc<FakeProvider>, tools: Vec<Box<dyn Tool>>) -> Agent {
    let mut agent = Agent::new(
        provider,
        tools,
        Config::default(),
        std::env::temp_dir(),
        "test system prompt".to_string(),
    );
    agent.set_permission_mode(PermissionMode::BypassPermissions);
    agent
}

async fn run(agent: &mut Agent) -> Result<String, RustyError> {
    agent
        .run(vec![ContentBlock::Text { text: "go".into() }], AgentCallbacks::default())
        .await
}

fn user_texts(req: &MessageRequest) -> Vec<String> {
    req.messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(Message::get_all_text)
        .collect()
}

fn count_user_msgs_containing(req: &MessageRequest, needle: &str) -> usize {
    user_texts(req).iter().filter(|t| t.contains(needle)).count()
}

fn tool_results(req: &MessageRequest) -> Vec<(String, Option<bool>)> {
    req.messages
        .iter()
        .flat_map(|m| m.content_blocks().to_vec())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { content, is_error, .. } => Some((content, is_error)),
            _ => None,
        })
        .collect()
}

const NUDGE: &str = "continue now";

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

#[tokio::test]
async fn s1_plain_text_answer_ends_after_one_call() {
    let (p, reqs) = FakeProvider::new(vec![end("Hello there.")]);
    let mut agent = make_agent(p, vec![]);
    assert_eq!(run(&mut agent).await.unwrap(), "Hello there.");
    assert_eq!(reqs.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn s2_write_tool_then_text_gets_nudged() {
    let (p, reqs) = FakeProvider::new(vec![
        call("fake_write", "{\"path\":\"a\"}"),
        end("Done."),
        end("Confirmed complete."),
    ]);
    let (w, inputs) = FakeTool::new("fake_write", PermissionLevel::Write);
    let mut agent = make_agent(p, vec![w]);
    assert_eq!(run(&mut agent).await.unwrap(), "Confirmed complete.");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 3);
    assert_eq!(inputs.lock().unwrap().as_slice(), &[json!({"path": "a"})]);
    assert_eq!(count_user_msgs_containing(&reqs[1], NUDGE), 0);
    assert_eq!(count_user_msgs_containing(&reqs[2], NUDGE), 1);
}

#[tokio::test]
async fn s3_readonly_tool_then_announced_work_nudges() {
    let (p, reqs) = FakeProvider::new(vec![
        call("fake_read", "{}"),
        end("Now let me update the tests:"),
        end("All done."),
    ]);
    let (r, _) = FakeTool::new("fake_read", PermissionLevel::ReadOnly);
    let mut agent = make_agent(p, vec![r]);
    assert_eq!(run(&mut agent).await.unwrap(), "All done.");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 3);
    assert_eq!(count_user_msgs_containing(&reqs[2], NUDGE), 1);
}

#[tokio::test]
async fn s4_readonly_tool_then_completion_text_no_nudge() {
    let (p, reqs) = FakeProvider::new(vec![call("fake_read", "{}"), end("All changes are complete.")]);
    let (r, _) = FakeTool::new("fake_read", PermissionLevel::ReadOnly);
    let mut agent = make_agent(p, vec![r]);
    assert_eq!(run(&mut agent).await.unwrap(), "All changes are complete.");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    assert_eq!(count_user_msgs_containing(&reqs[1], NUDGE), 0);
}

#[tokio::test]
async fn s5_nudge_cap_is_three() {
    // Each cycle: write tool call, then a stop with text. Cap = 3 nudges, so the
    // 4th stop terminates: 4 cycles = 8 calls.
    let mut script = Vec::new();
    for i in 0..4 {
        script.push(call("fake_write", &format!("{{\"n\":{i}}}")));
        script.push(end(&format!("stopped {i}")));
    }
    let (p, reqs) = FakeProvider::new(script);
    let (w, inputs) = FakeTool::new("fake_write", PermissionLevel::Write);
    let mut agent = make_agent(p, vec![w]);
    assert_eq!(run(&mut agent).await.unwrap(), "stopped 3");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 8);
    assert_eq!(inputs.lock().unwrap().len(), 4);
    let last = reqs.last().unwrap();
    assert_eq!(count_user_msgs_containing(last, NUDGE), 3);
}

#[tokio::test]
async fn s6_empty_response_retried_once_then_text() {
    let (p, reqs) = FakeProvider::new(vec![
        vec![StreamEvent::Done { stop_reason: Some("end_turn".into()) }],
        end("Recovered."),
    ]);
    let mut agent = make_agent(p, vec![]);
    assert_eq!(run(&mut agent).await.unwrap(), "Recovered.");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    assert_eq!(count_user_msgs_containing(&reqs[0], "Your last response was empty"), 0);
    assert_eq!(count_user_msgs_containing(&reqs[1], "Your last response was empty"), 1);
}

#[tokio::test]
async fn s6b_empty_response_cap() {
    let empty = || vec![StreamEvent::Done { stop_reason: Some("end_turn".into()) }];
    let (p, reqs) = FakeProvider::new(vec![empty(), empty(), empty()]);
    let mut agent = make_agent(p, vec![]);
    assert_eq!(run(&mut agent).await.unwrap(), "");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 3); // 1 + MAX_EMPTY_RESPONSE_RETRIES
    assert_eq!(count_user_msgs_containing(&reqs[2], "Your last response was empty"), 2);
}

#[tokio::test]
async fn s7_max_tokens_truncation_continues() {
    let (p, reqs) = FakeProvider::new(vec![text("Hello wor", "max_tokens"), end("ld.")]);
    let mut agent = make_agent(p, vec![]);
    assert_eq!(run(&mut agent).await.unwrap(), "Hello world.");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    assert_eq!(count_user_msgs_containing(&reqs[1], "cut off by the output token limit"), 1);
}

#[tokio::test]
async fn s8_malformed_args_not_executed() {
    let (p, reqs) = FakeProvider::new(vec![call("fake_read", "{\"path\": "), end("Finished.")]);
    let (r, inputs) = FakeTool::new("fake_read", PermissionLevel::ReadOnly);
    let mut agent = make_agent(p, vec![r]);
    assert_eq!(run(&mut agent).await.unwrap(), "Finished.");
    assert!(inputs.lock().unwrap().is_empty(), "tool must not run on invalid JSON");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    let results = tool_results(&reqs[1]);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].1, Some(true));
    assert!(results[0].0.contains("Invalid JSON arguments"), "got: {}", results[0].0);
}

#[tokio::test]
async fn s8b_empty_args_execute_with_empty_object() {
    let (p, reqs) = FakeProvider::new(vec![call("fake_read", ""), end("Finished.")]);
    let (r, inputs) = FakeTool::new("fake_read", PermissionLevel::ReadOnly);
    let mut agent = make_agent(p, vec![r]);
    assert_eq!(run(&mut agent).await.unwrap(), "Finished.");
    assert_eq!(inputs.lock().unwrap().as_slice(), &[json!({})]);
    assert_eq!(reqs.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn s9_question_after_write_is_not_nudged() {
    let (p, reqs) = FakeProvider::new(vec![call("fake_write", "{}"), end("I made the edit. Should I proceed?")]);
    let (w, _) = FakeTool::new("fake_write", PermissionLevel::Write);
    let mut agent = make_agent(p, vec![w]);
    assert_eq!(run(&mut agent).await.unwrap(), "I made the edit. Should I proceed?");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    assert_eq!(count_user_msgs_containing(&reqs[1], NUDGE), 0);
}

#[tokio::test]
async fn s10_stream_error_returns_err() {
    let (p, reqs) = FakeProvider::new(vec![vec![
        StreamEvent::TextDelta("partial".into()),
        StreamEvent::Error("boom".into()),
    ]]);
    let mut agent = make_agent(p, vec![]);
    let err = run(&mut agent).await.unwrap_err();
    assert!(err.to_string().contains("boom"), "got: {err}");
    assert_eq!(reqs.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn s11_tool_use_stop_reason_without_calls_behaves_like_end_turn() {
    // Consistent with scenario 2: write, then stop(tool_use) text -> nudge.
    let (p, reqs) = FakeProvider::new(vec![
        call("fake_write", "{}"),
        text("Done.", "tool_use"),
        text("Confirmed complete.", "tool_use"),
    ]);
    let (w, _) = FakeTool::new("fake_write", PermissionLevel::Write);
    let mut agent = make_agent(p, vec![w]);
    assert_eq!(run(&mut agent).await.unwrap(), "Confirmed complete.");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 3);
    assert_eq!(count_user_msgs_containing(&reqs[2], NUDGE), 1);
}

#[tokio::test]
async fn s11b_tool_use_stop_reason_completion_text_no_nudge() {
    // Consistent with scenario 4.
    let (p, reqs) = FakeProvider::new(vec![call("fake_read", "{}"), text("All changes are complete.", "tool_use")]);
    let (r, _) = FakeTool::new("fake_read", PermissionLevel::ReadOnly);
    let mut agent = make_agent(p, vec![r]);
    assert_eq!(run(&mut agent).await.unwrap(), "All changes are complete.");
    assert_eq!(reqs.lock().unwrap().len(), 2);
}

fn registry_agent(
    script: Vec<Vec<StreamEvent>>,
    tools: Vec<Box<dyn Tool>>,
    reg: Arc<TaskRegistry>,
) -> (Agent, Arc<Mutex<Vec<MessageRequest>>>) {
    let (p, reqs) = FakeProvider::new(script);
    let mut agent = make_agent(p, tools);
    agent.set_task_registry(reg, "s1".to_string());
    (agent, reqs)
}

#[tokio::test]
async fn s12_all_tasks_done_suppresses_nudge() {
    let dir = tempfile::TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::open(&dir.path().join("t.db")).unwrap());
    reg.create("s1", "only task", None, None).unwrap();
    reg.done("s1", "T1", None).unwrap();
    let (w, _) = FakeTool::new("fake_write", PermissionLevel::Write);
    let (mut agent, reqs) = registry_agent(vec![call("fake_write", "{}"), end("Done.")], vec![w], reg);
    assert_eq!(run(&mut agent).await.unwrap(), "Done.");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    assert_eq!(count_user_msgs_containing(&reqs[1], NUDGE), 0);
}

#[tokio::test]
async fn s12b_incomplete_task_reenters_until_completed() {
    let dir = tempfile::TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::open(&dir.path().join("t.db")).unwrap());
    reg.create("s1", "finish the thing", None, None).unwrap();
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let reg2 = reg.clone();
    let finisher: Box<dyn Tool> = Box::new(FakeTool {
        name: "finish_task",
        level: PermissionLevel::ReadOnly,
        inputs,
        fail: false,
        on_exec: Some(Box::new(move || {
            reg2.done("s1", "T1", None).unwrap();
        })),
    });
    let (mut agent, reqs) = registry_agent(
        vec![end("Working on it."), call("finish_task", "{}"), end("All finished.")],
        vec![finisher],
        reg,
    );
    assert_eq!(run(&mut agent).await.unwrap(), "All finished.");
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 3);
    assert_eq!(count_user_msgs_containing(&reqs[0], "You are about to finish"), 0);
    assert_eq!(count_user_msgs_containing(&reqs[1], "You are about to finish"), 1);
    assert_eq!(count_user_msgs_containing(&reqs[1], "finish the thing"), 1);
}

#[tokio::test]
async fn s12c_task_gate_cap() {
    let dir = tempfile::TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::open(&dir.path().join("t.db")).unwrap());
    reg.create("s1", "never done", None, None).unwrap();
    // MAX_TASK_GATE_REENTRIES = 6, so the 7th stop is allowed.
    let script = (0..7).map(|i| end(&format!("stop {i}"))).collect();
    let (mut agent, reqs) = registry_agent(script, vec![], reg);
    assert_eq!(run(&mut agent).await.unwrap(), "stop 6");
    assert_eq!(reqs.lock().unwrap().len(), 7);
}

// ---------------------------------------------------------------------------
// Plan mode: never left implicitly, so the user can review the plan.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn p1_plan_mode_blocks_writes_and_stops_after_two_blocked_turns() {
    let (p, reqs) = FakeProvider::new(vec![
        call("fake_write", "{\"path\":\"a\"}"),
        call("fake_write", "{\"path\":\"b\"}"),
    ]);
    let (w, inputs) = FakeTool::new("fake_write", PermissionLevel::Write);
    let mut agent = make_agent(p, vec![w]);
    agent.enter_plan_mode();
    let out = run(&mut agent).await.unwrap();
    assert!(out.starts_with("Stopped: still in plan mode"), "got {out:?}");
    assert_eq!(reqs.lock().unwrap().len(), 2);
    assert!(inputs.lock().unwrap().is_empty(), "write tool ran in plan mode");
    assert!(agent.plan_mode, "plan mode was left without exit_plan_mode");
}

#[tokio::test]
async fn p2_plan_mode_stays_on_after_a_single_blocked_write() {
    let (p, reqs) = FakeProvider::new(vec![
        call("fake_write", "{\"path\":\"a\"}"),
        end("Here is the plan."),
    ]);
    let (w, inputs) = FakeTool::new("fake_write", PermissionLevel::Write);
    let mut agent = make_agent(p, vec![w]);
    agent.enter_plan_mode();
    assert_eq!(run(&mut agent).await.unwrap(), "Here is the plan.");
    assert_eq!(reqs.lock().unwrap().len(), 2);
    assert!(inputs.lock().unwrap().is_empty());
    assert!(agent.plan_mode);
}

#[tokio::test]
async fn p3_non_consecutive_plan_blocks_do_not_stop_the_run() {
    let (p, reqs) = FakeProvider::new(vec![
        call("fake_write", "{\"path\":\"a\"}"),
        call("fake_read", "{\"path\":\"a\"}"),
        call("fake_write", "{\"path\":\"b\"}"),
        end("Here is the plan."),
    ]);
    let (w, w_inputs) = FakeTool::new("fake_write", PermissionLevel::Write);
    let (r, r_inputs) = FakeTool::new("fake_read", PermissionLevel::ReadOnly);
    let mut agent = make_agent(p, vec![w, r]);
    agent.enter_plan_mode();
    assert_eq!(run(&mut agent).await.unwrap(), "Here is the plan.");
    assert_eq!(reqs.lock().unwrap().len(), 4);
    assert!(w_inputs.lock().unwrap().is_empty());
    assert_eq!(r_inputs.lock().unwrap().len(), 1);
    assert!(agent.plan_mode);
}

// ---------------------------------------------------------------------------
// Fast-model routing and per-turn thinking budgets
// ---------------------------------------------------------------------------

const FAST: &str = "fast-model-x";

fn seed_history(agent: &mut Agent) {
    for i in 0..30 {
        let m = if i % 2 == 0 {
            Message::user(format!("question {i}"))
        } else {
            Message::assistant(format!("answer {i}"))
        };
        agent.messages_mut().push(m);
    }
}

#[tokio::test]
async fn f1_forced_compaction_goes_to_fast_provider_with_fast_model_name() {
    let (main, main_reqs) = FakeProvider::new(vec![]);
    let (fast, fast_reqs) = FakeProvider::named(FAST, vec![end("summary of the work")]);
    let mut agent = make_agent(main, vec![]);
    agent.set_fast_provider(Some(fast));
    seed_history(&mut agent);
    assert!(agent.compact().await.unwrap());
    assert!(main_reqs.lock().unwrap().is_empty());
    let fast_reqs = fast_reqs.lock().unwrap();
    assert_eq!(fast_reqs.len(), 1);
    assert_eq!(fast_reqs[0].model, FAST);
    assert_eq!(fast_reqs[0].thinking_budget, None);
}

#[tokio::test]
async fn f2_suggestion_goes_to_fast_provider_with_fast_model_name() {
    let (main, main_reqs) = FakeProvider::new(vec![end("Done.")]);
    let (fast, fast_reqs) = FakeProvider::named(FAST, vec![]);
    let mut agent = make_agent(main, vec![]);
    agent.set_fast_provider(Some(fast));
    run(&mut agent).await.unwrap();
    assert_eq!(main_reqs.lock().unwrap().len(), 1);
    let suggestion = agent.generate_followup_suggestion().await;
    assert_eq!(suggestion.as_deref(), Some("run the tests"));
    // Main provider saw only the main turn.
    assert_eq!(main_reqs.lock().unwrap().len(), 1);
    let fast_reqs = fast_reqs.lock().unwrap();
    assert_eq!(fast_reqs.len(), 1);
    assert_eq!(fast_reqs[0].model, FAST);
    assert_eq!(fast_reqs[0].thinking_budget, None);
}

#[tokio::test]
async fn f3_main_turns_stay_on_main_provider_and_model() {
    let (main, main_reqs) = FakeProvider::new(vec![call("fake_read", "{}"), end("All changes are complete.")]);
    let (fast, fast_reqs) = FakeProvider::named(FAST, vec![]);
    let (r, _) = FakeTool::new("fake_read", PermissionLevel::ReadOnly);
    let mut agent = make_agent(main, vec![r]);
    agent.set_fast_provider(Some(fast));
    run(&mut agent).await.unwrap();
    let main_reqs = main_reqs.lock().unwrap();
    assert_eq!(main_reqs.len(), 2);
    assert!(main_reqs.iter().all(|r| r.model == Config::default().model));
    assert!(fast_reqs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn f4_without_fast_provider_background_calls_fall_back_to_main() {
    let (main, main_reqs) = FakeProvider::new(vec![end("Done."), end("summary of the work")]);
    let mut agent = make_agent(main, vec![]);
    run(&mut agent).await.unwrap();
    assert!(agent.generate_followup_suggestion().await.is_some());
    seed_history(&mut agent);
    assert!(agent.compact().await.unwrap());
    let reqs = main_reqs.lock().unwrap();
    assert_eq!(reqs.len(), 3);
    assert_eq!(reqs[1].model, "fake-model");
    assert_eq!(reqs[2].model, "fake-model");
}

#[tokio::test]
async fn t1_thinking_budget_full_first_turn_minimal_after_success() {
    let (p, reqs) = FakeProvider::new(vec![call("fake_read", "{}"), end("All changes are complete.")]);
    let (r, _) = FakeTool::new("fake_read", PermissionLevel::ReadOnly);
    let mut agent = make_agent(p, vec![r]);
    agent.config_mut().thinking_budget = Some(8192);
    run(&mut agent).await.unwrap();
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs[0].thinking_budget, Some(8192));
    assert_eq!(reqs[1].thinking_budget, Some(1024));
    assert!(reqs[1].max_tokens > 1024);
}

#[tokio::test]
async fn t2_thinking_budget_full_after_error_tool_result() {
    let (p, reqs) = FakeProvider::new(vec![call("fake_fail", "{}"), end("All changes are complete.")]);
    let mut agent = make_agent(p, vec![FakeTool::failing("fake_fail", PermissionLevel::ReadOnly)]);
    agent.config_mut().thinking_budget = Some(8192);
    run(&mut agent).await.unwrap();
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs[0].thinking_budget, Some(8192));
    assert_eq!(reqs[1].thinking_budget, Some(8192));
}

#[tokio::test]
async fn t3_thinking_budget_full_in_plan_mode_and_none_when_unconfigured() {
    let (p, reqs) = FakeProvider::new(vec![call("fake_read", "{}"), end("Here is the plan.")]);
    let (r, _) = FakeTool::new("fake_read", PermissionLevel::ReadOnly);
    let mut agent = make_agent(p, vec![r]);
    agent.config_mut().thinking_budget = Some(8192);
    agent.enter_plan_mode();
    run(&mut agent).await.unwrap();
    assert!(reqs.lock().unwrap().iter().all(|r| r.thinking_budget == Some(8192)));

    let (p, reqs) = FakeProvider::new(vec![call("fake_read", "{}"), end("All changes are complete.")]);
    let (r, _) = FakeTool::new("fake_read", PermissionLevel::ReadOnly);
    let mut agent = make_agent(p, vec![r]);
    run(&mut agent).await.unwrap();
    assert!(reqs.lock().unwrap().iter().all(|r| r.thinking_budget.is_none()));
}

// ---------------------------------------------------------------------------
// Read-only calls of write-level tools don't count as writes.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e1_explore_agent_call_does_not_trigger_write_nudge() {
    let (p, reqs) = FakeProvider::new(vec![
        call("agent", "{\"task\":\"look around\",\"subagent_type\":\"explore\"}"),
        end("Here is the summary."),
    ]);
    let (a, _inputs) = FakeTool::new("agent", PermissionLevel::Write);
    let mut agent = make_agent(p, vec![a]);
    assert_eq!(run(&mut agent).await.unwrap(), "Here is the summary.");
    assert_eq!(reqs.lock().unwrap().len(), 2, "explore sub-agent call triggered a nudge");
}

#[tokio::test]
async fn e2_coder_agent_call_still_counts_as_write() {
    let (p, reqs) = FakeProvider::new(vec![
        call("agent", "{\"task\":\"edit\",\"subagent_type\":\"coder\"}"),
        end("Done."),
        end("Confirmed complete."),
    ]);
    let (a, _inputs) = FakeTool::new("agent", PermissionLevel::Write);
    let mut agent = make_agent(p, vec![a]);
    assert_eq!(run(&mut agent).await.unwrap(), "Confirmed complete.");
    assert_eq!(reqs.lock().unwrap().len(), 3);
}
