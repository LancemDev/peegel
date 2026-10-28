const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (sel) => document.querySelector(sel);

let state = null;

function timeAgo(ms) {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 10) return "just now";
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  return new Date(ms).toLocaleDateString();
}

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function renderStatus() {
  const status = $("#status");
  const online = state.devices.filter((d) => d.connected);
  status.classList.toggle("online", online.length > 0 && !state.paused);
  status.classList.toggle("paused", state.paused);
  if (state.paused) {
    status.textContent = "Sync paused";
  } else if (online.length > 0) {
    status.textContent = `Connected to ${online.map((d) => d.name).join(", ")}`;
  } else if (state.devices.length > 0) {
    status.textContent = "Waiting for your phone…";
  } else {
    status.textContent = "No phone paired yet";
  }
  $("#paused").checked = state.paused;
  $("#notify").checked = state.notify_on_receive;
}

function renderClips() {
  const list = $("#clips");
  const template = $("#clip-template");
  list.replaceChildren();
  for (const clip of state.history) {
    const item = template.content.firstElementChild.cloneNode(true);
    const origin = item.querySelector(".origin");
    origin.textContent = clip.from_phone ? "📱" : "💻";
    origin.classList.toggle("phone", clip.from_phone);
    const isImage = clip.mime.startsWith("image/");
    item.querySelector(".preview").textContent = isImage ? "" : clip.preview;
    item.querySelector(".preview").classList.toggle("hidden", isImage);
    if (isImage && clip.thumb) {
      const thumb = item.querySelector(".thumb");
      thumb.src = clip.thumb;
      thumb.classList.remove("hidden");
    }
    const dims = clip.width ? `${clip.width}×${clip.height} · ` : "";
    item.querySelector(".meta").textContent =
      `${clip.origin} · ${timeAgo(clip.ts)} · ${dims}${formatSize(clip.size)}`;
    const copy = async () => {
      try {
        await invoke("recopy", { id: clip.id });
        item.classList.add("flash");
        setTimeout(() => item.classList.remove("flash"), 900);
      } catch (err) {
        console.error(err);
      }
    };
    item.addEventListener("click", copy);
    item.addEventListener("keydown", (e) => e.key === "Enter" && copy());
    list.append(item);
  }
  const empty = state.history.length === 0;
  $("#clips-empty").classList.toggle("hidden", !empty);
  $("#clear").classList.toggle("hidden", empty);
}

function renderDevices() {
  const list = $("#devices");
  const template = $("#device-template");
  list.replaceChildren();
  for (const device of state.devices) {
    const item = template.content.firstElementChild.cloneNode(true);
    item.querySelector(".dot").classList.toggle("on", device.connected);
    item.querySelector(".name").textContent = device.name;
    item.querySelector(".meta").textContent = device.connected
      ? "Connected"
      : `Paired ${new Date(device.paired_at).toLocaleDateString()}`;
    item.querySelector(".unpair").addEventListener("click", async () => {
      if (confirm(`Unpair ${device.name}?`)) {
        await invoke("unpair", { id: device.id });
      }
    });
    list.append(item);
  }
  $("#qr").src = `data:image/svg+xml;utf8,${encodeURIComponent(state.qr_svg)}`;
  $("#addresses").textContent = state.addresses.length
    ? `${state.server_name} · ${state.addresses.join(", ")} · port ${state.port}`
    : "No Wi‑Fi network found. Connect to Wi‑Fi to pair.";
}

async function refresh() {
  state = await invoke("get_state");
  renderStatus();
  renderClips();
  renderDevices();
}

function showTab(name) {
  for (const tab of document.querySelectorAll(".tab")) {
    tab.classList.toggle("active", tab.dataset.tab === name);
  }
  $("#tab-clips").classList.toggle("hidden", name !== "clips");
  $("#tab-devices").classList.toggle("hidden", name !== "devices");
  $("#tab-settings").classList.toggle("hidden", name !== "settings");
}

for (const tab of document.querySelectorAll(".tab")) {
  tab.addEventListener("click", () => showTab(tab.dataset.tab));
}
$("#paused").addEventListener("change", (e) => invoke("set_paused", { paused: e.target.checked }));
$("#new-code").addEventListener("click", () => invoke("new_pairing_code"));
$("#clear").addEventListener("click", () => invoke("clear_history"));
$("#notify").addEventListener("change", (e) => invoke("set_notify", { notify: e.target.checked }));
$("#autostart").addEventListener("change", async (e) => {
  try {
    await invoke("set_autostart", { enabled: e.target.checked });
  } catch (err) {
    console.error(err);
  }
  e.target.checked = await invoke("get_autostart");
});
invoke("get_autostart").then((enabled) => ($("#autostart").checked = enabled));

listen("state-changed", refresh);
setInterval(() => state && renderClips(), 30_000);

refresh().then(() => {
  if (state.devices.length === 0) showTab("devices");
});
