# Peegel wire protocol (v1)

Peegel syncs clipboard contents (text and images) between a desktop and an
Android phone on the same local network. The desktop acts as the **server**,
the phone as the **client**.

## Transport

- WebSocket over TCP. Default port **47820**, path **`/ws`**.
- Handshake frames are WebSocket **text** frames containing JSON.
- After the handshake, every frame is a WebSocket **binary** frame that is
  encrypted (see [Encryption](#encryption)).
- Maximum frame size: **32 MiB**.

## Discovery

The desktop advertises an mDNS / DNS-SD service:

| Field         | Value                        |
|---------------|------------------------------|
| Service type  | `_peegel._tcp.local.`        |
| Instance name | Human readable desktop name  |
| Port          | WebSocket port               |
| TXT `id`      | `server_id`                  |
| TXT `v`       | Protocol version (`1`)       |

A paired phone uses the `id` TXT record to find its desktop again when the
desktop's IP address changes.

## Identifiers and encoding

- `server_id` and `device_id` are 16 random bytes, base64url without padding.
- All binary values in JSON (nonces, secrets) are base64url without padding.

## Pairing

The desktop shows a QR code with this URI:

```
peegel://pair?v=1&id=<server_id>&name=<url-encoded name>&h=<ip>[,<ip>...]&p=<port>&s=<pair_secret>
```

`pair_secret` is 32 random bytes. It is valid for one successful pairing and is
regenerated right after it is used, or when the user asks for a new code.

Both sides derive a long-term per-device key:

```
device_key = HKDF-SHA256(ikm = pair_secret,
                         salt = UTF-8(device_id),
                         info = "peegel/v1/device-key",
                         L = 32)
```

The desktop stores `device_id → (device_name, device_key)`. The phone stores
`server_id → (server_name, last known hosts, port, device_key)`.

## Handshake

```
C → S  text  {"t":"hello","v":1,"mode":"pair"|"resume",
              "device_id":"…","device_name":"…","nonce":"<32 bytes>"}
S → C  text  {"t":"challenge","v":1,"server_id":"…","server_name":"…",
              "nonce":"<32 bytes>"}
       or    {"t":"error","code":"version"|"unknown_device"|"pairing_closed"}
C → S  bin   encrypted {"t":"auth"}
S → C  bin   encrypted {"t":"welcome"}
```

- In `pair` mode the server derives `device_key` from its current
  `pair_secret`. In `resume` mode it looks up the stored key for `device_id`.
- If the first binary frame cannot be decrypted the server closes the
  connection with code `4001`.
- After a successful `pair` handshake, the server stores the device and
  rotates `pair_secret`.

## Encryption

Session keys:

```
okm   = HKDF-SHA256(ikm = device_key,
                    salt = client_nonce || server_nonce,
                    info = "peegel/v1/session",
                    L = 64)
k_c2s = okm[0..32]     // client → server
k_s2c = okm[32..64]    // server → client
```

Each direction uses **ChaCha20-Poly1305** with an implicit 12-byte nonce:
4 zero bytes followed by a 64-bit big-endian message counter that starts at 0
and increases by one per frame. A frame that fails to decrypt (wrong key,
replayed, reordered, or tampered with) closes the connection.

A binary frame is `ciphertext || 16-byte tag`. AAD is empty.

## Messages

The decrypted plaintext of every frame is:

```
u32 big-endian header length | header (UTF-8 JSON) | body (raw bytes)
```

| Header                                                              | Body          |
|---------------------------------------------------------------------|---------------|
| `{"t":"auth"}`                                                      | empty         |
| `{"t":"welcome"}`                                                   | empty         |
| `{"t":"ping"}` / `{"t":"pong"}`                                     | empty         |
| `{"t":"clip","id":"<uuid>","mime":"…","sha256":"<hex>","ts":<ms>}`  | clip contents |

Supported `mime` values: `text/plain` (UTF-8 body), `image/png`, `image/jpeg`.

`sha256` is the hex SHA-256 of the body. Each side remembers the hash of the
last clip it applied or sent, and ignores clipboard changes with the same hash.
This stops clips from bouncing back and forth between devices.

Either side sends `ping` every 20 seconds. A connection with no frames for 60
seconds is considered dead.
