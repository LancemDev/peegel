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
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Listener, Manager, State, WindowEvent,
};

use crate::core::{Core, Snapshot};

type CoreState<'a> = State<'a, Arc<Core>>;

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

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &AppHandle, core: &Arc<Core>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Peegel", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(
        app,
        "pause",
        "Pause sync",
        true,
        core.paused(),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &pause, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    TrayIconBuilder::with_id("main")
        .tooltip("Peegel")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "pause" => {
                let core = app.state::<Arc<Core>>();
                core.set_paused(!core.paused());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    // Keep the tray checkbox in sync with the toggle in the window.
    let tray_pause = pause;
    let handle = app.clone();
    app.listen_any("state-changed", move |_| {
        let paused = handle.state::<Arc<Core>>().paused();
        let _ = tray_pause.set_checked(paused);
    });
    Ok(())
}

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show_window(app)))
        .setup(|app| {
            let config_dir = match std::env::var_os("PEEGEL_CONFIG_DIR") {
                Some(dir) => dir.into(),
                None => app.path().app_config_dir()?,
            };
            let handle = app.handle().clone();
            let core = Core::new(config::ConfigStore::new(config_dir), move || {
                let _ = handle.emit("state-changed", ());
            });
            app.manage(core.clone());

            let watcher_core = core.clone();
            clipboard::watch(protocol::MAX_FRAME_BYTES - 1024, move |clip, initial| {
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

            if core.config().devices.is_empty() {
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
            clear_history
        ])
        .run(tauri::generate_context!())
        .expect("error while running Peegel");
}
