---
title: Sub-Agents
description: Spawn independent sub-agents for complex tasks
---

## Overview

The `agent` tool spawns a sub-agent to handle a complex subtask independently. Sub-agents run in their own context with their own message history, making them ideal for delegating research, parallel exploration, or multi-step work.

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `task` | string | Yes | The task description for the sub-agent |
| `context` | string | No | Additional context or constraints |

## How Sub-Agents Work

1. The parent agent invokes the `agent` tool with a task description.
2. A new `Agent` instance is spawned in a separate Tokio task.
3. The sub-agent has its own message history and runs independently.
4. The sub-agent has access to all tools **except** the `agent` tool itself (prevents recursive spawning).
5. Sub-agents run with `BypassPermissions` mode, so they do not prompt for permission.
6. When the sub-agent completes, its final response is returned to the parent agent.

## Use Cases

**Research tasks:** Delegate a research question that requires multiple steps of investigation.

```json
{
  "task": "Find all usages of the `RustyError` enum across the codebase and categorize them by variant.",
  "context": "Focus on error handling patterns in the agent and tools crates."
}
```

**Parallel exploration:** Spawn multiple sub-agents to explore different parts of the codebase simultaneously.

```json
{
  "task": "Explain the streaming implementation in the provider crate.",
  "context": "Look at openai.rs and types.rs. Focus on SSE parsing and event accumulation."
}
```

**Isolated analysis:** Run analysis that should not pollute the main conversation context.

```json
{
  "task": "Analyse the test coverage of the tools crate and identify untested edge cases."
}
```

## Constraints

- Sub-agents cannot spawn further sub-agents (no recursive nesting).
- Sub-agents bypass all permission prompts.
- Sub-agents share the same working directory and file sandbox as the parent.
- Sub-agent results are returned as a single text response; intermediate steps are not visible to the parent.
