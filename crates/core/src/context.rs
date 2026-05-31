// Copyright (C) 2025 Jeremy Moseley
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::Path;

/// Build system context: platform info, working directory, git status
pub async fn build_system_context(working_dir: &Path) -> String {
    let mut parts = Vec::new();

    parts.push(format!(
        "Platform: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    parts.push(format!("Working directory: {}", working_dir.display()));
    parts.push(format!(
        "SANDBOX: All file operations are restricted to {} and its subdirectories. \
         Accessing files outside this directory will be denied. \
         Do not attempt to read or write files above the working directory.",
        working_dir.display()
    ));

    // Git status
    if let Ok(output) = tokio::process::Command::new("git")
        .args(["status", "--short"])
        .current_dir(working_dir)
        .output()
        .await
    {
        if output.status.success() {
            let status = String::from_utf8_lossy(&output.stdout);
            if !status.trim().is_empty() {
                parts.push(format!("Git status:\n{}", status.trim()));
            } else {
                parts.push("Git status: (clean)".to_string());
            }
        }
    }

    // Recent commits
    if let Ok(output) = tokio::process::Command::new("git")
        .args(["log", "--oneline", "-5"])
        .current_dir(working_dir)
        .output()
        .await
    {
        if output.status.success() {
            let log = String::from_utf8_lossy(&output.stdout);
            if !log.trim().is_empty() {
                parts.push(format!("Recent commits:\n{}", log.trim()));
            }
        }
    }

    parts.join("\n")
}

/// Build user context: date, CLAUDE.md / RUSTY.md files
pub async fn build_user_context(working_dir: &Path, no_claude_md: bool) -> String {
    let mut parts = Vec::new();

    parts.push(format!(
        "Today's date is {}.",
        chrono::Local::now().format("%Y-%m-%d")
    ));

    if !no_claude_md {
        let md_files = discover_md_files(working_dir).await;
        if !md_files.is_empty() {
            parts.push(md_files);
        }
    }

    parts.join("\n\n")
}

async fn discover_md_files(working_dir: &Path) -> String {
    let mut contents = Vec::new();

    // Walk up from working_dir to root, collecting CLAUDE.md / RUSTY.md
    let mut dir = Some(working_dir.to_path_buf());
    while let Some(d) = dir {
        for name in &["CLAUDE.md", "RUSTY.md"] {
            let path = d.join(name);
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                contents.push(format!("# {} ({})\n{}", name, path.display(), content));
            }
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }

    // Also check ~/.rusty/CLAUDE.md
    if let Some(home) = dirs::home_dir() {
        let path = home.join(".rusty").join("CLAUDE.md");
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            contents.push(format!("# CLAUDE.md ({})\n{}", path.display(), content));
        }
    }

    if contents.is_empty() {
        String::new()
    } else {
        contents.join("\n\n---\n\n")
    }
}
