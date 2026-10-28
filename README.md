# Peegel

**Shared clipboard between your Android phone and your Linux laptop over local Wi‑Fi.**

Copy on the laptop → paste on the phone. Copy on the phone → paste on the laptop.
Text and images. No cloud, no account; everything stays on your network and is
end-to-end encrypted.

> Status: early development. See the [milestones](https://github.com/LancemDev/peegel/milestones).

## How it works

```
┌──────────────────────┐   mDNS discovery    ┌──────────────────────┐
│  Desktop (Tauri)     │ ◀────────────────── │  Android app         │
│  tray + clipboard    │                     │  foreground service  │
│  watcher + WS server │ ◀═════════════════▶ │  + WS client         │
└──────────────────────┘  encrypted WebSocket└──────────────────────┘
```

| Direction        | How                                                                 |
|------------------|---------------------------------------------------------------------|
| Laptop → Phone   | Automatic. Anything you copy on the laptop lands in the phone's clipboard. |
| Phone → Laptop   | One tap: Quick Settings tile, the notification's **Send clipboard** button, or the Share menu. Android 10+ blocks background apps from reading the clipboard. |

Received clips go straight to the clipboard without a popup. Desktop notifications
are opt-in (Settings), and the tray's **Recent clips** menu keeps the last 8 clips
one click away — pick one to copy it again.

## Repository layout

| Path              | What                                                         |
|-------------------|--------------------------------------------------------------|
| `desktop/`        | Desktop app: Rust + Tauri 2 tray app (Linux, Wayland and X11) |
| `android/`        | Android app: Kotlin + Jetpack Compose                         |
| `docs/PROTOCOL.md`| Wire protocol: discovery, pairing, encryption, messages      |

## Tech stack

- **Desktop:** Rust, Tauri 2, tokio, tokio-tungstenite, mdns-sd, wl-clipboard-rs / arboard
- **Android:** Kotlin, Jetpack Compose, OkHttp WebSocket, NsdManager, Google code scanner
- **Security:** QR-code pairing, HKDF-SHA256 key derivation, ChaCha20-Poly1305

## Building

Instructions are in [`desktop/README.md`](desktop/README.md) and
[`android/README.md`](android/README.md).

## License

[MIT](LICENSE)
