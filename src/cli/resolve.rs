use std::env;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    let paths = env::split_paths(&path_var);
    for path in paths {
        let exe_path = path.join(name);
        if cfg!(windows) {
            for ext in &["exe", "cmd", "bat"] {
                let with_ext = exe_path.with_extension(ext);
                if with_ext.is_file() {
                    return Some(with_ext);
                }
            }
        }
        if exe_path.is_file() {
            return Some(exe_path);
        }
    }
    None
}

pub fn get_windows_npm_paths(cli_name: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    
    if let Ok(appdata) = env::var("APPDATA") {
        paths.push(Path::new(&appdata).join("npm").join(format!("{}.cmd", cli_name)));
    }
    if let Ok(userprofile) = env::var("USERPROFILE") {
        paths.push(Path::new(&userprofile)
            .join("AppData")
            .join("Roaming")
            .join("npm")
            .join(format!("{}.cmd", cli_name)));
    }
    if let Some(user_profile) = env::var_os("USERPROFILE") {
        paths.push(PathBuf::from(user_profile)
            .join("AppData")
            .join("Roaming")
            .join("npm")
            .join(format!("{}.cmd", cli_name)));
    }
    
    paths
}

pub fn resolve_cli_binary(cli_name: &str) -> String {
    // 1. Try PATH
    if let Some(path) = find_in_path(cli_name) {
        debug!("Resolved {} from PATH: {:?}", cli_name, path);
        return path.to_string_lossy().to_string();
    }

    // 2. Try Windows npm paths
    if cfg!(windows) {
        for npm_path in get_windows_npm_paths(cli_name) {
            if npm_path.exists() {
                let mut cmd = create_std_command(&npm_path.to_string_lossy());
                cmd.arg("--version");
                
                // Avoid console window flashing
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                }
                
                if let Ok(_child) = cmd.spawn() {
                    // Check briefly if spawned
                    info!("Resolved {} from npm: {:?}", cli_name, npm_path);
                    return npm_path.to_string_lossy().to_string();
                }

            }
        }
    }

    warn!("CLI {} not found on PATH or npm, falling back to name", cli_name);
    cli_name.to_string()
}

pub fn create_command(bin_path: &str) -> tokio::process::Command {
    if cfg!(windows) && (bin_path.to_lowercase().ends_with(".cmd") || bin_path.to_lowercase().ends_with(".bat")) {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/c").arg(bin_path);
        cmd
    } else {
        tokio::process::Command::new(bin_path)
    }
}

pub fn create_std_command(bin_path: &str) -> std::process::Command {
    if cfg!(windows) && (bin_path.to_lowercase().ends_with(".cmd") || bin_path.to_lowercase().ends_with(".bat")) {
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/c").arg(bin_path);
        cmd
    } else {
        std::process::Command::new(bin_path)
    }
}
