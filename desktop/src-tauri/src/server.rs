//! WebSocket server that phones connect to. Implements the handshake and
//! session described in docs/PROTOCOL.md.

use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::broadcast::error::RecvError,
    time::{interval, timeout, MissedTickBehavior},
};
use tokio_tungstenite::{
    accept_async_with_config,
    tungstenite::{
        protocol::{frame::coding::CloseCode, CloseFrame, WebSocketConfig},
        Message,
    },
    WebSocketStream,
};

use crate::{
    core::Core,
    crypto::{self, FrameCipher},
    protocol::{self, Handshake, Header, Mode},
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const PING_INTERVAL: Duration = Duration::from_secs(20);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const CLOSE_AUTH_FAILED: u16 = 4001;

type Ws = WebSocketStream<TcpStream>;

pub async fn run(core: Arc<Core>) -> Result<()> {
    let port = core.config().port;
    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .with_context(|| format!("could not listen on port {port}"))?;
    log::info!("listening on 0.0.0.0:{port}");
    loop {
        let (stream, addr) = listener.accept().await?;
        let core = core.clone();
        tokio::spawn(async move {
            if let Err(err) = handle(core, stream).await {
                log::info!("connection from {addr} ended: {err:#}");
            }
        });
    }
}

struct Session {
    device_id: String,
    device_name: String,
    rx: FrameCipher,
    tx: FrameCipher,
}

async fn handle(core: Arc<Core>, stream: TcpStream) -> Result<()> {
    stream.set_nodelay(true)?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(protocol::MAX_FRAME_BYTES))
        .max_frame_size(Some(protocol::MAX_FRAME_BYTES));
    let mut ws = accept_async_with_config(stream, Some(config)).await?;

    let mut session = timeout(HANDSHAKE_TIMEOUT, handshake(&core, &mut ws))
        .await
        .map_err(|_| anyhow!("handshake timed out"))??;
    log::info!("{} connected", session.device_name);

    core.set_connected(&session.device_id, true);
    let result = run_session(&core, &mut ws, &mut session).await;
    core.set_connected(&session.device_id, false);
    let _ = ws.close(None).await;
    result
}

async fn send_handshake(ws: &mut Ws, message: &Handshake) -> Result<()> {
    ws.send(Message::text(serde_json::to_string(message)?)).await?;
    Ok(())
}

async fn reject(ws: &mut Ws, code: &str) -> Result<Session> {
    send_handshake(
        ws,
        &Handshake::Error {
            code: code.to_string(),
        },
    )
    .await?;
    let _ = ws.close(None).await;
    bail!("rejected: {code}")
}

async fn handshake(core: &Core, ws: &mut Ws) -> Result<Session> {
    let hello = match ws.next().await {
        Some(Ok(Message::Text(text))) => serde_json::from_str::<Handshake>(&text)?,
        _ => bail!("expected hello"),
    };
    let Handshake::Hello {
        v,
        mode,
        device_id,
        device_name,
        nonce: client_nonce,
    } = hello
    else {
        bail!("expected hello");
    };
    if v != protocol::VERSION {
        return reject(ws, "version").await;
    }
    let client_nonce = crypto::unb64(&client_nonce)?;
    if client_nonce.len() != 32 || device_id.is_empty() || device_id.len() > 64 {
        bail!("malformed hello");
    }
    let device_name: String = device_name.chars().take(64).collect();

    let pair_secret = core.pair_secret();
    let device_key = match mode {
        Mode::Pair => crypto::derive_device_key(&pair_secret, &device_id),
        Mode::Resume => match core.device_key(&device_id) {
            Some(key) => key,
            None => return reject(ws, "unknown_device").await,
        },
    };

    let server_nonce = crypto::random_bytes::<32>();
    let config = core.config();
    send_handshake(
        ws,
        &Handshake::Challenge {
            v: protocol::VERSION,
            server_id: config.server_id,
            server_name: config.server_name,
            nonce: crypto::b64(&server_nonce),
        },
    )
    .await?;

    let (c2s, s2c) = crypto::derive_session_keys(&device_key, &client_nonce, &server_nonce);
    let mut session = Session {
        device_id,
        device_name,
        rx: FrameCipher::new(&c2s),
        tx: FrameCipher::new(&s2c),
    };

    let authed = match ws.next().await {
        Some(Ok(Message::Binary(frame))) => session
            .rx
            .open(&frame)
            .ok()
            .and_then(|plain| protocol::decode(&plain).ok().map(|(h, _)| h))
            .is_some_and(|header| header == Header::Auth),
        _ => false,
    };
    if !authed {
        let _ = ws
            .close(Some(CloseFrame {
                code: CloseCode::from(CLOSE_AUTH_FAILED),
                reason: "authentication failed".into(),
            }))
            .await;
        bail!("authentication failed ({mode:?})");
    }

    match mode {
        Mode::Pair => {
            core.add_device(&session.device_id, &session.device_name, &device_key);
            core.rotate_pair_secret();
            log::info!("paired with {}", session.device_name);
        }
        Mode::Resume => core.update_device_name(&session.device_id, &session.device_name),
    }

    send_frame(ws, &mut session, &Header::Welcome, &[]).await?;
    Ok(session)
}

async fn send_frame(ws: &mut Ws, session: &mut Session, header: &Header, body: &[u8]) -> Result<()> {
    let frame = session.tx.seal(&protocol::encode(header, body));
    ws.send(Message::binary(frame)).await?;
    Ok(())
}

async fn run_session(core: &Core, ws: &mut Ws, session: &mut Session) -> Result<()> {
    let mut outgoing = core.subscribe();
    let mut ping = interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ping.tick().await;

    loop {
        tokio::select! {
            incoming = timeout(IDLE_TIMEOUT, ws.next()) => {
                let message = match incoming {
                    Err(_) => bail!("idle timeout"),
                    Ok(None) => return Ok(()),
                    Ok(Some(message)) => message?,
                };
                match message {
                    Message::Binary(frame) => {
                        let plain = session.rx.open(&frame)?;
                        let (header, body) = protocol::decode(&plain)?;
                        match header {
                            Header::Ping => send_frame(ws, session, &Header::Pong, &[]).await?,
                            Header::Pong => {}
                            Header::Clip { id, mime, sha256, .. } => {
                                if let Err(err) = core.remote_clip(
                                    &session.device_id,
                                    &session.device_name,
                                    id,
                                    mime,
                                    sha256,
                                    body,
                                ) {
                                    log::warn!("dropped clip from {}: {err:#}", session.device_name);
                                }
                            }
                            other => log::debug!("ignoring unexpected {other:?}"),
                        }
                    }
                    Message::Close(_) => return Ok(()),
                    _ => {}
                }
            }
            clip = outgoing.recv() => {
                let clip = match clip {
                    Ok(clip) => clip,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return Ok(()),
                };
                if clip.origin_device.as_deref() == Some(session.device_id.as_str()) {
                    continue;
                }
                let header = Header::Clip {
                    id: clip.id.clone(),
                    mime: clip.mime.clone(),
                    sha256: clip.sha256.clone(),
                    ts: clip.ts,
                };
                send_frame(ws, session, &header, &clip.data).await?;
            }
            _ = ping.tick() => {
                if !core.is_paired(&session.device_id) {
                    bail!("device was unpaired");
                }
                send_frame(ws, session, &Header::Ping, &[]).await?;
            }
        }
    }
}
