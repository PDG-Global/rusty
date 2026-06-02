// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    BypassPermissions,
    Plan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionLevel {
    None,
    ReadOnly,
    Write,
    Execute,
}

#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub id: u64,
    pub tool_name: String,
    pub description: String,
    pub raw_input: String,
    pub is_read_only: bool,
    pub required_level: PermissionLevel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    AllowOnce,
    AllowSession,
    AllowAlways,
    Deny(String),
}

pub fn check_auto_permission(mode: PermissionMode, level: PermissionLevel) -> PermissionDecision {
    match mode {
        PermissionMode::BypassPermissions => PermissionDecision::AllowOnce,
        PermissionMode::AcceptEdits => PermissionDecision::AllowOnce,
        PermissionMode::Plan => {
            if level == PermissionLevel::ReadOnly || level == PermissionLevel::None {
                PermissionDecision::AllowOnce
            } else {
                PermissionDecision::Deny("Plan mode is read-only".to_string())
            }
        }
        PermissionMode::Default => {
            if level == PermissionLevel::ReadOnly || level == PermissionLevel::None {
                PermissionDecision::AllowOnce
            } else {
                PermissionDecision::Deny("Requires user approval".to_string())
            }
        }
    }
}

// --- Bash command classification ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BashClassification {
    ReadOnly,
    Write,
}

/// Returns true if the command contains shell metacharacters that could enable
/// command substitution (backticks, $()).
fn contains_command_substitution(cmd: &str) -> bool {
    cmd.contains('`') || cmd.contains("$(")
}

/// Classify whether a bash command is read-only or write/execute.
pub fn classify_bash_command(command: &str) -> BashClassification {
    let trimmed = command.trim();

    // Redirects always mean write
    if trimmed.contains('>') || trimmed.contains(">>") {
        return BashClassification::Write;
    }

    // Command substitution (backticks, $()) can execute arbitrary commands
    if contains_command_substitution(trimmed) {
        return BashClassification::Write;
    }

    // Split on shell operators: &&, ||, |, ;
    let parts = split_shell_operators(trimmed);

    for part in &parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if classify_single_command(part) == BashClassification::Write {
            return BashClassification::Write;
        }
    }

    BashClassification::ReadOnly
}

fn split_shell_operators(cmd: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if i + 1 < chars.len() {
            let pair = format!("{}{}", chars[i], chars[i + 1]);
            if pair == "&&" || pair == "||" {
                parts.push(current.clone());
                current.clear();
                i += 2;
                continue;
            }
        }
        if chars[i] == '|' || chars[i] == ';' || chars[i] == '\n' {
            parts.push(current.clone());
            current.clear();
            i += 1;
            continue;
        }
        current.push(chars[i]);
        i += 1;
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }

    parts
}

fn classify_single_command(cmd: &str) -> BashClassification {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return BashClassification::ReadOnly;
    }

    let bin = parts[0];

    // Commands that are always safe (read-only)
    match bin {
        "ls" | "cat" | "find" | "grep" | "rg" | "wc" | "head" | "tail" | "echo"
        | "pwd" | "which" | "env" | "printenv" | "whoami" | "date" | "uname"
        | "file" | "stat" | "du" | "df" | "tree" | "less" | "more" | "type"
        | "command" | "help" | "man" | "info" | "true" | "false" | "test" | "["
        | "diff" | "comm" | "sort" | "uniq" | "cut" | "tr"
        | "basename" | "dirname" | "realpath" | "readlink"
        | "hostname" | "id" | "groups" | "tty" | "stty" | "clear" | "history"
        | "strings" | "hexdump" | "od" | "xxd" | "md5sum" | "sha256sum" => {
            return BashClassification::ReadOnly;
        }
        // sed: read-only unless -i (in-place edit)
        "sed" => {
            if parts.iter().any(|a| *a == "-i" || a.starts_with("-i")) {
                return BashClassification::Write;
            }
            return BashClassification::ReadOnly;
        }
        // awk: read-only in typical usage; hard to detect all write patterns
        // but safe for the common case (no redirection in the program text)
        "awk" | "gawk" | "mawk" => {
            let full = parts[1..].join(" ");
            if full.contains('>') || full.contains("`") || full.contains("$(") {
                return BashClassification::Write;
            }
            return BashClassification::ReadOnly;
        }
        // xargs: always write — it executes whatever command is piped into it
        "xargs" => {
            return BashClassification::Write;
        }
        // tee: always write — its purpose is to write to files
        "tee" => {
            return BashClassification::Write;
        }
        _ => {}
    }

    // git subcommands
    if bin == "git" {
        return classify_git_subcommand(&parts[1..]);
    }

    // cargo subcommands
    if bin == "cargo" {
        return classify_cargo_subcommand(&parts[1..]);
    }

    // npm/npx subcommands
    if bin == "npm" || bin == "npx" {
        return classify_npm_subcommand(&parts[1..]);
    }

    // pip subcommands
    if bin == "pip" || bin == "pip3" {
        return classify_pip_subcommand(&parts[1..]);
    }

    // docker subcommands
    if bin == "docker" {
        return classify_docker_subcommand(&parts[1..]);
    }

    // rustfmt: read-only with --check, write without
    if bin == "rustfmt" {
        if parts.iter().any(|a| *a == "--check") {
            return BashClassification::ReadOnly;
        }
        return BashClassification::Write;
    }

    // node/python with --version or --help is read-only
    if bin == "node" || bin == "python" || bin == "python3" || bin == "ruby" || bin == "perl" {
        if parts.len() > 1
            && (parts.contains(&"--version") || parts.contains(&"--help") || parts.contains(&"-V"))
        {
            return BashClassification::ReadOnly;
        }
        // Running scripts could be anything — classify as write
        return BashClassification::Write;
    }

    // Default: anything we don't recognize is write
    BashClassification::Write
}

fn classify_git_subcommand(args: &[&str]) -> BashClassification {
    let sub = match args.first() {
        Some(s) => *s,
        None => return BashClassification::ReadOnly, // bare `git` is harmless
    };

    // git config: read-only for `git config --get/list`, write for --global/--system/--edit/set
    if sub == "config" {
        let rest = &args[1..];
        if rest.iter().any(|a| {
            matches!(
                *a,
                "--global" | "--system" | "--edit" | "--replace-all" | "--unset" | "--unset-all"
            ) || a.starts_with("--global=")
                || a.starts_with("--system=")
        }) {
            return BashClassification::Write;
        }
        // `git config key value` (setting a value) has 2+ non-flag args
        let non_flags: Vec<&str> = rest.iter().copied().filter(|a| !a.starts_with('-')).collect();
        if non_flags.len() >= 2 {
            return BashClassification::Write;
        }
        return BashClassification::ReadOnly;
    }

    // git bisect modifies .git/ state
    if sub == "bisect" {
        return BashClassification::Write;
    }

    match sub {
        // Read-only git subcommands
        "status" | "log" | "diff" | "show" | "branch" | "tag" | "remote"
        | "blame" | "shortlog" | "describe" | "rev-parse" | "rev-list"
        | "ls-files" | "ls-remote" | "ls-tree" | "cat-file" | "for-each-ref"
        | "symbolic-ref" | "name-rev" | "count-objects" | "version"
        | "help" | "archive" | "grep" => {
            BashClassification::ReadOnly
        }
        _ => BashClassification::Write,
    }
}

fn classify_cargo_subcommand(args: &[&str]) -> BashClassification {
    let sub = match args.first() {
        Some(s) => *s,
        None => return BashClassification::ReadOnly,
    };

    // cargo fmt without --check modifies files in-place
    if sub == "fmt" {
        if args[1..].iter().any(|a| *a == "--check") {
            return BashClassification::ReadOnly;
        }
        return BashClassification::Write;
    }

    match sub {
        "check" | "clippy" | "test" | "doc" | "tree" | "metadata"
        | "version" | "help" | "search" | "verify-project" | "locate-project"
        | "manifest" | "read-manifest" | "rustc" | "rustdoc" => {
            BashClassification::ReadOnly
        }
        _ => BashClassification::Write,
    }
}

fn classify_npm_subcommand(args: &[&str]) -> BashClassification {
    let sub = match args.first() {
        Some(s) => *s,
        None => return BashClassification::ReadOnly,
    };

    match sub {
        "list" | "ls" | "outdated" | "view" | "info" | "search" | "whoami"
        | "version" | "help" | "config" | "prefix" | "root" | "bin" => {
            BashClassification::ReadOnly
        }
        _ => BashClassification::Write,
    }
}

fn classify_pip_subcommand(args: &[&str]) -> BashClassification {
    let sub = match args.first() {
        Some(s) => *s,
        None => return BashClassification::ReadOnly,
    };

    match sub {
        "list" | "show" | "search" | "freeze" | "check" | "index" | "help" => {
            BashClassification::ReadOnly
        }
        _ => BashClassification::Write,
    }
}

fn classify_docker_subcommand(args: &[&str]) -> BashClassification {
    let sub = match args.first() {
        Some(s) => *s,
        None => return BashClassification::ReadOnly,
    };

    match sub {
        "ps" | "images" | "logs" | "inspect" | "top" | "port" | "stats"
        | "version" | "info" | "help" => BashClassification::ReadOnly,
        _ => BashClassification::Write,
    }
}

/// Build a human-readable description for a tool call
pub fn build_tool_description(tool_name: &str, arguments: &str) -> String {
    let input: serde_json::Value =
        serde_json::from_str(arguments).unwrap_or(serde_json::Value::Null);

    match tool_name {
        "bash" => input["command"]
            .as_str()
            .unwrap_or("(no command)")
            .to_string(),
        "file_read" => format!("Read {}", input["path"].as_str().unwrap_or("?")),
        "file_write" => format!("Write {}", input["path"].as_str().unwrap_or("?")),
        "file_edit" => format!("Edit {}", input["path"].as_str().unwrap_or("?")),
        "glob" => format!("Glob: {}", input["pattern"].as_str().unwrap_or("?")),
        "grep" => format!("Grep: {}", input["pattern"].as_str().unwrap_or("?")),
        _ => format!("{tool_name}({arguments})"),
    }
}

/// Create an allow key for the allowlist. For bash, includes the first command word.
pub fn make_allow_key(tool_name: &str, arguments: &str) -> String {
    if tool_name == "bash" {
        let input: serde_json::Value =
            serde_json::from_str(arguments).unwrap_or(serde_json::Value::Null);
        let cmd = input["command"].as_str().unwrap_or("");
        let first_word = cmd.split_whitespace().next().unwrap_or("");
        // For git/cargo/npm, include the subcommand
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.len() >= 2 && matches!(parts[0], "git" | "cargo" | "npm" | "docker" | "pip") {
            format!("bash:{} {}", parts[0], parts[1])
        } else {
            format!("bash:{first_word}")
        }
    } else {
        tool_name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_read_only_commands() {
        assert_eq!(classify_bash_command("ls -la"), BashClassification::ReadOnly);
        assert_eq!(classify_bash_command("cat foo.txt"), BashClassification::ReadOnly);
        assert_eq!(classify_bash_command("git status"), BashClassification::ReadOnly);
        assert_eq!(classify_bash_command("git log --oneline"), BashClassification::ReadOnly);
        assert_eq!(classify_bash_command("git diff"), BashClassification::ReadOnly);
        assert_eq!(classify_bash_command("cargo check"), BashClassification::ReadOnly);
        assert_eq!(classify_bash_command("cargo test"), BashClassification::ReadOnly);
        assert_eq!(classify_bash_command("npm list"), BashClassification::ReadOnly);
    }

    #[test]
    fn test_classify_write_commands() {
        assert_eq!(classify_bash_command("rm -rf /tmp/foo"), BashClassification::Write);
        assert_eq!(classify_bash_command("git push origin main"), BashClassification::Write);
        assert_eq!(classify_bash_command("git commit -m msg"), BashClassification::Write);
        assert_eq!(classify_bash_command("cargo build"), BashClassification::Write);
        assert_eq!(classify_bash_command("cargo run"), BashClassification::Write);
        assert_eq!(classify_bash_command("npm install foo"), BashClassification::Write);
    }

    #[test]
    fn test_compound_commands() {
        assert_eq!(classify_bash_command("ls && git push"), BashClassification::Write);
        assert_eq!(classify_bash_command("ls | grep foo"), BashClassification::ReadOnly);
        assert_eq!(classify_bash_command("echo hello > file.txt"), BashClassification::Write);
    }
}
