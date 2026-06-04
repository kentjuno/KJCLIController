use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tracing::info;
use crate::cli::resolve::{resolve_cli_binary, create_command};
use crate::cli::validate::{validate_attachments, validate_prompt_size};
use crate::cli::CliError;


fn probe_image_flag() -> Option<String> {
    let resolved = resolve_cli_binary("codex");
    let mut cmd = std::process::Command::new(resolved);
    cmd.arg("exec").arg("--help");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    if let Ok(out) = cmd.output() {
        let help_text = String::from_utf8_lossy(&out.stdout);
        for candidate in &["--image", "--attach", "--file", "--input"] {
            if help_text.contains(candidate) {
                info!("Found Codex image attachment flag: {}", candidate);
                return Some(candidate.to_string());
            }
        }
    }
    None
}

fn get_image_flag() -> Option<String> {
    static FLAG: OnceLock<Option<String>> = OnceLock::new();
    FLAG.get_or_init(probe_image_flag).clone()
}


pub async fn run_codex(
    prompt: &str,
    system_prompt: Option<&str>,
    attachments: &[String],
    timeout_secs: u64,
    temp_dir: &str,
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

    // 2. Validate attachment capability support
    let image_flag = get_image_flag();
    if !attachments.is_empty() && image_flag.is_none() {
        return Err(CliError::Validation(
            "OpenAI Codex CLI on this host does not support attachments (no attachment flags like --image or --attach found in help output)".to_string()
        ));
    }

    run_codex_cli(prompt, system_prompt, attachments, image_flag.as_deref(), timeout_secs, temp_dir, cwd).await
}

async fn run_codex_cli(
    prompt: &str,
    system_prompt: Option<&str>,
    attachments: &[String],
    image_flag: Option<&str>,
    timeout_secs: u64,
    temp_dir: &str,
    cwd: Option<&str>,
) -> Result<String, CliError> {
    let resolved_bin = resolve_cli_binary("codex");
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    
    let temp_dir_path = PathBuf::from(temp_dir);
    let abs_temp_dir = fs::canonicalize(&temp_dir_path).unwrap_or(temp_dir_path.clone());
    let temp_out = abs_temp_dir.join(format!("codex_out_{}.txt", run_id));
    let clean_temp_out = crate::cli::clean_path(&temp_out);

    let mut args = vec![
        "-a".to_string(),
        "never".to_string(),
        "-s".to_string(),
        "danger-full-access".to_string(),
        "--no-alt-screen".to_string(),
        "exec".to_string(),
        "--dangerously-bypass-approvals-and-sandbox".to_string(),
        "--dangerously-bypass-hook-trust".to_string(),
        "--ephemeral".to_string(),
        "-o".to_string(),
        clean_temp_out,
    ];

    if !attachments.is_empty() {
        if let Some(flag) = image_flag {
            for path_str in attachments {
                let path = Path::new(path_str);
                let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
                args.push(flag.to_string());
                args.push(crate::cli::clean_path(&abs_path));
            }
        }
    }

    // Stdin sentinel `-` tells codex to read prompt from stdin
    args.push("-".to_string());

    let mut cmd = create_command(&resolved_bin);
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
        Err(e) => return Err(CliError::Execution(format!("Failed to spawn Codex CLI: {}", e))),
    };

    // Write input and drop stdin
    if let Some(mut stdin) = child.stdin.take() {
        let full_input = if let Some(sys) = system_prompt {
            format!("[System: {}]\n\n{}", sys, prompt)
        } else {
            prompt.to_string()
        };
        if let Err(e) = stdin.write_all(full_input.as_bytes()).await {
            return Err(CliError::Execution(format!("Failed to write to Codex CLI stdin: {}", e)));
        }
    }

    // Wait with timeout
    let wait_output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    ).await;

    let output = match wait_output {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            let _ = fs::remove_file(&temp_out);
            return Err(CliError::Execution(format!("Codex CLI execution failed: {}", e)));
        }
        Err(_) => {
            let _ = fs::remove_file(&temp_out);
            return Err(CliError::Timeout(format!("Codex CLI timed out after {}s", timeout_secs)));
        }
    };


    if !output.status.success() {
        let _ = fs::remove_file(&temp_out);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CliError::Execution(format!(
            "Codex CLI exited with code {}: {}",
            output.status,
            stderr.chars().take(400).collect::<String>()
        )));
    }

    // Read the result from output file
    let mut response_text = String::new();
    if temp_out.exists() {
        if let Ok(text) = fs::read_to_string(&temp_out) {
            response_text = text.trim().to_string();
        }
        let _ = fs::remove_file(&temp_out);
    }

    if response_text.is_empty() {
        // Fallback to reading stdout if output file was empty
        response_text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }

    if response_text.is_empty() {
        return Err(CliError::Execution("Codex CLI returned empty output".to_string()));
    }

    Ok(response_text)
}
