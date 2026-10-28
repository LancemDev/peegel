//! System clipboard access.
//!
//! On Wayland the compositor's data-control protocol (`ext-data-control` or
//! `wlr-data-control`) is used to get notified of every change without
//! polling. Elsewhere (X11, or compositors without data-control) we poll.

use std::{
    borrow::Cow,
    io::{Cursor, Read},
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

use anyhow::{anyhow, Result};
use wl_clipboard_rs::{copy, paste, watch};

pub const TEXT_MIME: &str = "text/plain";
pub const PNG_MIME: &str = "image/png";
pub const JPEG_MIME: &str = "image/jpeg";

const WAYLAND_TEXT_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];
const IMAGE_MIMES: &[&str] = &[PNG_MIME, JPEG_MIME];
/// Password managers (KeePassXC, KDE) tag secrets with this so they are not synced.
const PASSWORD_HINT_MIME: &str = "x-kde-passwordManagerHint";

#[derive(Debug, Clone)]
pub struct LocalClip {
    pub mime: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_text_bytes: usize,
    pub max_image_bytes: usize,
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
pub fn watch<F>(limits: Limits, on_change: F)
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
                .spawn(move || wayland_loop(watcher, limits, on_change))
                .expect("spawn clipboard thread");
        }
        None => {
            let _ = BACKEND.set(Backend::Polling);
            log::info!("clipboard backend: polling");
            thread::Builder::new()
                .name("clipboard-poll".into())
                .spawn(move || polling_loop(limits, on_change))
                .expect("spawn clipboard thread");
        }
    }
}

/// Picks what to read from an offer. Text wins when both are offered (e.g.
/// spreadsheet cells come with a rendered image too); image-only offers such
/// as screenshots or "Copy image" in a browser are read as images.
fn choose_mime(offered: &[String]) -> Option<(&'static str, &'static str)> {
    let has = |mime: &str| offered.iter().any(|m| m == mime);
    if let Some(mime) = WAYLAND_TEXT_MIMES.iter().find(|m| has(m)) {
        return Some((mime, TEXT_MIME));
    }
    IMAGE_MIMES
        .iter()
        .find(|m| has(m))
        .map(|mime| (*mime, *mime))
}

fn wayland_loop<F>(mut watcher: watch::Watcher, limits: Limits, mut on_change: F)
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
        let Some((source_mime, mime)) = choose_mime(&mime_types) else {
            initial = false;
            continue;
        };
        let max = if mime == TEXT_MIME {
            limits.max_text_bytes
        } else {
            limits.max_image_bytes
        };
        let data = offer
            .receive(source_mime)
            .map_err(anyhow::Error::from)
            .and_then(|pipe| read_limited(pipe, max));
        match data {
            Ok(data) if !data.is_empty() => on_change(
                LocalClip {
                    mime: mime.into(),
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

fn polling_loop<F>(limits: Limits, mut on_change: F)
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
    let mut last_text: Option<String> = None;
    let mut last_image: Option<u64> = None;
    let mut initial = true;
    loop {
        if let Ok(text) = clipboard.get_text() {
            if last_text.as_deref() != Some(text.as_str()) {
                if !text.is_empty() && text.len() <= limits.max_text_bytes {
                    on_change(
                        LocalClip {
                            mime: TEXT_MIME.into(),
                            data: text.clone().into_bytes(),
                        },
                        initial,
                    );
                }
                last_text = Some(text);
                last_image = None;
            }
        } else if let Ok(image) = clipboard.get_image() {
            let fingerprint = quick_fingerprint(&image.bytes);
            if last_image != Some(fingerprint) {
                last_image = Some(fingerprint);
                last_text = None;
                match encode_png(image.width as u32, image.height as u32, &image.bytes) {
                    Ok(png) if png.len() <= limits.max_image_bytes => on_change(
                        LocalClip {
                            mime: PNG_MIME.into(),
                            data: png,
                        },
                        initial,
                    ),
                    Ok(_) => log::info!("skipping image larger than the size limit"),
                    Err(err) => log::warn!("could not encode clipboard image: {err:#}"),
                }
            }
        }
        initial = false;
        thread::sleep(Duration::from_millis(600));
    }
}

fn quick_fingerprint(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.len().hash(&mut hasher);
    bytes.iter().step_by(97).for_each(|b| b.hash(&mut hasher));
    hasher.finish()
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let buffer = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| anyhow!("invalid image buffer"))?;
    let mut out = Cursor::new(Vec::new());
    buffer.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
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
                let rgba = image::load_from_memory(&clip.data)?.to_rgba8();
                clipboard.set_image(arboard::ImageData {
                    width: rgba.width() as usize,
                    height: rgba.height() as usize,
                    bytes: Cow::Owned(rgba.into_raw()),
                })?;
            }
        }
    }
    Ok(())
}

/// Small PNG preview for the history list, as a data URL, plus the original size.
pub fn thumbnail(data: &[u8]) -> Option<(String, u32, u32)> {
    let image = image::load_from_memory(data).ok()?;
    let (width, height) = (image.width(), image.height());
    let thumb = image.thumbnail(160, 160);
    let mut out = Cursor::new(Vec::new());
    thumb.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some((
        format!(
            "data:image/png;base64,{}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, out.get_ref())
        ),
        width,
        height,
    ))
}
