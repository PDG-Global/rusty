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
        "Switch to plan mode for read-only investigation. Write and execute tools are \
         disabled until the user approves a plan. Call exit_plan_mode with the full plan \
         when ready."
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
             Investigate read-only, then call exit_plan_mode with the complete plan."
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
        "Submit your plan for the user's approval and leave plan mode. Provide the complete \
         plan as markdown with numbered steps. The user approves it, or sends feedback so \
         you can revise it. Nothing is changed until the plan is approved."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "The complete plan in markdown, as numbered steps"
                }
            },
            "required": ["plan"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        // The agent loop intercepts this tool to run the approval flow; this body
        // only runs when the tool is driven outside the loop.
        let plan = input["plan"].as_str().unwrap_or("").trim();
        if plan.is_empty() {
            Ok(ToolResult::error(
                "Provide the full plan in the plan field.".to_string(),
            ))
        } else {
            Ok(ToolResult::success(format!("Plan submitted:\n{plan}")))
        }
    }
}
