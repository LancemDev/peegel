#!/usr/bin/env python3
"""Minimal Peegel client for testing the desktop app without a phone.

    pip install websockets cryptography
    python3 tools/test_client.py 'peegel://pair?...'          # pair, then listen
    python3 tools/test_client.py 'peegel://pair?...' --send "hello"

The pairing URI is shown in the desktop app (and logged with RUST_LOG=debug
in debug builds). After pairing, the device key is saved to
~/.cache/peegel-test-client.json and reused with --resume.
"""

import argparse
import asyncio
import base64
import hashlib
import hmac
import json
import os
import struct
import time
import uuid
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import websockets
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305

STATE = Path.home() / ".cache" / "peegel-test-client.json"


def b64(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def unb64(data: str) -> bytes:
    return base64.urlsafe_b64decode(data + "=" * (-len(data) % 4))


def hkdf(ikm: bytes, salt: bytes, info: bytes, length: int) -> bytes:
    prk = hmac.new(salt, ikm, hashlib.sha256).digest()
    out, block, counter = b"", b"", 1
    while len(out) < length:
        block = hmac.new(prk, block + info + bytes([counter]), hashlib.sha256).digest()
        out += block
        counter += 1
    return out[:length]


class Cipher:
    def __init__(self, key: bytes):
        self.aead = ChaCha20Poly1305(key)
        self.counter = 0

    def _nonce(self) -> bytes:
        nonce = b"\0\0\0\0" + struct.pack(">Q", self.counter)
        self.counter += 1
        return nonce

    def seal(self, data: bytes) -> bytes:
        return self.aead.encrypt(self._nonce(), data, None)

    def open(self, data: bytes) -> bytes:
        return self.aead.decrypt(self._nonce(), data, None)


def encode(header: dict, body: bytes = b"") -> bytes:
    raw = json.dumps(header, separators=(",", ":")).encode()
    return struct.pack(">I", len(raw)) + raw + body


def decode(frame: bytes):
    (length,) = struct.unpack(">I", frame[:4])
    return json.loads(frame[4 : 4 + length]), frame[4 + length :]


async def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("uri", nargs="?", help="peegel://pair?... URI")
    parser.add_argument("--resume", action="store_true")
    parser.add_argument("--send", help="text to send after connecting")
    parser.add_argument("--send-image", help="PNG or JPEG file to send after connecting")
    args = parser.parse_args()

    if args.resume:
        saved = json.loads(STATE.read_text())
        host, port = saved["host"], saved["port"]
        device_id, device_key, mode = saved["device_id"], unb64(saved["key"]), "resume"
    else:
        query = {k: v[0] for k, v in parse_qs(urlparse(args.uri).query).items()}
        host, port = query["h"].split(",")[0], int(query["p"])
        device_id = b64(os.urandom(16))
        device_key = hkdf(unb64(query["s"]), device_id.encode(), b"peegel/v1/device-key", 32)
        mode = "pair"

    async with websockets.connect(f"ws://{host}:{port}/ws", max_size=32 * 1024 * 1024) as ws:
        client_nonce = os.urandom(32)
        await ws.send(json.dumps({
            "t": "hello", "v": 1, "mode": mode, "device_id": device_id,
            "device_name": "Test client", "nonce": b64(client_nonce),
        }))
        challenge = json.loads(await ws.recv())
        if challenge["t"] != "challenge":
            raise SystemExit(f"rejected: {challenge}")
        okm = hkdf(device_key, client_nonce + unb64(challenge["nonce"]), b"peegel/v1/session", 64)
        tx, rx = Cipher(okm[:32]), Cipher(okm[32:])
        await ws.send(tx.seal(encode({"t": "auth"})))
        header, _ = decode(rx.open(await ws.recv()))
        assert header["t"] == "welcome", header
        print(f"connected to {challenge['server_name']} ({mode})")

        STATE.parent.mkdir(parents=True, exist_ok=True)
        STATE.write_text(json.dumps({
            "host": host, "port": port, "device_id": device_id, "key": b64(device_key),
        }))

        async def send_clip(mime: str, body: bytes) -> None:
            await ws.send(tx.seal(encode({
                "t": "clip", "id": str(uuid.uuid4()), "mime": mime,
                "sha256": hashlib.sha256(body).hexdigest(), "ts": int(time.time() * 1000),
            }, body)))
            print(f"sent {mime} ({len(body)} bytes)", flush=True)

        if args.send:
            await send_clip("text/plain", args.send.encode())
        if args.send_image:
            data = Path(args.send_image).read_bytes()
            await send_clip("image/jpeg" if data[:2] == b"\xff\xd8" else "image/png", data)

        async for message in ws:
            header, body = decode(rx.open(message))
            if header["t"] == "ping":
                await ws.send(tx.seal(encode({"t": "pong"})))
            elif header["t"] == "clip":
                if header["mime"].startswith("text/"):
                    preview = body.decode(errors="replace")
                else:
                    path = Path("/tmp") / f"peegel-{header['id']}.{header['mime'].split('/')[1]}"
                    path.write_bytes(body)
                    preview = f"{len(body)} bytes saved to {path}"
                print(f"clip [{header['mime']}]: {preview}", flush=True)


if __name__ == "__main__":
    asyncio.run(main())
