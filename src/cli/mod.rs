pub mod resolve;
pub mod validate;
pub mod claude;
pub mod gemini;
pub mod codex;

use std::fmt;

#[derive(Debug)]
pub enum CliError {
    Validation(String),
    NotFound(String),
    Timeout(String),
    Execution(String),
    JsonParse(String),
    QuotaLimit(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Validation(e) => write!(f, "Validation error: {}", e),
            CliError::NotFound(e) => write!(f, "Binary or file not found: {}", e),
            CliError::Timeout(e) => write!(f, "Execution timed out: {}", e),
            CliError::Execution(e) => write!(f, "Execution failed: {}", e),
            CliError::JsonParse(e) => write!(f, "Failed to parse JSON output: {}", e),
            CliError::QuotaLimit(e) => write!(f, "AI provider quota limit hit: {}", e),
        }
    }
}

impl std::error::Error for CliError {}

pub use claude::run_claude;
pub use gemini::run_gemini;
pub use codex::run_codex;

pub fn clean_path<P: AsRef<std::path::Path>>(path: P) -> String {
    let s = path.as_ref().to_string_lossy().to_string();
    if s.starts_with(r"\\?\") {
        s[4..].to_string()
    } else {
        s
    }
}

