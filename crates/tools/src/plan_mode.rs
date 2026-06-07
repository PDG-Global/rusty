// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};

use crate::{Tool, ToolContext, ToolResult};

pub struct EnterPlanModeTool;

#[async_trait]
impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "enter_plan_mode"
    }

    fn description(&self) -> &str {
        "Enter explicit plan mode. In this mode, you can only read files and use todowrite \
         to plan work. Write and execute tools are disabled. Call exit_plan_mode when ready \
         to execute."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        Ok(ToolResult::success(
            "Entered plan mode. Write and execute tools are now disabled. \
             Use todowrite to plan your work, then call exit_plan_mode when ready."
                .to_string(),
        ))
    }
}

pub struct ExitPlanModeTool;

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "exit_plan_mode"
    }

    fn description(&self) -> &str {
        "Exit explicit plan mode and return to normal execution. The user will be asked \
         to review and approve the plan before execution begins."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Optional summary of the plan for user review"
                }
            },
            "required": []
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let summary = input["summary"].as_str().unwrap_or("");
        if summary.is_empty() {
            Ok(ToolResult::success(
                "Exited plan mode. Returning to normal execution.".to_string(),
            ))
        } else {
            Ok(ToolResult::success(format!(
                "Exited plan mode. Plan summary:\n{summary}"
            )))
        }
    }
}
