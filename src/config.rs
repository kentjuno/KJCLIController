use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub token: String,
    pub port: u16,
    pub temp_dir: String,
    pub output_dir: String,
    #[serde(default)]
    pub openai_api_key: Option<String>,
}

impl AppConfig {
    pub fn load() -> Self {
        let config_path = Path::new("config.json");
        
        // Try reading from config.json first
        if config_path.exists() {
            if let Ok(content) = fs::read_to_string(config_path) {
                if let Ok(config) = serde_json::from_str::<AppConfig>(&content) {
                    // Create directories if they do not exist
                    let _ = fs::create_dir_all(&config.temp_dir);
                    let _ = fs::create_dir_all(&config.output_dir);
                    return config;
                }
            }
        }

        // Fallback to environment variables
        let token = std::env::var("CLI_CONTROLLER_TOKEN")
            .unwrap_or_else(|_| "my-secret-lan-token".to_string());
        
        let port = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(8080);
            
        let temp_dir = std::env::var("TEMP_DIR")
            .unwrap_or_else(|_| "./temp_uploads".to_string());
            
        let output_dir = std::env::var("OUTPUT_DIR")
            .unwrap_or_else(|_| "./outputs".to_string());

        let openai_api_key = std::env::var("OPENAI_API_KEY").ok();

        let config = AppConfig {
            token,
            port,
            temp_dir,
            output_dir,
            openai_api_key,
        };

        // Create directories
        let _ = fs::create_dir_all(&config.temp_dir);
        let _ = fs::create_dir_all(&config.output_dir);

        // Optionally write it back for visibility
        if !config_path.exists() {
            let _ = fs::write(
                config_path,
                serde_json::to_string_pretty(&config).unwrap_or_default(),
            );
        }

        config
    }
}
