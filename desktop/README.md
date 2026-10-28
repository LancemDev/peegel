# Peegel desktop

Tray app for Linux (Rust + Tauri 2). It watches the clipboard, runs the
encrypted WebSocket server phones connect to, and advertises itself over mDNS.

## Requirements

- Rust 1.80+
- Node.js + pnpm (only for the Tauri CLI)
- Tauri's Linux dependencies: `webkit2gtk-4.1`, `libayatana-appindicator` (or
  `libappindicator-gtk3`), `librsvg`. See <https://tauri.app/start/prerequisites/>.
- On Wayland, a compositor with `ext-data-control` or `wlr-data-control`
  (KDE Plasma 6, Sway, Hyprland, …). Otherwise the app falls back to polling.

## Run

```sh
cd desktop
pnpm install
pnpm dev        # or: cd src-tauri && cargo run
```

## Build packages

```sh
pnpm build      # .deb, .rpm and AppImage in src-tauri/target/release/bundle/
```

## Firewall

Phones connect on TCP port **47820** and find the computer with mDNS
(UDP 5353). If you run a firewall, allow both on your home network, e.g.

```sh
sudo ufw allow 47820/tcp
sudo ufw allow 5353/udp
```

## Testing without a phone

`tools/test_client.py` speaks the same protocol as the Android app:

```sh
RUST_LOG=peegel_lib=debug cargo run --manifest-path src-tauri/Cargo.toml   # logs the pairing URI
python3 tools/test_client.py 'peegel://pair?...' --send "hello"
python3 tools/test_client.py --resume            # prints clips copied on this computer
```

Set `PEEGEL_CONFIG_DIR` to use a separate config (paired devices, settings)
while testing.
