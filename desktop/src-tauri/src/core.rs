//! Application state shared between the network server, the clipboard
//! watcher and the UI.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Result};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::{
    clipboard::{self, LocalClip},
    config::{Config, ConfigStore, PairedDevice},
    crypto, pairing,
};

const HISTORY_LEN: usize = 50;
const PREVIEW_CHARS: usize = 400;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct Clip {
    pub id: String,
    pub mime: String,
    pub data: Arc<Vec<u8>>,
    pub sha256: String,
    pub ts: u64,
    /// `None` when the clip was copied on this computer.
    pub origin_device: Option<String>,
    pub origin_name: String,
    /// Data URL preview and original dimensions, for images.
    pub thumb: Option<(String, u32, u32)>,
}

impl Clip {
    fn is_supported_mime(mime: &str) -> bool {
        matches!(
            mime,
            clipboard::TEXT_MIME | clipboard::PNG_MIME | clipboard::JPEG_MIME
        )
    }

    fn thumb_for(mime: &str, data: &[u8]) -> Option<(String, u32, u32)> {
        mime.starts_with("image/")
            .then(|| clipboard::thumbnail(data))
            .flatten()
    }
}

/// One-line label for the tray's recent-clips menu.
fn clip_menu_label(clip: &Clip) -> String {
    if clip.mime == clipboard::TEXT_MIME {
        let text: String = String::from_utf8_lossy(&clip.data)
            .chars()
            .take(256)
            .collect();
        let mut preview = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if preview.chars().count() > 48 {
            preview = preview.chars().take(47).collect::<String>() + "…";
        }
        if preview.is_empty() {
            "Empty text clip".to_string()
        } else {
            preview
        }
    } else {
        match &clip.thumb {
            Some((_, width, height)) => format!("Image · {width}×{height}"),
            None => "Image".to_string(),
        }
    }
}

#[derive(Serialize)]
pub struct DeviceView {
    id: String,
    name: String,
    connected: bool,
    paired_at: u64,
}

#[derive(Serialize)]
pub struct ClipView {
    id: String,
    mime: String,
    preview: String,
    size: usize,
    thumb: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    origin: String,
    from_phone: bool,
    ts: u64,
}

#[derive(Serialize)]
pub struct Snapshot {
    server_name: String,
    port: u16,
    paused: bool,
    notify_on_receive: bool,
    pair_uri: String,
    qr_svg: String,
    addresses: Vec<String>,
    devices: Vec<DeviceView>,
    history: Vec<ClipView>,
}

pub struct Core {
    store: ConfigStore,
    config: Mutex<Config>,
    pair_secret: Mutex<[u8; 32]>,
    connected: Mutex<HashMap<String, usize>>,
    history: Mutex<VecDeque<Clip>>,
    last_hash: Mutex<Option<String>>,
    outgoing: broadcast::Sender<Arc<Clip>>,
    on_change: Box<dyn Fn() + Send + Sync>,
    on_receive: Box<dyn Fn(&Clip) + Send + Sync>,
}

impl Core {
    pub fn new(
        store: ConfigStore,
        on_change: impl Fn() + Send + Sync + 'static,
        on_receive: impl Fn(&Clip) + Send + Sync + 'static,
    ) -> Arc<Self> {
        let config = store.load();
        let (outgoing, _) = broadcast::channel(16);
        Arc::new(Self {
            store,
            config: Mutex::new(config),
            pair_secret: Mutex::new(crypto::random_bytes()),
            connected: Mutex::new(HashMap::new()),
            history: Mutex::new(VecDeque::new()),
            last_hash: Mutex::new(None),
            outgoing,
            on_change: Box::new(on_change),
            on_receive: Box::new(on_receive),
        })
    }

    fn changed(&self) {
        (self.on_change)();
    }

    fn update_config(&self, f: impl FnOnce(&mut Config)) {
        let mut config = self.config.lock().unwrap();
        f(&mut config);
        if let Err(err) = self.store.save(&config) {
            log::error!("could not save config: {err:#}");
        }
    }

    pub fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Clip>> {
        self.outgoing.subscribe()
    }

    // --- pairing ---------------------------------------------------------

    pub fn pair_secret(&self) -> [u8; 32] {
        *self.pair_secret.lock().unwrap()
    }

    pub fn rotate_pair_secret(&self) {
        *self.pair_secret.lock().unwrap() = crypto::random_bytes();
        self.changed();
    }

    pub fn device_key(&self, device_id: &str) -> Option<[u8; 32]> {
        let config = self.config.lock().unwrap();
        let device = config.devices.iter().find(|d| d.id == device_id)?;
        crypto::unb64(&device.key).ok()?.try_into().ok()
    }

    pub fn is_paired(&self, device_id: &str) -> bool {
        self.config
            .lock()
            .unwrap()
            .devices
            .iter()
            .any(|d| d.id == device_id)
    }

    pub fn add_device(&self, id: &str, name: &str, key: &[u8; 32]) {
        self.update_config(|config| {
            config.devices.retain(|d| d.id != id);
            config.devices.push(PairedDevice {
                id: id.into(),
                name: name.into(),
                key: crypto::b64(key),
                paired_at: now_ms(),
            });
        });
        self.changed();
    }

    pub fn update_device_name(&self, id: &str, name: &str) {
        let needs_update = self
            .config
            .lock()
            .unwrap()
            .devices
            .iter()
            .any(|d| d.id == id && d.name != name);
        if needs_update {
            self.update_config(|config| {
                if let Some(device) = config.devices.iter_mut().find(|d| d.id == id) {
                    device.name = name.into();
                }
            });
        }
    }

    pub fn remove_device(&self, id: &str) {
        self.update_config(|config| config.devices.retain(|d| d.id != id));
        self.changed();
    }

    pub fn set_connected(&self, device_id: &str, connected: bool) {
        {
            let mut map = self.connected.lock().unwrap();
            let count = map.entry(device_id.to_string()).or_default();
            if connected {
                *count += 1;
            } else {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    map.remove(device_id);
                }
            }
        }
        self.changed();
    }

    pub fn set_paused(&self, paused: bool) {
        self.update_config(|config| config.paused = paused);
        self.changed();
    }

    pub fn set_notify_on_receive(&self, notify: bool) {
        self.update_config(|config| config.notify_on_receive = notify);
        self.changed();
    }

    pub fn paused(&self) -> bool {
        self.config.lock().unwrap().paused
    }

    // --- clips -----------------------------------------------------------

    /// Records a clip as current and returns `false` if it was already current.
    fn mark_current(&self, sha256: &str) -> bool {
        let mut last = self.last_hash.lock().unwrap();
        if last.as_deref() == Some(sha256) {
            return false;
        }
        *last = Some(sha256.to_string());
        true
    }

    fn push_history(&self, clip: Clip) {
        let mut history = self.history.lock().unwrap();
        history.retain(|c| c.sha256 != clip.sha256);
        history.push_front(clip);
        history.truncate(HISTORY_LEN);
    }

    /// Called by the clipboard watcher when something was copied on this computer.
    pub fn local_clip_changed(&self, local: LocalClip, initial: bool) {
        let sha256 = crypto::sha256_hex(&local.data);
        if !self.mark_current(&sha256) || initial {
            return;
        }
        let clip = Clip {
            id: uuid::Uuid::new_v4().to_string(),
            thumb: Clip::thumb_for(&local.mime, &local.data),
            mime: local.mime,
            data: Arc::new(local.data),
            sha256,
            ts: now_ms(),
            origin_device: None,
            origin_name: "This computer".into(),
        };
        self.push_history(clip.clone());
        if !self.paused() {
            let _ = self.outgoing.send(Arc::new(clip));
        }
        self.changed();
    }

    /// Called by a session when a phone sent a clip.
    pub fn remote_clip(
        &self,
        device_id: &str,
        device_name: &str,
        id: String,
        mime: String,
        sha256: String,
        body: &[u8],
    ) -> Result<()> {
        if !Clip::is_supported_mime(&mime) {
            bail!("unsupported mime type {mime}");
        }
        if mime.starts_with("image/") && body.len() > self.config().max_image_bytes {
            bail!("image larger than the size limit");
        }
        if crypto::sha256_hex(body) != sha256 {
            bail!("clip hash mismatch");
        }
        if self.paused() {
            return Ok(());
        }
        let clip = Clip {
            id,
            thumb: Clip::thumb_for(&mime, body),
            mime,
            data: Arc::new(body.to_vec()),
            sha256,
            ts: now_ms(),
            origin_device: Some(device_id.to_string()),
            origin_name: device_name.to_string(),
        };
        if self.mark_current(&clip.sha256) {
            clipboard::set(&LocalClip {
                mime: clip.mime.clone(),
                data: clip.data.to_vec(),
            })?;
            // Forward to any other connected phones.
            let _ = self.outgoing.send(Arc::new(clip.clone()));
            if self.config().notify_on_receive {
                (self.on_receive)(&clip);
            }
        }
        self.push_history(clip);
        self.changed();
        Ok(())
    }

    /// Puts a history entry back on the clipboard. The clipboard watcher then
    /// picks it up and syncs it like any other copy.
    pub fn recopy(&self, id: &str) -> Result<()> {
        let clip = self
            .history
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.id == id)
            .cloned();
        let Some(clip) = clip else {
            bail!("clip not found");
        };
        // Force the watcher to treat it as new so phones receive it again.
        *self.last_hash.lock().unwrap() = None;
        clipboard::set(&LocalClip {
            mime: clip.mime,
            data: clip.data.to_vec(),
        })
    }

    pub fn clear_history(&self) {
        self.history.lock().unwrap().clear();
        self.changed();
    }

    /// Short labels for the tray's recent-clips menu, newest first.
    pub fn recent_for_menu(&self, limit: usize) -> Vec<(String, String)> {
        self.history
            .lock()
            .unwrap()
            .iter()
            .take(limit)
            .map(|clip| (clip.id.clone(), clip_menu_label(clip)))
            .collect()
    }

    pub fn snapshot(&self) -> Snapshot {
        let config = self.config();
        let addresses = pairing::lan_addresses();
        let pair_uri = pairing::pair_uri(&config, &self.pair_secret(), &addresses);
        #[cfg(debug_assertions)]
        log::debug!("pairing uri: {pair_uri}");
        let connected = self.connected.lock().unwrap();
        let devices = config
            .devices
            .iter()
            .map(|d| DeviceView {
                id: d.id.clone(),
                name: d.name.clone(),
                connected: connected.contains_key(&d.id),
                paired_at: d.paired_at,
            })
            .collect();
        let history = self
            .history
            .lock()
            .unwrap()
            .iter()
            .map(|c| ClipView {
                id: c.id.clone(),
                mime: c.mime.clone(),
                preview: if c.mime == clipboard::TEXT_MIME {
                    String::from_utf8_lossy(&c.data)
                        .chars()
                        .take(PREVIEW_CHARS)
                        .collect()
                } else {
                    String::new()
                },
                size: c.data.len(),
                thumb: c.thumb.as_ref().map(|t| t.0.clone()),
                width: c.thumb.as_ref().map(|t| t.1),
                height: c.thumb.as_ref().map(|t| t.2),
                origin: c.origin_name.clone(),
                from_phone: c.origin_device.is_some(),
                ts: c.ts,
            })
            .collect();
        Snapshot {
            qr_svg: pairing::qr_svg(&pair_uri),
            pair_uri,
            server_name: config.server_name,
            port: config.port,
            paused: config.paused,
            notify_on_receive: config.notify_on_receive,
            addresses: addresses.iter().map(ToString::to_string).collect(),
            devices,
            history,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(mime: &str, data: &[u8]) -> Clip {
        Clip {
            id: "test".into(),
            thumb: None,
            mime: mime.into(),
            data: Arc::new(data.to_vec()),
            sha256: String::new(),
            ts: 0,
            origin_device: None,
            origin_name: String::new(),
        }
    }

    #[test]
    fn menu_labels_are_short_and_single_line() {
        assert_eq!(
            clip_menu_label(&clip(clipboard::TEXT_MIME, b"hello\n\t world")),
            "hello world"
        );
        let long = "a".repeat(200);
        let label = clip_menu_label(&clip(clipboard::TEXT_MIME, long.as_bytes()));
        assert_eq!(label.chars().count(), 48);
        assert!(label.ends_with('…'));
        assert_eq!(clip_menu_label(&clip(clipboard::PNG_MIME, b"png")), "Image");
    }
}
