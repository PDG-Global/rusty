---
title: 任务管理
description: 用 todowrite 进行结构化任务跟踪
---


## 概述

`todowrite` 工具在对话中提供结构化的任务列表管理。它让 LLM 能够规划、跟踪并报告多步工作的进展。

## 参数

该工具接受一个任务项数组：

| 字段 | 类型 | 必填 | 说明 |
|-------|------|----------|-------------|
| `content` | string | 是 | 任务描述 |
| `status` | enum | 否 | `pending`、`in_progress`、`completed` 或 `cancelled` |
| `priority` | enum | 否 | `high`、`medium` 或 `low` |

## 任务状态

| 状态 | 含义 |
|--------|---------|
| `pending` | 尚未开始 |
| `in_progress` | 正在进行 |
| `completed` | 已成功完成 |
| `cancelled` | 已跳过或受阻（附原因） |

## 工作原理

任务列表在整个对话中持续存在。每次调用 `todowrite` 都会替换整个列表，因此必须始终传入当前的完整状态。任务会按优先级分组渲染，并带有状态指示。

## 工作流

多步任务的推荐工作流：

1.  **规划**——创建一个包含具体、可执行条目的任务列表。每个任务都应具体（例如“在 Z.rs 的 Y 结构体中添加 X 字段”），而非含糊（例如“改进错误处理”）。

2.  **执行**——依次处理任务。开始前将任务标记为 `in_progress`，完成后标记为 `completed`。

3.  **跟踪**——若执行中发现新工作，将其加入列表。若任务受阻，标记为 `cancelled` 并附上原因。

4.  **核验**——所有任务完成后，对照原始请求审阅成果。

## 示例

执行过程中一个典型的任务列表：

```json
[
  {
    "content": "在 error.rs 中为网络超时添加错误变体",
    "status": "completed",
    "priority": "high"
  },
  {
    "content": "更新 provider 以映射超时错误",
    "status": "in_progress",
    "priority": "high"
  },
  {
    "content": "添加带指数退避的重试逻辑",
    "status": "pending",
    "priority": "medium"
  },
  {
    "content": "为超时处理编写测试",
    "status": "pending",
    "priority": "medium"
  }
]
```

## 最佳实践

- **保持列表简短。** 3–7 个任务较为典型。较大的工作拆分成阶段。
- **具体明确。** 每个任务都应描述一个具体动作，而非一个目标。
- **立即更新。** 边做边把任务标记为 in_progress/completed，不要批量更新。
- **补充发现的工作。** 执行中发现新任务就加进去。
- **切勿提前停止。** 持续进行，直到所有任务都完成或取消。
