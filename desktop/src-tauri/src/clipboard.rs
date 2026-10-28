//! System clipboard access.
//!
//! On Wayland the compositor's data-control protocol (`ext-data-control` or
//! `wlr-data-control`) is used to get notified of every change without
//! polling. Elsewhere (X11, or compositors without data-control) we poll.

use std::{
    io::Read,
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

use anyhow::{anyhow, Result};
use wl_clipboard_rs::{copy, paste, watch};

pub const TEXT_MIME: &str = "text/plain";

const WAYLAND_TEXT_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];
/// Password managers (KeePassXC, KDE) tag secrets with this so they are not synced.
const PASSWORD_HINT_MIME: &str = "x-kde-passwordManagerHint";

#[derive(Debug, Clone)]
pub struct LocalClip {
    pub mime: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Backend {
    Wayland,
    Polling,
}

static BACKEND: OnceLock<Backend> = OnceLock::new();
static ARBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

/// Starts watching the clipboard on a background thread. `on_change` is called
/// with `initial = true` for whatever is already on the clipboard at startup.
pub fn watch<F>(max_bytes: usize, on_change: F)
where
    F: FnMut(LocalClip, bool) + Send + 'static,
{
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let watcher = if wayland {
        match watch::Watcher::new(watch::ClipboardType::Regular, paste::Seat::Unspecified) {
            Ok(watcher) => Some(watcher),
            Err(err) => {
                log::warn!("Wayland data-control unavailable ({err}), falling back to polling");
                None
            }
        }
    } else {
        None
    };

    match watcher {
        Some(watcher) => {
            let _ = BACKEND.set(Backend::Wayland);
            log::info!("clipboard backend: wayland data-control");
            thread::Builder::new()
                .name("clipboard-watch".into())
                .spawn(move || wayland_loop(watcher, max_bytes, on_change))
                .expect("spawn clipboard thread");
        }
        None => {
            let _ = BACKEND.set(Backend::Polling);
            log::info!("clipboard backend: polling");
            thread::Builder::new()
                .name("clipboard-poll".into())
                .spawn(move || polling_loop(max_bytes, on_change))
                .expect("spawn clipboard thread");
        }
    }
}

fn wayland_loop<F>(mut watcher: watch::Watcher, max_bytes: usize, mut on_change: F)
where
    F: FnMut(LocalClip, bool),
{
    let mut initial = true;
    loop {
        let event = match watcher.next_event() {
            Ok(Some(event)) => event,
            Ok(None) => return,
            Err(err) => {
                log::error!("clipboard watcher stopped: {err}");
                return;
            }
        };
        let watch::ClipboardEvent::Changed {
            mime_types,
            mut offer,
            ..
        } = event
        else {
            continue;
        };
        if mime_types.iter().any(|m| m == PASSWORD_HINT_MIME) {
            log::debug!("skipping clipboard entry marked as a password");
            initial = false;
            continue;
        }
        let Some(source_mime) = WAYLAND_TEXT_MIMES
            .iter()
            .find(|m| mime_types.iter().any(|offered| offered == *m))
        else {
            initial = false;
            continue;
        };
        let data = offer
            .receive(source_mime)
            .map_err(anyhow::Error::from)
            .and_then(|pipe| read_limited(pipe, max_bytes));
        match data {
            Ok(data) if !data.is_empty() => on_change(
                LocalClip {
                    mime: TEXT_MIME.into(),
                    data,
                },
                initial,
            ),
            Ok(_) => {}
            Err(err) => log::warn!("could not read clipboard: {err:#}"),
        }
        initial = false;
    }
}

fn read_limited(reader: impl Read, max_bytes: usize) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    reader
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut data)?;
    if data.len() > max_bytes {
        return Err(anyhow!("clipboard entry larger than {max_bytes} bytes"));
    }
    Ok(data)
}

fn polling_loop<F>(max_bytes: usize, mut on_change: F)
where
    F: FnMut(LocalClip, bool),
{
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(clipboard) => clipboard,
        Err(err) => {
            log::error!("no clipboard available: {err}");
            return;
        }
    };
    let mut last: Option<String> = None;
    let mut initial = true;
    loop {
        if let Ok(text) = clipboard.get_text() {
            if last.as_deref() != Some(text.as_str()) {
                if !text.is_empty() && text.len() <= max_bytes {
                    on_change(
                        LocalClip {
                            mime: TEXT_MIME.into(),
                            data: text.clone().into_bytes(),
                        },
                        initial,
                    );
                }
                last = Some(text);
            }
        }
        initial = false;
        thread::sleep(Duration::from_millis(600));
    }
}

/// Replaces the system clipboard contents.
pub fn set(clip: &LocalClip) -> Result<()> {
    match BACKEND.get().copied().unwrap_or(Backend::Polling) {
        Backend::Wayland => {
            let mime = if clip.mime == TEXT_MIME {
                copy::MimeType::Text
            } else {
                copy::MimeType::Specific(clip.mime.clone())
            };
            copy::Options::new().copy(
                copy::Source::Bytes(clip.data.clone().into_boxed_slice()),
                mime,
            )?;
        }
        Backend::Polling => {
            let mut guard = ARBOARD.lock().unwrap();
            if guard.is_none() {
                *guard = Some(arboard::Clipboard::new()?);
            }
            let clipboard = guard.as_mut().unwrap();
            if clip.mime == TEXT_MIME {
                clipboard.set_text(String::from_utf8_lossy(&clip.data).into_owned())?;
            } else {
                return Err(anyhow!("unsupported clipboard type {}", clip.mime));
            }
        }
    }
    Ok(())
}
