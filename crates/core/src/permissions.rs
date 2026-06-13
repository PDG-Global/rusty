// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    BypassPermissions,
    Plan,
}

/// Returns a short behavioral guidance string for the given permission mode.
/// This is injected into the system prompt to tell the model how to behave
/// regarding approvals and autonomy.
pub fn permission_mode_prompt(mode: PermissionMode) -> Option<&'static str> {
    match mode {
        PermissionMode::Default => Some(
            "## Permission Mode\n\n\
            You are in default permission mode. Write and execute tools require user approval. \
            Proceed with read-only work while waiting for approval.",
        ),
        PermissionMode::AcceptEdits => Some(
            "## Permission Mode\n\n\
            You are in accept-edits mode. File writes and edits are auto-approved. \
            Execute commands still require user approval. Proceed with file changes directly.",
        ),
        PermissionMode::BypassPermissions => Some(
            "## Permission Mode\n\n\
            You are in bypass-permissions mode. All tools are auto-approved. \
            Work autonomously — do not pause for approvals or ask clarifying questions. \
            Make reasonable decisions and continue working.",
        ),
        PermissionMode::Plan => Some(
            "## Permission Mode\n\n\
            You are in plan mode. Write and execute tools are disabled. \
            Focus on research and planning using read-only tools. \
            Use exit_plan_mode when you are ready to execute.",
        ),
    }
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

/// Files and directories that should never be auto-approved for writes,
/// even in AcceptEdits or Bypass mode. These always require explicit user confirmation.
pub const PROTECTED_PATH_PATTERNS: &[&str] = &[
    ".gitconfig",
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".zshrc",
    ".zprofile",
    ".zshenv",
    ".profile",
    ".ssh",
    ".gnupg",
    ".mcp.json",
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    "id_dsa",
    ".npmrc",
    ".pypirc",
    ".netrc",
    ".docker/config.json",
    ".aws/credentials",
    ".aws/config",
    ".kube/config",
    ".rusty",
];

/// Check if a file path matches any protected pattern.
/// Uses path-component matching to avoid false positives from substring matching
/// (e.g. "my_ssh_config" should not match ".ssh").
pub fn is_protected_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let components: Vec<&str> = lower.split('/').collect();

    PROTECTED_PATH_PATTERNS.iter().any(|pat| {
        // Single-segment pattern: match against individual path components
        if !pat.contains('/') {
            if components.iter().any(|c| *c == *pat) {
                return true;
            }
            // Also match without leading dot (e.g. "id_rsa" in a path)
            let trimmed = pat.trim_start_matches('.');
            if trimmed != *pat
                && components
                    .iter()
                    .any(|c| c.trim_start_matches('.') == trimmed)
            {
                return true;
            }
            return false;
        }
        // Multi-segment pattern: match against joined consecutive components
        let seg_count = pat.matches('/').count() + 1;
        if components.len() >= seg_count {
            for window in components.windows(seg_count) {
                if window.join("/") == *pat {
                    return true;
                }
            }
        }
        false
    })
}

pub fn check_auto_permission(mode: PermissionMode, level: PermissionLevel) -> PermissionDecision {
    match mode {
        PermissionMode::BypassPermissions => PermissionDecision::AllowOnce,
        PermissionMode::AcceptEdits => {
            if level == PermissionLevel::Write {
                PermissionDecision::AllowOnce
            } else if level == PermissionLevel::ReadOnly || level == PermissionLevel::None {
                PermissionDecision::AllowOnce
            } else {
                // Execute still requires approval in AcceptEdits mode
                PermissionDecision::Deny(
                    "Execute requires approval in AcceptEdits mode".to_string(),
                )
            }
        }
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
    Execute,
}

/// Returns true if the command contains shell metacharacters that could enable
/// command substitution (backticks, $()), process substitution (<(), >()), or
/// other dangerous constructs.
fn contains_command_substitution(cmd: &str) -> bool {
    cmd.contains('`') || cmd.contains("$(") || cmd.contains("<(") || cmd.contains(">(")
}

/// Returns true if the command contains a heredoc (<< or <<-) redirect.
/// Heredocs allow writing arbitrary content and should be classified as write.
fn contains_heredoc(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for i in 0..len {
        match bytes[i] {
            b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            b'<' if !in_single_quote && !in_double_quote => {
                if i + 1 < len && bytes[i + 1] == b'<' {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Classify whether a bash command is read-only or write/execute.
pub fn classify_bash_command(command: &str) -> BashClassification {
    let trimmed = command.trim();

    // Shell redirects: >file, >>file, 2>file, 2>>file, etc.
    // Use a simple state machine to detect redirects while avoiding false
    // positives from `>` inside strings or arguments like `->`.
    if contains_redirect(trimmed) {
        return BashClassification::Write;
    }

    // Heredocs (<<, <<-) allow writing arbitrary content
    if contains_heredoc(trimmed) {
        return BashClassification::Write;
    }

    // Command substitution (backticks, $()) and process substitution (<(), >())
    // can execute arbitrary commands
    if contains_command_substitution(trimmed) {
        return BashClassification::Write;
    }

    // Split on shell operators: &&, ||, |, ;
    let parts = split_shell_operators(trimmed);

    let mut result = BashClassification::ReadOnly;

    for part in &parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match classify_single_command(part) {
            BashClassification::Execute => return BashClassification::Execute,
            BashClassification::Write => result = BashClassification::Write,
            _ => {}
        }
    }

    result
}

/// Detect shell redirect operators (> file, >>file, 2>file, etc.)
/// without false positives from `>` in arguments or quoted strings.
fn contains_redirect(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < len {
        match bytes[i] {
            b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            b'>' if !in_single_quote && !in_double_quote => {
                // Check if this `>` looks like a redirect:
                // - preceded by whitespace, start-of-string, or a digit (fd), or & (2>&1)
                // - not preceded by another > that we already counted (handles >>)
                let prev_ok = if i == 0 {
                    true
                } else {
                    matches!(
                        bytes[i - 1],
                        b' ' | b'\t' | b'\n' | b'0'
                            ..=b'9' | b'&' | b'>' | b'|' | b';' | b'(' | b'{'
                    )
                };
                if prev_ok {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
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
        if chars[i] == '|' || chars[i] == ';' || chars[i] == '\n' || chars[i] == '&' {
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
        "ls" | "cat" | "grep" | "rg" | "wc" | "head" | "tail" | "echo" | "pwd" | "which"
        | "whoami" | "date" | "uname" | "file" | "stat" | "du" | "df" | "tree" | "less"
        | "more" | "type" | "help" | "man" | "info" | "true" | "false" | "test" | "[" | "diff"
        | "comm" | "uniq" | "cut" | "tr" | "basename" | "dirname" | "realpath" | "readlink"
        | "hostname" | "id" | "groups" | "tty" | "stty" | "clear" | "history" | "strings"
        | "hexdump" | "od" | "xxd" | "md5sum" | "sha256sum" => {
            return BashClassification::ReadOnly;
        }
        // sed: read-only unless -i (in-place edit) or w (write to file)
        "sed" => {
            if parts.iter().any(|a| *a == "-i" || a.starts_with("-i")) {
                return BashClassification::Write;
            }
            let full = parts[1..].join(" ");
            // sed 'w filename' writes pattern space to a file
            if Regex::new(r"(?:^|[;\s|])w\s+\S").unwrap().is_match(&full) {
                return BashClassification::Write;
            }
            return BashClassification::ReadOnly;
        }
        // awk: read-only in typical usage; detect write patterns
        "awk" | "gawk" | "mawk" => {
            let full = parts[1..].join(" ");
            if full.contains('>')
                || full.contains("`")
                || full.contains("$(")
                || full.contains("system(")
                || full.contains("getline")
            {
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
        // eval/source/.: always execute — they run arbitrary code from arguments/files
        // env/printenv: always execute — they expose all environment variables including secrets
        "eval" | "source" | "." | "env" | "printenv" => {
            return BashClassification::Execute;
        }
        // find: read-only unless -exec, -execdir, -ok, -okdir (execute arbitrary commands)
        "find" => {
            let full = parts[1..].join(" ");
            if full.contains("-exec") || full.contains("-ok") {
                return BashClassification::Execute;
            }
            return BashClassification::ReadOnly;
        }
        // command: read-only only for -v/-V (query), otherwise executes
        "command" => {
            if parts.len() >= 2 && (parts[1] == "-v" || parts[1] == "-V") {
                return BashClassification::ReadOnly;
            }
            return BashClassification::Execute;
        }
        // sort: read-only unless -o (output to file)
        "sort" => {
            let full = parts[1..].join(" ");
            if full.contains("-o") {
                return BashClassification::Write;
            }
            return BashClassification::ReadOnly;
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

    // Flags that let git operate outside the working directory change the
    // effective scope of the command; treat them as write/execute.
    if args.iter().any(|a| {
        *a == "-C"
            || a.starts_with("-C=")
            || *a == "--git-dir"
            || a.starts_with("--git-dir=")
            || *a == "--work-tree"
            || a.starts_with("--work-tree=")
    }) {
        return BashClassification::Write;
    }

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
        let non_flags: Vec<&str> = rest
            .iter()
            .copied()
            .filter(|a| !a.starts_with('-'))
            .collect();
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
        "status" | "log" | "diff" | "show" | "branch" | "tag" | "remote" | "blame" | "shortlog"
        | "describe" | "rev-parse" | "rev-list" | "ls-files" | "ls-remote" | "ls-tree"
        | "cat-file" | "for-each-ref" | "symbolic-ref" | "name-rev" | "count-objects"
        | "version" | "help" | "archive" | "grep" => BashClassification::ReadOnly,
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
        "check" | "clippy" | "test" | "doc" | "tree" | "metadata" | "version" | "help"
        | "search" | "verify-project" | "locate-project" | "manifest" | "read-manifest"
        | "rustc" | "rustdoc" => BashClassification::ReadOnly,
        _ => BashClassification::Write,
    }
}

fn classify_npm_subcommand(args: &[&str]) -> BashClassification {
    let sub = match args.first() {
        Some(s) => *s,
        None => return BashClassification::ReadOnly,
    };

    match sub {
        "list" | "ls" | "outdated" | "view" | "info" | "search" | "whoami" | "version" | "help"
        | "config" | "prefix" | "root" | "bin" => BashClassification::ReadOnly,
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
        "ps" | "images" | "logs" | "inspect" | "top" | "port" | "stats" | "version" | "info"
        | "help" => BashClassification::ReadOnly,
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
/// Dangerous commands always include the full command for safety.
pub fn make_allow_key(tool_name: &str, arguments: &str) -> String {
    // Commands that should never be broadly allowed — require full command match
    const DANGEROUS_COMMANDS: &[&str] = &[
        "rm",
        "chmod",
        "chown",
        "kill",
        "dd",
        "mkfs",
        "fdisk",
        "mount",
        "umount",
        "reboot",
        "shutdown",
        "init",
        "systemctl",
        "service",
        "mv",
        "cp",
        "sudo",
        "ssh",
        "curl",
        "wget",
        "tar",
        "apt",
        "make",
    ];

    if tool_name == "bash" {
        let input: serde_json::Value =
            serde_json::from_str(arguments).unwrap_or(serde_json::Value::Null);
        let cmd = input["command"].as_str().unwrap_or("");
        let first_word = cmd.split_whitespace().next().unwrap_or("");
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.len() >= 2 && matches!(parts[0], "git" | "cargo" | "npm" | "docker" | "pip") {
            format!("bash:{} {}", parts[0], parts[1])
        } else if DANGEROUS_COMMANDS.contains(&first_word) {
            // Include full command for dangerous commands
            format!("bash:{}", parts.join(" "))
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
        assert_eq!(
            classify_bash_command("ls -la"),
            BashClassification::ReadOnly
        );
        assert_eq!(
            classify_bash_command("cat foo.txt"),
            BashClassification::ReadOnly
        );
        assert_eq!(
            classify_bash_command("git status"),
            BashClassification::ReadOnly
        );
        assert_eq!(
            classify_bash_command("git log --oneline"),
            BashClassification::ReadOnly
        );
        assert_eq!(
            classify_bash_command("git diff"),
            BashClassification::ReadOnly
        );
        assert_eq!(
            classify_bash_command("cargo check"),
            BashClassification::ReadOnly
        );
        assert_eq!(
            classify_bash_command("cargo test"),
            BashClassification::ReadOnly
        );
        assert_eq!(
            classify_bash_command("npm list"),
            BashClassification::ReadOnly
        );
    }

    #[test]
    fn test_classify_write_commands() {
        assert_eq!(
            classify_bash_command("rm -rf /tmp/foo"),
            BashClassification::Write
        );
        assert_eq!(
            classify_bash_command("git push origin main"),
            BashClassification::Write
        );
        assert_eq!(
            classify_bash_command("git commit -m msg"),
            BashClassification::Write
        );
        assert_eq!(
            classify_bash_command("cargo build"),
            BashClassification::Write
        );
        assert_eq!(
            classify_bash_command("cargo run"),
            BashClassification::Write
        );
        assert_eq!(
            classify_bash_command("npm install foo"),
            BashClassification::Write
        );
    }

    #[test]
    fn test_git_outside_working_dir_is_write() {
        // -C and --git-dir let git operate on arbitrary directories.
        assert_eq!(
            classify_bash_command("git -C /etc status"),
            BashClassification::Write
        );
        assert_eq!(
            classify_bash_command("git --git-dir=/etc/.git status"),
            BashClassification::Write
        );
        assert_eq!(
            classify_bash_command("git --work-tree=/etc status"),
            BashClassification::Write
        );
        // Plain git status remains read-only.
        assert_eq!(
            classify_bash_command("git status"),
            BashClassification::ReadOnly
        );
    }

    #[test]
    fn test_compound_commands() {
        assert_eq!(
            classify_bash_command("ls && git push"),
            BashClassification::Write
        );
        assert_eq!(
            classify_bash_command("ls | grep foo"),
            BashClassification::ReadOnly
        );
        assert_eq!(
            classify_bash_command("echo hello > file.txt"),
            BashClassification::Write
        );
    }

    #[test]
    fn test_eval_source_always_execute() {
        assert_eq!(
            classify_bash_command("eval echo hi"),
            BashClassification::Execute
        );
        assert_eq!(
            classify_bash_command("source setup.sh"),
            BashClassification::Execute
        );
        assert_eq!(
            classify_bash_command(". setup.sh"),
            BashClassification::Execute
        );
        assert_eq!(classify_bash_command("env"), BashClassification::Execute);
        assert_eq!(
            classify_bash_command("printenv"),
            BashClassification::Execute
        );
        assert_eq!(
            classify_bash_command("env | grep PATH"),
            BashClassification::Execute
        );
    }

    #[test]
    fn test_process_substitution_is_write() {
        assert_eq!(
            classify_bash_command("diff <(ls a) <(ls b)"),
            BashClassification::Write
        );
    }

    #[test]
    fn test_check_auto_permission_accept_edits_execute_denied() {
        // AcceptEdits should allow Write and ReadOnly but deny Execute
        assert_eq!(
            check_auto_permission(PermissionMode::AcceptEdits, PermissionLevel::Write),
            PermissionDecision::AllowOnce
        );
        assert_eq!(
            check_auto_permission(PermissionMode::AcceptEdits, PermissionLevel::ReadOnly),
            PermissionDecision::AllowOnce
        );
        assert!(matches!(
            check_auto_permission(PermissionMode::AcceptEdits, PermissionLevel::Execute),
            PermissionDecision::Deny(_)
        ));
    }

    // ── is_protected_path ────────────────────────────────────────────

    #[test]
    fn test_protected_gitconfig() {
        assert!(is_protected_path("/home/user/.gitconfig"));
        assert!(is_protected_path(".gitconfig"));
    }

    #[test]
    fn test_protected_ssh() {
        assert!(is_protected_path("/home/user/.ssh/id_rsa"));
        assert!(is_protected_path("~/.ssh/known_hosts"));
    }

    #[test]
    fn test_protected_rusty_dir() {
        assert!(is_protected_path("/home/user/.rusty/settings.json"));
    }

    #[test]
    fn test_not_protected_normal_file() {
        assert!(!is_protected_path("src/main.rs"));
        assert!(!is_protected_path("README.md"));
        assert!(!is_protected_path("Cargo.toml"));
    }
}
