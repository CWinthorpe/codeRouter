#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri application entry point for CodeRouter.
//!
//! Sets up the system tray, spawns the proxy sidecar process, polls the proxy
//! health endpoint, and registers all IPC command handlers for the frontend.

mod commands;

use commands::{AppState, kill_sidecar, spawn_sidecar};
use coderouter_proxy::config::store::load_app_config;
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

type TrayMenuResult = Result<(Menu<tauri::Wry>, MenuItem<tauri::Wry>, MenuItem<tauri::Wry>), String>;

/// Returns the raw PNG bytes for the system tray icon.
///
/// Selects the active icon when the proxy is running and the inactive icon otherwise.
fn load_icon_bytes(running: bool) -> &'static [u8] {
    if running {
        include_bytes!("../icons/tray-active.png")
    } else {
        include_bytes!("../icons/tray-inactive.png")
    }
}

/// Decodes a PNG byte slice into raw RGBA pixel data.
///
/// # Returns
/// `Some((width, height, rgba_bytes))` on success, `None` if decoding fails.
fn decode_png_to_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    let width = info.width;
    let height = info.height;
    // RGBA needs 4 bytes per pixel
    let buf_size = (width as usize) * (height as usize) * 4;
    let mut buf = vec![0u8; buf_size];
    reader.next_frame(&mut buf).ok()?;
    Some((width, height, buf))
}

/// Creates a Tauri-compatible image for the system tray icon.
///
/// # Returns
/// `Some(Image)` when the icon bytes decode successfully, `None` otherwise.
fn make_icon(running: bool) -> Option<tauri::image::Image<'static>> {
    let (w, h, rgba) = decode_png_to_rgba(load_icon_bytes(running))?;
    Some(tauri::image::Image::new_owned(rgba, w, h))
}

/// Updates the system tray icon to reflect the current proxy state.
///
/// Looks up the tray by its `"main_tray"` id and swaps the icon between the
/// active (green) and inactive (grey) variants.
pub(crate) fn update_tray_icon(app: &tauri::AppHandle, running: bool) {
    if let Some(tray) = app.tray_by_id("main_tray") {
        if let Some(icon) = make_icon(running) {
            let _ = tray.set_icon(Some(icon));
        }
    }
}

/// Updates the proxy status label and toggle action text in the tray menu.
///
/// Shows "Proxy: Running/Stopped" on the status item and "Stop Proxy/Start Proxy"
/// on the toggle item depending on the current state.
pub(crate) fn update_menu_labels(state: &AppState, running: bool) {
    let _ = state.proxy_status_item.set_text(if running { "Proxy: Running" } else { "Proxy: Stopped" });
    let _ = state.toggle_proxy_item.set_text(if running { "Stop Proxy" } else { "Start Proxy" });
}

/// Builds the system tray context menu.
///
/// # Returns
/// A tuple of `(menu, proxy_status_item, toggle_proxy_item)` used to later
/// update the proxy status label and toggle action text at runtime.
///
/// # Menu layout
/// - Open CodeRouter
/// - ---
/// - Proxy: Running/Stopped (disabled)
/// - Start/Stop Proxy
/// - ---
/// - Configure OpenCode
/// - ---
/// - Quit
fn build_menu(app_handle: &tauri::AppHandle, running: bool) -> TrayMenuResult {
    let open_item = MenuItem::with_id(app_handle, "open_window", "Open CodeRouter", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let sep1 = PredefinedMenuItem::separator(app_handle).map_err(|e| e.to_string())?;
    let proxy_status_label = MenuItem::with_id(
        app_handle,
        "proxy_status",
        if running { "Proxy: Running" } else { "Proxy: Stopped" },
        false,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let toggle_proxy = MenuItem::with_id(
        app_handle,
        "toggle_proxy",
        if running { "Stop Proxy" } else { "Start Proxy" },
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let sep2 = PredefinedMenuItem::separator(app_handle).map_err(|e| e.to_string())?;
    let configure_opencode = MenuItem::with_id(app_handle, "configure_opencode", "Configure OpenCode", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let sep3 = PredefinedMenuItem::separator(app_handle).map_err(|e| e.to_string())?;
    let quit_item = MenuItem::with_id(app_handle, "quit", "Quit", true, None::<&str>)
        .map_err(|e| e.to_string())?;

    let menu = Menu::with_items(
        app_handle,
        &[
            &open_item,
            &sep1,
            &proxy_status_label,
            &toggle_proxy,
            &sep2,
            &configure_opencode,
            &sep3,
            &quit_item,
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok((menu, proxy_status_label, toggle_proxy))
}

/// Periodically polls the proxy health endpoint and updates tray state.
///
/// Every 5 seconds, sends a GET to `/health` on the configured proxy port.
/// Updates `proxy_running`, the tray icon, and menu labels based on whether
/// the proxy responds with a successful status.
async fn poll_health(app: tauri::AppHandle) {
    let client = reqwest::Client::new();
    let port = match load_app_config() {
        Ok(config) => config.proxy_port,
        Err(_) => 4141, // fallback to default port when config is missing
    };
    let health_url = format!("http://localhost:{port}/health");
    loop {
        let running = match client.get(&health_url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        };

        {
            let state = app.state::<AppState>();
            // Recover from a poisoned mutex by taking and unwrapping the inner value
            let mut proxy_running = state.proxy_running.lock().unwrap_or_else(|e| e.into_inner());
            *proxy_running = running;
        }

        update_tray_icon(&app, running);
        {
            let state = app.state::<AppState>();
            update_menu_labels(&state, running);
        }

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

/// Application entry point.
///
/// Initializes the metrics database, creates the Tauri window and system tray,
/// spawns the proxy sidecar, and registers all IPC command handlers.
/// Window close is intercepted so the app minimizes to tray instead of quitting.
fn main() {
    if let Err(e) = commands::init_metrics_db() {
        eprintln!("Warning: Failed to initialize metrics database: {}", e);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let app_handle = app.handle().clone();

            let inactive_icon = make_icon(false).expect("Failed to load tray icon");
            let (menu, proxy_status_item, toggle_proxy_item) = build_menu(&app_handle, false)?;

            app.manage(AppState {
                app_handle: app_handle.clone(),
                sidecar: Mutex::new(None),
                proxy_running: Mutex::new(false),
                proxy_status_item,
                toggle_proxy_item,
            });

            let _tray = TrayIconBuilder::with_id("main_tray")
                .icon(inactive_icon)
                .tooltip("CodeRouter")
                .menu(&menu)
                .on_menu_event(move |app, event| {
                    match event.id().as_ref() {
                        "open_window" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "toggle_proxy" => {
                            let state = app.state::<AppState>();
                            let running = *state.proxy_running.lock().unwrap();
                            if running {
                                // Stop the proxy: kill the sidecar process
                                let mut sidecar_guard = state.sidecar.lock().unwrap();
                                if let Some(child) = sidecar_guard.as_mut() {
                                    kill_sidecar(child);
                                }
                                *sidecar_guard = None;
                                *state.proxy_running.lock().unwrap() = false;
                            } else {
                                // Start the proxy: spawn a new sidecar process
                                match spawn_sidecar() {
                                    Ok(child) => {
                                        let mut sidecar_guard = state.sidecar.lock().unwrap();
                                        *sidecar_guard = Some(child);
                                        *state.proxy_running.lock().unwrap() = true;
                                    }
                                    Err(e) => eprintln!("Failed to start sidecar: {}", e),
                                }
                            }
                            let running = *state.proxy_running.lock().unwrap();
                            update_tray_icon(app, running);
                            update_menu_labels(&state, running);
                        }
                        "configure_opencode" => {
                            // Navigate the frontend to the OpenCode configuration page
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                                let _ = window.eval("window.location.hash = '#/opencode';");
                            }
                        }
                        "quit" => {
                            // Clean up: kill the sidecar before exiting
                            let state = app.state::<AppState>();
                            let mut sidecar_guard = state.sidecar.lock().unwrap();
                            if let Some(child) = sidecar_guard.as_mut() {
                                kill_sidecar(child);
                            }
                            *sidecar_guard = None;
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    // Left-click toggles window visibility (minimize to tray pattern)
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        ..
                    } = event
                    {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            if let Ok(visible) = window.is_visible() {
                                if visible {
                                    let _ = window.hide();
                                } else {
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                            }
                        }
                    }
                })
                .build(app)
                .expect("Failed to build tray");

            let state = app_handle.state::<AppState>();
            match spawn_sidecar() {
                Ok(child) => {
                    let mut sidecar_guard = state.sidecar.lock().unwrap();
                    *sidecar_guard = Some(child);
                    *state.proxy_running.lock().unwrap() = true;
                }
                Err(e) => eprintln!("Warning: Failed to spawn sidecar: {}", e),
            }

            // Start background health polling so the tray icon stays in sync
            let app_handle_clone = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                poll_health(app_handle_clone).await;
            });

            Ok(())
        })
        // Intercept window close to minimize to tray instead of quitting
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_providers,
            commands::save_provider,
            commands::toggle_provider_enabled,
            commands::delete_provider,
            commands::get_groups,
            commands::save_group,
            commands::delete_group,
            commands::get_app_config,
            commands::save_app_config,
            commands::test_provider_connection,
            commands::refresh_provider_models,
            commands::get_router_status,
            commands::set_entry_enabled,
            commands::get_daily_summary,
            commands::get_recent_requests,
            commands::get_usage_by_day,
            commands::get_usage_by_group,
            commands::get_opencode_config_path,
            commands::inject_opencode_provider,
            commands::remove_opencode_provider,
            commands::set_opencode_agent_models,
            commands::remove_opencode_agent_models,
            commands::get_opencode_agent_models,
            commands::preview_opencode_config,
            commands::clear_metrics_data,
            commands::reset_all_config,
            commands::restart_proxy,
            commands::is_group_referenced_in_opencode,
            commands::set_opencode_config_path,
            commands::get_latency_percentiles,
            commands::get_cost_summary,
            commands::get_app_version,
            commands::remove_coderouter_from_opencode,
            commands::dismiss_onboarding,
            commands::check_proxy_health,
            commands::check_for_updates,
            commands::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}