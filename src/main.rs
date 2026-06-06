// Hide the console window in release builds on Windows; the app lives in the
// system tray instead. Debug builds keep the console so logs are visible during dev.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use axum::{
    extract::{DefaultBodyLimit, Multipart},
    http::StatusCode,
    middleware,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing::{error, info};
use uuid::Uuid;
use base64::Engine;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Asset;


mod auth;
mod cli;
mod config;
mod registry;

// System tray runs only in Windows release builds (see Cargo.toml target deps).
#[cfg(all(windows, not(debug_assertions)))]
mod tray;

use auth::auth_middleware;
use config::AppConfig;
use registry::run_llm;

#[derive(Debug, Deserialize)]
struct ChatCompletionMessage {
    role: String,
    #[serde(default)]
    content: ValueOrArray,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
enum ValueOrArray {
    String(String),
    Array(Vec<MessageContentPart>),
}

impl Default for ValueOrArray {
    fn default() -> Self {
        ValueOrArray::String(String::new())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct MessageContentPart {
    #[serde(rename = "type")]
    part_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<ImageUrlPart>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ImageUrlPart {
    url: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatCompletionMessage>,
    #[serde(default = "default_timeout")]
    timeout: u64,
    #[serde(default)]
    cwd: Option<String>,
}

fn default_timeout() -> u64 {
    90
}

struct AppState {
    config: AppConfig,
}

/// Absolute path to the log file, used both by the logger and the tray "View Logs" action.
fn log_file_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("logs")
        .join("clicontroller.log")
}

/// Initialize logging to both stdout and `logs/clicontroller.log`.
/// Returns the appender guard, which must be kept alive for the program's lifetime.
fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt;
    use tracing_subscriber::prelude::*;

    let _ = fs::create_dir_all("logs");
    // A single fixed file (no date suffix) keeps the "View Logs" path predictable.
    let file_appender = tracing_appender::rolling::never("logs", "clicontroller.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(LevelFilter::INFO)
        .with(fmt::layer().with_writer(std::io::stdout))
        .with(fmt::layer().with_ansi(false).with_writer(file_writer))
        .init();

    guard
}

/// Build the full Axum router (API + static + frontend routes).
fn build_router(state: Arc<AppState>, config: &AppConfig) -> Router {
    let cors = CorsLayer::permissive();

    // API routes requiring Bearer Token Auth
    let api_routes = Router::new()
        .route("/v1/chat/completions", post(handle_chat_completion))
        .route("/api/upload", post(handle_upload))
        .route("/api/providers", get(handle_list_providers))
        .layer(middleware::from_fn({
            let token = config.token.clone();
            move |req, next| auth_middleware(req, next, token.clone())
        }))
        .with_state(state.clone());

    // Public / static routes
    Router::new()
        .merge(api_routes)
        .route("/static/*path", get(serve_static))
        .nest_service("/outputs", ServeDir::new(&config.output_dir))
        // Serve frontend home
        .route("/", get(serve_index))
        .route("/chat", get(serve_chat))
        .route("/api/guide", get(handle_guide))
        // Agent-readable orchestration protocol (markdown) + downloadable helper
        .route("/agents", get(serve_agents))
        .route("/consult.py", get(serve_consult_py))
        .layer(cors)
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024)) // 20 MB limit for file uploads
}

/// Bind and serve the application. Runs forever.
async fn serve(app: Router, port: u16) {
    match tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await {
        Ok(listener) => {
            info!("Server listening on http://0.0.0.0:{port}");
            if let Err(e) = axum::serve(listener, app).await {
                error!("Server error: {e}");
            }
        }
        Err(e) => {
            error!("Failed to bind port {port} (is it already in use?): {e}");
        }
    }
}

fn main() {
    // Keep the logging guard alive for the whole process.
    let _log_guard = init_logging();

    for file in Asset::iter() {
        info!("Embedded file: {}", file);
    }

    // Load config
    let config = AppConfig::load();

    // Set OpenAI API key if configured
    if let Some(key) = &config.openai_api_key {
        if !key.is_empty() {
            info!("Setting OPENAI_API_KEY from configuration");
            std::env::set_var("OPENAI_API_KEY", key);
        }
    }

    info!("Starting KJ CLIController on port {}", config.port);

    let _ = fs::create_dir_all(&config.temp_dir);
    let _ = fs::create_dir_all(&config.output_dir);

    let state = Arc::new(AppState { config: config.clone() });
    let app = build_router(state, &config);
    let port = config.port;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");

    // Windows release: run the server on a background thread and the system-tray
    // event loop on the main thread (the tray/event loop must own the main thread).
    #[cfg(all(windows, not(debug_assertions)))]
    {
        std::thread::spawn(move || {
            rt.block_on(serve(app, port));
        });
        tray::run_tray(port, log_file_path());
    }

    // Everywhere else (Linux/macOS, or Windows debug builds): just run the server.
    #[cfg(not(all(windows, not(debug_assertions))))]
    {
        let _ = log_file_path; // silence unused-fn warning on these targets
        rt.block_on(serve(app, port));
    }
}

async fn serve_static(axum::extract::Path(path): axum::extract::Path<String>) -> impl axum::response::IntoResponse {
    let clean_path = match path.find('?') {
        Some(idx) => &path[..idx],
        None => &path,
    };
    
    match Asset::get(clean_path) {
        Some(content) => {
            let mime = mime_guess::from_path(clean_path).first_or_octet_stream();
            axum::response::Response::builder()
                .header("content-type", mime.as_ref())
                .body(axum::body::Body::from(content.data.into_owned()))
                .unwrap()
        }
        None => axum::response::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::Body::from("404 Asset Not Found"))
            .unwrap(),
    }
}

// Homepage: serve the marketing landing page.
async fn serve_index() -> impl axum::response::IntoResponse {
    serve_embedded_html("landing.html")
}

// Chat dashboard moved to /chat so the landing page can own "/".
async fn serve_chat() -> impl axum::response::IntoResponse {
    serve_embedded_html("index.html")
}

fn serve_embedded_html(name: &str) -> axum::response::Response {
    match Asset::get(name) {
        Some(content) => axum::response::Response::builder()
            .header("content-type", "text/html; charset=utf-8")
            .body(axum::body::Body::from(content.data.into_owned()))
            .unwrap(),
        None => axum::response::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::Body::from(format!("{name} not found")))
            .unwrap(),
    }
}

// The orchestration helper script, embedded at compile time so the gateway can serve it
// for download on any machine: `curl -O http://localhost:8080/consult.py`.
const CONSULT_PY: &str = include_str!("../skills/consult-local-clis/scripts/consult.py");

// Agent-readable orchestration protocol (raw markdown). Served at /agents — a short,
// memorable URL to hand to a desktop agent ("read localhost:8080/agents and apply it").
async fn serve_agents() -> impl axum::response::IntoResponse {
    match Asset::get("agents.md") {
        Some(content) => axum::response::Response::builder()
            .header("content-type", "text/plain; charset=utf-8")
            .body(axum::body::Body::from(content.data.into_owned()))
            .unwrap(),
        None => axum::response::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::Body::from("agents.md not found"))
            .unwrap(),
    }
}

async fn serve_consult_py() -> impl axum::response::IntoResponse {
    axum::response::Response::builder()
        .header("content-type", "text/x-python; charset=utf-8")
        .body(axum::body::Body::from(CONSULT_PY))
        .unwrap()
}

async fn handle_guide() -> impl axum::response::IntoResponse {
    match Asset::get("guide.html") {
        Some(content) => axum::response::Response::builder()
            .header("content-type", "text/html; charset=utf-8")
            .body(axum::body::Body::from(content.data.into_owned()))
            .unwrap(),
        None => axum::response::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::Body::from("guide.html not found"))
            .unwrap(),
    }
}

async fn handle_list_providers() -> Json<serde_json::Value> {
    let providers = registry::list_providers().await;
    Json(json!(providers))
}

async fn handle_upload(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut file_path = String::new();
    let mut filename_out = String::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            let filename = field.file_name().unwrap_or("upload").to_string();
            // Sanitize filename
            let safe_filename: String = filename
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
                .collect();
            
            let file_uuid = Uuid::new_v4().simple().to_string();
            let dest_filename = format!("{}_{}", file_uuid, safe_filename);
            let dest_path = PathBuf::from(&state.config.temp_dir).join(&dest_filename);
            
            let data = field.bytes().await.map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("Failed to read upload data: {}", e)})),
                )
            })?;

            fs::write(&dest_path, &data).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("Failed to write file to disk: {}", e)})),
                )
            })?;

            file_path = crate::cli::clean_path(&dest_path.canonicalize().unwrap_or(dest_path));
            filename_out = dest_filename;
            break;
        }
    }

    if file_path.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "No file field found in request"})),
        ));
    }

    Ok(Json(json!({
        "path": file_path,
        "filename": filename_out
    })))
}

fn detect_cwd(attachments: &[String], payload_cwd: Option<String>) -> Option<String> {
    if let Some(dir) = payload_cwd {
        if !dir.is_empty() {
            return Some(dir);
        }
    }
    if let Some(first) = attachments.first() {
        let path = Path::new(first);
        if let Some(parent) = path.parent() {
            if let Ok(abs_parent) = parent.canonicalize() {
                return Some(crate::cli::clean_path(&abs_parent));
            }
            return Some(crate::cli::clean_path(parent));
        }
    }
    None
}

async fn handle_chat_completion(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<ChatCompletionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    info!("Received chat completion request for model: {}", payload.model);

    // 1. Flatten messages into prompts
    let mut system_prompt = String::new();
    let mut user_prompt_parts = Vec::new();
    let mut attachments = Vec::new();
    let mut temp_files_to_cleanup = Vec::new();

    for msg in payload.messages {
        match msg.role.as_str() {
            "system" => {
                match msg.content {
                    ValueOrArray::String(s) => {
                        if !system_prompt.is_empty() {
                            system_prompt.push('\n');
                        }
                        system_prompt.push_str(&s);
                    }
                    ValueOrArray::Array(arr) => {
                        for part in arr {
                            if part.part_type == "text" {
                                if let Some(t) = part.text {
                                    if !system_prompt.is_empty() {
                                        system_prompt.push('\n');
                                    }
                                    system_prompt.push_str(&t);
                                }
                            }
                        }
                    }
                }
            }
            "user" | "assistant" => {
                // If it's a model request, we compile user messages into the user prompt
                let prefix = if msg.role == "assistant" { "Assistant: " } else { "" };
                match msg.content {
                    ValueOrArray::String(s) => {
                        user_prompt_parts.push(format!("{}{}", prefix, s));
                    }
                    ValueOrArray::Array(arr) => {
                        for part in arr {
                            if part.part_type == "text" {
                                if let Some(t) = part.text {
                                    user_prompt_parts.push(format!("{}{}", prefix, t));
                                }
                            } else if part.part_type == "image_url" {
                                if let Some(img_part) = part.image_url {
                                    // Parse data URL: data:image/png;base64,iVBOR...
                                    if img_part.url.starts_with("data:image/") {
                                        if let Some(comma_pos) = img_part.url.find(',') {
                                            let meta = &img_part.url[..comma_pos];
                                            let data_b64 = &img_part.url[comma_pos + 1..];
                                            
                                            // Guess extension
                                            let ext = if meta.contains("png") {
                                                "png"
                                            } else if meta.contains("gif") {
                                                "gif"
                                            } else if meta.contains("webp") {
                                                "webp"
                                            } else {
                                                "jpg"
                                            };
                                            
                                            if let Ok(decoded_bytes) = base64::engine::general_purpose::STANDARD.decode(data_b64.trim()) {
                                                let temp_filename = format!("b64_upload_{}.{}", Uuid::new_v4().simple(), ext);
                                                let temp_filepath = PathBuf::from(&state.config.temp_dir).join(&temp_filename);
                                                if fs::write(&temp_filepath, &decoded_bytes).is_ok() {
                                                    let abs_path = crate::cli::clean_path(&temp_filepath.canonicalize().unwrap_or(temp_filepath));
                                                    attachments.push(abs_path.clone());
                                                    temp_files_to_cleanup.push(abs_path);
                                                }
                                            }
                                        }
                                    } else {
                                        // Remote image URL or local server absolute path
                                        attachments.push(img_part.url);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let user_prompt = user_prompt_parts.join("\n\n");
    let system_opt = if system_prompt.is_empty() { None } else { Some(system_prompt.as_str()) };

    // 2. Snapshot outputs/ directory before run
    let mut initial_output_files = HashSet::new();
    if let Ok(entries) = fs::read_dir(&state.config.output_dir) {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                initial_output_files.insert(name);
            }
        }
    }

    // Detect current working directory
    let cwd_opt = detect_cwd(&attachments, payload.cwd);

    // 3. Dispatch to CLI runner
    let run_res = run_llm(
        &payload.model,
        &user_prompt,
        system_opt,
        &attachments,
        payload.timeout,
        &state.config.temp_dir,
        cwd_opt.as_deref(),
    )
    .await;

    // Clean up temporary base64 image uploads immediately
    for path in temp_files_to_cleanup {
        let _ = fs::remove_file(path);
    }

    let mut response_text = match run_res {
        Ok(text) => text,
        Err(e) => {
            error!("CLI completion error: {:?}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "message": e.to_string(),
                        "type": "cli_execution_error",
                        "param": null,
                        "code": null
                    }
                })),
            ));
        }
    };

    // 4. Scan outputs/ directory for new files generated by the run
    let mut new_output_files = Vec::new();
    if let Ok(entries) = fs::read_dir(&state.config.output_dir) {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                if !initial_output_files.contains(&name) {
                    // It is a new file! Isolate with unique prefix to avoid races
                    let file_uuid = Uuid::new_v4().simple().to_string();
                    let new_name = format!("{}_{}", file_uuid, name);
                    let old_path = entry.path();
                    let new_path = PathBuf::from(&state.config.output_dir).join(&new_name);
                    
                    if fs::rename(&old_path, &new_path).is_ok() {
                        new_output_files.push(new_name);
                    } else {
                        // Fallback to old name if rename failed
                        new_output_files.push(name);
                    }
                }
            }
        }
    }

    // Append newly found outputs to response text
    if !new_output_files.is_empty() {
        let mut append_block = String::from("\n\n### Generated Outputs:\n");
        for file in new_output_files {
            let is_image = file.ends_with(".png") || file.ends_with(".jpg") || file.ends_with(".jpeg") || file.ends_with(".gif") || file.ends_with(".webp");
            if is_image {
                append_block.push_str(&format!("![Generated Output](/outputs/{})\n", file));
            } else {
                append_block.push_str(&format!("- [Download Generated File](/outputs/{})\n", file));
            }
        }
        response_text.push_str(&append_block);
    }

    // 5. Wrap response in standard OpenAI JSON format
    let completion_id = format!("chatcmpl-{}", Uuid::new_v4().simple());
    let created_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let openai_response = json!({
        "id": completion_id,
        "object": "chat.completion",
        "created": created_time,
        "model": payload.model,
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": response_text
                },
                "finish_reason": "stop"
            }
        ],
        "usage": {
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0
        }
    });

    Ok(Json(openai_response))
}
