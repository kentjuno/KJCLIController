use std::fs;
use std::path::Path;

pub const MAX_PROMPT_BYTES: usize = 100 * 1024; // 100 KB
pub const MAX_ATTACHMENTS: usize = 10;

pub fn validate_prompt_size(prompt: &str) -> Result<(), String> {
    if prompt.len() > MAX_PROMPT_BYTES {
        return Err(format!(
            "Prompt size {} exceeds limit of {} KB",
            prompt.len(),
            MAX_PROMPT_BYTES / 1024
        ));
    }
    Ok(())
}

pub fn validate_attachments(attachments: &[String]) -> Result<(), String> {
    if attachments.len() > MAX_ATTACHMENTS {
        return Err(format!(
            "Attachment count {} exceeds limit of {}",
            attachments.len(),
            MAX_ATTACHMENTS
        ));
    }

    for path_str in attachments {
        // Sanitize path against directory traversal
        let path = Path::new(path_str);
        
        // Simple sanity check for relative parent traversal
        if path_str.contains("..") {
            return Err(format!("Invalid path traversal in filename: {}", path_str));
        }

        if !path.exists() {
            return Err(format!("File does not exist: {}", path_str));
        }

        if !path.is_file() {
            return Err(format!("Attachment is not a file: {}", path_str));
        }

        // Check if readable by trying to open metadata
        if fs::metadata(path).is_err() {
            return Err(format!("File is not readable: {}", path_str));
        }
    }

    Ok(())
}
