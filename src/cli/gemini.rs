use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Semaphore;
use uuid::Uuid;
use regex::Regex;
use crate::cli::resolve::resolve_cli_binary;
use crate::cli::validate::{validate_attachments, validate_prompt_size};
use crate::cli::CliError;

fn get_agy_semaphore() -> &'static Semaphore {
    static SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();
    SEMAPHORE.get_or_init(|| Semaphore::new(1))
}

pub fn strip_ansi(text: &str) -> String {
    // Strip ANSI escape sequences using regex
    if let Ok(re) = Regex::new(r"\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])") {
        re.replace_all(text, "").into_owned()
    } else {
        text.to_string()
    }
}

pub fn extract_json_fragment(text: &str) -> Option<String> {
    // Look for matching { } or [ ] to parse JSON fragment
    let starts: Vec<usize> = text.char_indices().filter(|&(_, c)| c == '{' || c == '[').map(|(i, _)| i).collect();
    let ends: Vec<usize> = text.char_indices().filter(|&(_, c)| c == '}' || c == ']').map(|(i, _)| i).collect();

    for &start in &starts {
        for &end in ends.iter().rev() {
            if start >= end {
                continue;
            }
            let slice = &text[start..=end];
            if serde_json::from_str::<serde_json::Value>(slice).is_ok() {
                return Some(slice.to_string());
            }
        }
    }
    None
}

pub async fn run_gemini(
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

    // Acquire Semaphore permit (serialize calls)
    let _permit = get_agy_semaphore().acquire().await.unwrap();

    let resolved_bin = resolve_cli_binary("agy");
    let run_id = Uuid::new_v4().simple().to_string();
    
    let temp_dir_path = PathBuf::from(temp_dir);
    let abs_temp_dir = fs::canonicalize(&temp_dir_path).unwrap_or(temp_dir_path.clone());
    let temp_in = abs_temp_dir.join(format!("in_{}.txt", run_id));
    let temp_out = abs_temp_dir.join(format!("out_{}.txt", run_id));

    let clean_temp_in = crate::cli::clean_path(&temp_in);
    let clean_temp_out = crate::cli::clean_path(&temp_out);

    // Construct instruction payload
    let mut parts = Vec::new();
    if let Some(sys) = system_prompt {
        parts.push(format!("[System: {}]", sys));
    }
    parts.push(prompt.to_string());
    
    if !attachments.is_empty() {
        parts.push("Attachments:".to_string());
        for path_str in attachments {
            let path = Path::new(path_str);
            let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            parts.push(format!("- {}", crate::cli::clean_path(&abs_path)));
        }
    }

    let payload = parts.join("\n\n");
    let instruction = format!(
        "{}\n\nWrite the final response to this exact file path: {}.\nReturn plain text or JSON. Do not rely on stdout.",
        payload,
        clean_temp_out
    );

    // Write input instruction to file
    if let Err(e) = fs::write(&temp_in, instruction) {
        return Err(CliError::Execution(format!("Failed to write agy temp input: {}", e)));
    }

    let prompt_to_cli = format!(
        "Please read all instructions from this file: {}\nFollow them carefully and write the final response to the requested output file.",
        clean_temp_in
    );

    let work_dir = if let Some(dir) = cwd {
        if Path::new(dir).exists() {
            PathBuf::from(dir)
        } else {
            temp_dir_path.clone()
        }
    } else {
        temp_dir_path.clone()
    };

    let mut cmd = Command::new(&resolved_bin);
    cmd.args(&[
        "--print".to_string(),
        prompt_to_cli,
        "--dangerously-skip-permissions".to_string(),
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .current_dir(&work_dir)
    .kill_on_drop(true);

    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_file(&temp_in);
            return Err(CliError::Execution(format!("Failed to spawn agy CLI: {}", e)));
        }
    };


    // Wait with timeout
    let wait_output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    ).await;

    // Ensure temp input is cleaned up
    let _ = fs::remove_file(&temp_in);

    let output = match wait_output {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            let _ = fs::remove_file(&temp_out);
            return Err(CliError::Execution(format!("agy CLI execution failed: {}", e)));
        }
        Err(_) => {
            let _ = fs::remove_file(&temp_out);
            return Err(CliError::Timeout(format!("agy CLI timed out after {}s", timeout_secs)));
        }
    };


    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stderr_str = String::from_utf8_lossy(&output.stderr);
    let combined_out = format!("{}\n{}", stdout_str, stderr_str);

    // Check for quota limits
    let combined_lower = combined_out.to_lowercase();
    if ["usage limit", "upgrade to pro", "rate limit", "credits"].iter().any(|&kw| combined_lower.contains(kw)) {
        let _ = fs::remove_file(&temp_out);
        return Err(CliError::QuotaLimit(format!(
            "agy CLI quota limit hit: {}",
            strip_ansi(&combined_out).chars().take(200).collect::<String>().trim()
        )));
    }

    if !output.status.success() {
        let _ = fs::remove_file(&temp_out);
        let err_text = strip_ansi(&stderr_str);
        return Err(CliError::Execution(format!(
            "agy CLI exited with code {}: {}",
            output.status,
            err_text.chars().take(400).collect::<String>().trim()
        )));
    }

    // Try reading output from file
    let mut response_text = String::new();
    if temp_out.exists() {
        if let Ok(text) = fs::read_to_string(&temp_out) {
            response_text = text.trim().to_string();
        }
        let _ = fs::remove_file(&temp_out);
    }

    // Fallback to stdout if file was empty
    if response_text.is_empty() {
        response_text = stdout_str.trim().to_string();
    }

    if response_text.is_empty() {
        let err_text = strip_ansi(&stderr_str);
        return Err(CliError::Execution(format!(
            "agy CLI returned empty output: {}",
            err_text.chars().take(200).collect::<String>()
        )));
    }

    // Strip ANSI and parse JSON if applicable
    let cleaned = strip_ansi(&response_text);
    if let Some(json_frag) = extract_json_fragment(&cleaned) {
        Ok(json_frag)
    } else {
        Ok(cleaned)
    }
}
