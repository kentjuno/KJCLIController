//! Windows system-tray integration.
//!
//! The HTTP server runs on a background thread; this module owns the main thread,
//! showing a tray icon with a menu to open the UI in a browser, view live logs, or
//! quit. Compiled only for Windows release builds (see `Cargo.toml` target deps and
//! the `cfg` gate on the `mod tray;` declaration).

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use tracing::{error, info};

use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIconBuilder};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Name of the HKCU\...\Run value used for "start with Windows".
const RUN_VALUE_NAME: &str = "KJ CLIController";
const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Whether this app is registered to launch at user login.
fn startup_is_enabled() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.open_subkey(RUN_KEY_PATH) {
        Ok(run) => run.get_value::<String, _>(RUN_VALUE_NAME).is_ok(),
        Err(_) => false,
    }
}

/// Enable or disable launch at login by writing/removing the HKCU Run value.
/// When enabling, the value is set to the quoted path of the current executable.
fn set_startup(enabled: bool) -> std::io::Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if enabled {
        let exe = std::env::current_exe()?;
        let quoted = format!("\"{}\"", exe.display());
        let (run, _) = hkcu.create_subkey(RUN_KEY_PATH)?;
        run.set_value(RUN_VALUE_NAME, &quoted)?;
    } else {
        // Opening with write access; ignore "not found" when the value is already absent.
        if let Ok(run) = hkcu.open_subkey_with_flags(RUN_KEY_PATH, KEY_WRITE) {
            let _ = run.delete_value(RUN_VALUE_NAME);
        }
    }
    Ok(())
}

/// Open a URL in the user's default browser without flashing a console window.
fn open_url(url: &str) {
    let _ = Command::new("cmd")
        .args(["/C", "start", "", url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

/// Open a new console window that live-tails the log file.
fn open_logs(log_path: &PathBuf) {
    let path = log_path.display().to_string();
    let _ = Command::new("powershell")
        .args([
            "-NoExit",
            "-NoProfile",
            "-Command",
            &format!("Get-Content -LiteralPath '{path}' -Wait -Tail 200"),
        ])
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn();
}

/// Build a simple 32x32 RGBA icon: a filled circle with a cyan→emerald gradient.
fn build_icon() -> Option<Icon> {
    const SIZE: u32 = 32;
    let size_f = SIZE as f32;
    let center = (size_f - 1.0) / 2.0;
    let radius = size_f / 2.0;

    // cyan #06b6d4 -> emerald #10b981
    let (r0, g0, b0) = (6.0, 182.0, 212.0);
    let (r1, g1, b1) = (16.0, 185.0, 129.0);

    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > radius {
                rgba.extend_from_slice(&[0, 0, 0, 0]); // transparent outside the circle
                continue;
            }
            let t = (x as f32 + y as f32) / (2.0 * (size_f - 1.0));
            let r = (r0 + (r1 - r0) * t) as u8;
            let g = (g0 + (g1 - g0) * t) as u8;
            let b = (b0 + (b1 - b0) * t) as u8;
            // Soft 1px edge so the circle isn't jagged.
            let alpha = if dist > radius - 1.0 { 180 } else { 255 };
            rgba.extend_from_slice(&[r, g, b, alpha]);
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).ok()
}

/// Run the tray event loop. Blocks (the `tao` event loop owns the thread).
pub fn run_tray(port: u16, log_path: PathBuf) {
    let event_loop = EventLoopBuilder::new().build();

    let menu = Menu::new();
    let title = MenuItem::new(format!("KJ CLIController · :{port}"), false, None);
    let open_landing = MenuItem::new("Open Landing Page", true, None);
    let open_dashboard = MenuItem::new("Open Dashboard", true, None);
    let view_logs = MenuItem::new("View Logs", true, None);

    // Settings submenu with a checkable "Start with Windows" toggle, initialized
    // from the current registry state so the checkmark reflects reality on launch.
    let start_with_windows =
        CheckMenuItem::new("Start with Windows", true, startup_is_enabled(), None);
    let settings = Submenu::new("Settings", true);
    let _ = settings.append(&start_with_windows);

    let quit = MenuItem::new("Quit", true, None);

    let _ = menu.append_items(&[
        &title,
        &PredefinedMenuItem::separator(),
        &open_landing,
        &open_dashboard,
        &view_logs,
        &PredefinedMenuItem::separator(),
        &settings,
        &PredefinedMenuItem::separator(),
        &quit,
    ]);

    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(format!("KJ CLIController — running on http://localhost:{port}"));
    if let Some(icon) = build_icon() {
        builder = builder.with_icon(icon);
    }
    // Held for the lifetime of the process (the diverging event loop never drops it).
    let _tray = builder.build().expect("failed to build tray icon");

    // Cache menu ids for comparison inside the loop.
    let id_landing = open_landing.id().clone();
    let id_dashboard = open_dashboard.id().clone();
    let id_logs = view_logs.id().clone();
    let id_startup = start_with_windows.id().clone();
    let id_quit = quit.id().clone();

    let landing_url = format!("http://localhost:{port}/");
    let dashboard_url = format!("http://localhost:{port}/chat");

    let menu_channel = MenuEvent::receiver();

    event_loop.run(move |_event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Ok(event) = menu_channel.try_recv() {
            if event.id == id_landing {
                open_url(&landing_url);
            } else if event.id == id_dashboard {
                open_url(&dashboard_url);
            } else if event.id == id_logs {
                open_logs(&log_path);
            } else if event.id == id_startup {
                // muda has already toggled the visible checkmark; sync the registry
                // to match it. If the registry write fails, revert the checkmark so
                // the UI never lies about the actual startup state.
                let desired = start_with_windows.is_checked();
                match set_startup(desired) {
                    Ok(()) => info!(
                        "Start with Windows {}",
                        if desired { "enabled" } else { "disabled" }
                    ),
                    Err(e) => {
                        error!("Failed to update startup setting: {e}");
                        start_with_windows.set_checked(!desired);
                    }
                }
            } else if event.id == id_quit {
                std::process::exit(0);
            }
        }
    });
}
