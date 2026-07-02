---
title: Agent Loop
description: How the Rusty agent loop works internally
---

## Overview

The agent loop is the core orchestrator in Rusty. It manages the conversation between the user, the LLM, and the tools.

## Loop Flow

1.  **User message** -- The user's input is added to the message history.

2.  **Auto-compaction check** -- If context usage is high, older messages are managed via a three-tier compaction system (see below).

3.  **Send to LLM** -- The full message history, system prompt, and tool definitions are sent to the LLM provider via SSE streaming.

4.  **Stream response** -- The response is streamed back event by event: text deltas, thinking deltas, and tool call deltas are accumulated as they arrive.

5.  **Process tool calls** -- If the response contains tool calls, all calls are executed concurrently via a `JoinSet`. Each tool call runs as an independent spawned task, with results collected and returned in call-order once all complete.

6.  **Loop or complete** -- If tool calls were present, the loop returns to step 2 with the new messages. If no tool calls were present, the response text is returned as the final answer.

## Auto-Compaction

Long conversations are managed via a three-tier compaction system to stay within the context window:

| Tier | Context Threshold | Action |
|------|------------------|--------|
| Tier 1 | 25% | Micro-compact: replace old tool results with short placeholders |
| Tier 2 | 50% | Extract structured checkpoint to `checkpoint.md` via an LLM call |
| Tier 3 | 75% | Summarise old messages via an LLM call, keeping the last 10 messages |

Each tier triggers at most once per conversation. The notes scratchpad content (from the `note` tool) is incorporated into checkpoints and summaries, then cleared.

Use `/compact` to trigger compaction manually.

## Concurrent Tool Execution

When the LLM issues multiple tool calls in a single response, they are executed concurrently using `tokio::JoinSet`. This means:

- Independent tools (e.g. two `file_read` calls) run in parallel.
- Results are collected once all tools complete.
- Wall-clock time is significantly reduced compared to sequential execution.

## Cancellation

The agent loop can be cancelled at any time via `Ctrl+C` in TUI mode. Cancellation sets an atomic flag that the loop checks between steps. When cancelled:

- The current LLM stream is terminated
- Partial text is discarded
- The session state is preserved (no messages are lost)

## Turn Limits

The `--max-turns` flag limits the number of agent loop iterations. Each iteration where tool calls are executed counts as one turn. This prevents runaway loops where the agent keeps calling tools indefinitely.

```bash
rusty --max-turns 10 --prompt "Analyse this codebase"
```

## System Prompt

The system prompt is assembled from several sources:

- **Tool descriptions**: Names, descriptions, and input schemas for all available tools
- **Permission mode**: Current permission mode and allowed tools
- **Platform info**: Operating system, architecture, working directory
- **Git status**: Current branch, recent commits, working tree status
- **Sandbox notice**: Reminder that file operations are restricted to the working directory
- **Context files**: Contents of AGENTS.md, CLAUDE.md, or RUSTY.md files found in the project
- **Current date**: For time-sensitive operations
- **Plan-with-tasks instructions**: Optional instructions for structured task tracking

## Streaming

All LLM communication is streaming-first. The provider yields a stream of events:

| Event | Description |
|-------|-------------|
| `TextDelta` | A chunk of response text |
| `ThinkingDelta` | A chunk of reasoning/thinking content |
| `ToolCallDelta` | A chunk of a tool call (name or arguments) |
| `Usage` | Token usage information |
| `Done` | Stream completed |
| `Error` | An error occurred |

Events are consumed one by one by the agent loop and forwarded to the UI via callbacks.

## Callbacks

The agent accepts optional callbacks for real-time UI updates:

| Callback | Purpose |
|----------|---------|
| `TextCallback` | Called with each text delta |
| `ThinkingCallback` | Called with each thinking delta |
| `ToolCallback` | Called when a tool call starts or completes |
| `PermissionCallback` | Called when a tool requires permission |

These callbacks are what drive the TUI streaming display and the permission prompt overlay.

## Sub-Agents

Sub-agents are spawned as independent Tokio tasks with their own agent loop instance. They run with `BypassPermissions` and have access to all tools except the `agent` tool (to prevent recursive spawning). The parent agent receives the sub-agent's final response as a single text block.
