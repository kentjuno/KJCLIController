use std::sync::OnceLock;
use std::collections::HashMap;
use tokio::sync::RwLock;
use crate::cli::resolve::{resolve_cli_binary, create_std_command};
use crate::cli::{run_claude, run_gemini, run_codex, CliError};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderStatus {
    pub name: String,
    pub available: bool,
    pub supports_vision: bool,
}

// Caching provider availability for 60 seconds to avoid calling CLI `--version` on every request
struct AvailabilityCache {
    cached_states: HashMap<String, (bool, std::time::Instant)>,
}

fn get_cache() -> &'static RwLock<AvailabilityCache> {
    static CACHE: OnceLock<RwLock<AvailabilityCache>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(AvailabilityCache {
        cached_states: HashMap::new(),
    }))
}

pub async fn is_available(provider: &str) -> bool {
    let now = std::time::Instant::now();
    
    // Check cache
    {
        let cache = get_cache().read().await;
        if let Some(&(val, expiry)) = cache.cached_states.get(provider) {
            if now < expiry {
                return val;
            }
        }
    }

    // Cache miss, run probe
    let val = match provider {
        "claude" => {
            let bin = resolve_cli_binary("claude");
            let mut cmd = create_std_command(&bin);
            cmd.arg("--version");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            cmd.output().map(|out| out.status.success()).unwrap_or(false)
        }
        "gemini" | "agy" => {
            let bin = resolve_cli_binary("agy");
            let mut cmd = create_std_command(&bin);
            cmd.arg("--version");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            cmd.output().map(|out| out.status.success()).unwrap_or(false)
        }
        "openai" | "codex" => {
            let bin = resolve_cli_binary("codex");
            let mut cmd = create_std_command(&bin);
            cmd.arg("--version");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            cmd.output().map(|out| out.status.success()).unwrap_or(false)
        }
        _ => false,
    };

    // Update cache with 60-second TTL
    {
        let mut cache = get_cache().write().await;
        cache.cached_states.insert(
            provider.to_string(),
            (val, now + std::time::Duration::from_secs(60)),
        );
    }

    val
}

pub async fn list_providers() -> Vec<ProviderStatus> {
    vec![
        ProviderStatus {
            name: "claude".to_string(),
            available: is_available("claude").await,
            supports_vision: true,
        },
        ProviderStatus {
            name: "gemini".to_string(),
            available: is_available("gemini").await,
            supports_vision: true,
        },
        ProviderStatus {
            name: "openai".to_string(),
            available: is_available("openai").await,
            supports_vision: true,
        },
    ]
}

pub async fn run_llm(
    provider: &str,
    prompt: &str,
    system_prompt: Option<&str>,
    attachments: &[String],
    timeout_secs: u64,
    temp_dir: &str,
    cwd: Option<&str>,
) -> Result<String, CliError> {
    // Fail-fast gate: check availability before launching process
    if !is_available(provider).await {
        return Err(CliError::NotFound(format!(
            "Provider '{}' is not available or configured on this host",
            provider
        )));
    }

    match provider {
        "claude" => run_claude(prompt, system_prompt, attachments, timeout_secs, cwd).await,
        "gemini" | "agy" => run_gemini(prompt, system_prompt, attachments, timeout_secs, temp_dir, cwd).await,
        "openai" | "codex" => run_codex(prompt, system_prompt, attachments, timeout_secs, temp_dir, cwd).await,
        _ => Err(CliError::Validation(format!("Unsupported provider: {}", provider))),
    }
}
