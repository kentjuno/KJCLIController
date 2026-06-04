use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use serde_json::Value;
use crate::cli::resolve::resolve_cli_binary;
use crate::cli::validate::{validate_attachments, validate_prompt_size};
use crate::cli::CliError;

pub async fn run_claude(
    prompt: &str,
    system_prompt: Option<&str>,
    attachments: &[String],
    timeout_secs: u64,
    cwd: Option<&str>,
) -> Result<String, CliError> {
    // 1. Validate inputs
    if let Err(e) = validate_prompt_size(prompt) {
        return Err(CliError::Validation(e));
    }
    if let Some(sys) = system_prompt {
        if let Err(e) = validate_prompt_size(sys) {
            return Err(CliError::Validation(e));
        }
    }
    if let Err(e) = validate_attachments(attachments) {
        return Err(CliError::Validation(e));
    }

    // 2. Prepare command args
    let resolved_bin = resolve_cli_binary("claude");
    let mut args = vec!["-p".to_string(), "--output-format".to_string(), "json".to_string()];

    if let Some(sys) = system_prompt {
        args.push("--system-prompt".to_string());
        args.push(sys.to_string());
    }

    let mut bypass_permissions = false;
    let mut seen_dirs = HashSet::new();

    if let Some(dir) = cwd {
        if Path::new(dir).exists() {
            let abs_dir = Path::new(dir).canonicalize().unwrap_or_else(|_| Path::new(dir).to_path_buf());
            let abs_dir_str = crate::cli::clean_path(&abs_dir);
            if seen_dirs.insert(abs_dir_str.clone()) {
                args.push("--add-dir".to_string());
                args.push(abs_dir_str);
            }
            bypass_permissions = true;
        }
    }

    if !attachments.is_empty() {
        for path_str in attachments {
            let path = Path::new(path_str);
            if let Some(parent) = path.parent() {
                let abs_parent = parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf());
                let abs_parent_str = crate::cli::clean_path(&abs_parent);
                if seen_dirs.insert(abs_parent_str.clone()) {
                    args.push("--add-dir".to_string());
                    args.push(abs_parent_str);
                }
            }
        }
        bypass_permissions = true;
    }

    if bypass_permissions {
        args.push("--permission-mode".to_string());
        args.push("bypassPermissions".to_string());
    }

    // Construct full prompt with attachments appended as @<path>
    let mut full_prompt = prompt.to_string();
    if !attachments.is_empty() {
        let mut suffix = String::new();
        for path_str in attachments {
            let path = Path::new(path_str);
            let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            if !suffix.is_empty() {
                suffix.push(' ');
            }
            suffix.push_str(&format!("@{}", crate::cli::clean_path(&abs_path)));
        }
        if !full_prompt.is_empty() {
            full_prompt.push_str("\n\n");
        }
        full_prompt.push_str(&suffix);
    }

    // 3. Spawn process
    let mut cmd = Command::new(&resolved_bin);
    cmd.args(&args)
       .stdin(Stdio::piped())
       .stdout(Stdio::piped())
       .stderr(Stdio::piped())
       .kill_on_drop(true);

    if let Some(dir) = cwd {
        if Path::new(dir).exists() {
            cmd.current_dir(dir);
        }
    }

    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Err(CliError::Execution(format!("Failed to spawn Claude CLI: {}", e))),
    };

    // Write input and drop stdin to signal EOF
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(full_prompt.as_bytes()).await {
            return Err(CliError::Execution(format!("Failed to write to Claude CLI stdin: {}", e)));
        }
    }

    // Wait with timeout
    let wait_output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    ).await;

    let output = match wait_output {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(CliError::Execution(format!("Claude CLI execution failed: {}", e))),
        Err(_) => {
            return Err(CliError::Timeout(format!("Claude CLI timed out after {}s", timeout_secs)));
        }
    };


    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CliError::Execution(format!(
            "Claude CLI exited with status {}: {}",
            output.status,
            stderr.chars().take(400).collect::<String>()
        )));
    }

    // 4. Parse JSON envelope
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let envelope: Value = match serde_json::from_str(&stdout_str) {
        Ok(v) => v,
        Err(e) => return Err(CliError::JsonParse(format!(
            "Claude CLI returned non-JSON output: {} (error: {})",
            stdout_str.chars().take(200).collect::<String>(),
            e
        ))),
    };

    if let Some(is_err) = envelope.get("is_error").and_then(|v| v.as_bool()) {
        if is_err {
            let err_msg = envelope
                .get("result")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(CliError::Execution(err_msg.to_string()));
        }
    }

    if let Some(res) = envelope.get("result").and_then(|v| v.as_str()) {
        Ok(res.to_string())
    } else {
        Err(CliError::JsonParse("Claude CLI envelope missing string 'result' field".to_string()))
    }
}
