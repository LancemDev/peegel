mod clipboard;
mod config;
mod core;
mod crypto;
mod discovery;
mod pairing;
mod protocol;
mod server;

use std::sync::Arc;

use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Listener, Manager, State, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_notification::NotificationExt;

use crate::core::{Core, Snapshot};

type CoreState<'a> = State<'a, Arc<Core>>;

/// Passed when launched by the login autostart entry: stay in the tray.
const MINIMIZED_ARG: &str = "--minimized";

#[tauri::command]
fn get_state(core: CoreState) -> Snapshot {
    core.snapshot()
}

#[tauri::command]
fn new_pairing_code(core: CoreState) {
    core.rotate_pair_secret();
}

#[tauri::command]
fn set_paused(core: CoreState, paused: bool) {
    core.set_paused(paused);
}

#[tauri::command]
fn unpair(core: CoreState, id: String) {
    core.remove_device(&id);
}

#[tauri::command]
fn recopy(core: CoreState, id: String) -> Result<(), String> {
    core.recopy(&id).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn clear_history(core: CoreState) {
    core.clear_history();
}

#[tauri::command]
fn set_notify(core: CoreState, notify: bool) {
    core.set_notify_on_receive(notify);
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    let result = if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    };
    result.map_err(|e| e.to_string())
}

fn notify_received(app: &AppHandle, clip: &core::Clip) {
    let body = if clip.mime == clipboard::TEXT_MIME {
        let text = String::from_utf8_lossy(&clip.data);
        let mut preview: String = text.chars().take(120).collect();
        if text.chars().count() > 120 {
            preview.push('…');
        }
        preview
    } else {
        "Image copied to the clipboard".to_string()
    };
    let _ = app
        .notification()
        .builder()
        .title(format!("Copied from {}", clip.origin_name))
        .body(body)
        .show();
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Builds the tray menu, including the recent-clips history.
fn tray_menu(app: &AppHandle, core: &Arc<Core>) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(app, "open", "Open Peegel", true, None::<&str>)?;
    let recent = Submenu::with_id(app, "recent", "Recent clips", true)?;
    let clips = core.recent_for_menu(8);
    if clips.is_empty() {
        let empty = MenuItem::with_id(app, "recent:none", "No clips yet", false, None::<&str>)?;
        recent.append(&empty)?;
    } else {
        for (id, label) in clips {
            let item = MenuItem::with_id(app, format!("recent:{id}"), label, true, None::<&str>)?;
            recent.append(&item)?;
        }
    }
    let pause = CheckMenuItem::with_id(
        app,
        "pause",
        "Pause sync",
        true,
        core.paused(),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    Menu::with_items(
        app,
        &[
            &open,
            &recent,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )
}

fn rebuild_tray_menu(app: &AppHandle, core: &Arc<Core>) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    match tray_menu(app, core) {
        Ok(menu) => match tray.set_menu(Some(menu)) {
            Ok(()) => log::debug!("tray menu rebuilt"),
            Err(err) => log::warn!("could not update the tray menu: {err}"),
        },
        Err(err) => log::warn!("could not build the tray menu: {err}"),
    }
}

fn build_tray(app: &AppHandle, core: &Arc<Core>) -> tauri::Result<()> {
    let menu = tray_menu(app, core)?;

    TrayIconBuilder::with_id("main")
        .tooltip("Peegel")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .menu(&menu)
        .on_menu_event(move |app, event| {
            let id = event.id.as_ref();
            match id {
                "open" => show_window(app),
                "pause" => {
                    let core = app.state::<Arc<Core>>();
                    core.set_paused(!core.paused());
                }
                "quit" => app.exit(0),
                _ => {
                    if let Some(clip_id) = id.strip_prefix("recent:") {
                        if clip_id != "none" {
                            let core = app.state::<Arc<Core>>();
                            if let Err(err) = core.recopy(clip_id) {
                                log::warn!("could not copy the selected clip: {err:#}");
                            }
                        }
                    }
                }
            }
        })
        .build(app)?;

    // Keep the recent-clips list and the pause check in sync with the app state.
    let handle = app.clone();
    app.listen_any("state-changed", move |_| {
        let app = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            let core = app.state::<Arc<Core>>();
            rebuild_tray_menu(&app, &core);
        });
    });
    Ok(())
}

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show_window(app)))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![MINIMIZED_ARG]),
        ))
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let config_dir = match std::env::var_os("PEEGEL_CONFIG_DIR") {
                Some(dir) => dir.into(),
                None => app.path().app_config_dir()?,
            };
            let handle = app.handle().clone();
            let notify_handle = app.handle().clone();
            let core = Core::new(
                config::ConfigStore::new(config_dir),
                move || {
                    let _ = handle.emit("state-changed", ());
                },
                move |clip| notify_received(&notify_handle, clip),
            );
            app.manage(core.clone());

            let watcher_core = core.clone();
            let limits = clipboard::Limits {
                max_text_bytes: protocol::MAX_FRAME_BYTES - 1024,
                max_image_bytes: core.config().max_image_bytes,
            };
            clipboard::watch(limits, move |clip, initial| {
                watcher_core.local_clip_changed(clip, initial);
            });

            let server_core = core.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = server::run(server_core).await {
                    log::error!("server stopped: {err:#}");
                }
            });

            match discovery::Advertiser::start(&core.config()) {
                Ok(advertiser) => {
                    app.manage(advertiser);
                }
                Err(err) => log::error!("mDNS advertising failed: {err:#}"),
            }

            build_tray(app.handle(), &core)?;

            let minimized = std::env::args().any(|arg| arg == MINIMIZED_ARG);
            if !minimized {
                show_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            new_pairing_code,
            set_paused,
            unpair,
            recopy,
            clear_history,
            set_notify,
            get_autostart,
            set_autostart
        ])
        .run(tauri::generate_context!())
        .expect("error while running Peegel");
}
