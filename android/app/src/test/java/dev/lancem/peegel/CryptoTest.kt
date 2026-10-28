package dev.lancem.peegel

import dev.lancem.peegel.crypto.Crypto
import dev.lancem.peegel.crypto.FrameCipher
import dev.lancem.peegel.sync.Protocol
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

/** Vectors shared with desktop/src-tauri/src/crypto.rs. */
class CryptoTest {
    private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }

    @Test
    fun knownVectors() {
        val deviceKey = Crypto.deviceKey(ByteArray(32) { 7 }, "phone-1")
        assertEquals("3efd567dc00a006d30bd088a7e172909cebf196d08f957999649a963c2db57a4", hex(deviceKey))

        val (c2s, s2c) = Crypto.sessionKeys(deviceKey, ByteArray(32) { 1 }, ByteArray(32) { 2 })
        assertEquals("9ba6af2735b49a685654595352a9d7b13d6967023ed00e8e6a23cf92a97d2661", hex(c2s))
        assertEquals("db0b68ed8c23fb6321f55c69ea4b3fb495b0a41cca1e08d2a0c916b9a8c96e75", hex(s2c))

        val frame = Protocol.encode(Protocol.header("auth"))
        assertEquals("0000000c7b2274223a2261757468227d", hex(frame))
        assertEquals("0f7dcdd99b16f4ec3665ed880aceb93533775b9a5ce751fdfca2d3b92bc5adb2", hex(FrameCipher(c2s).seal(frame)))
    }

    @Test
    fun rejectsReplayedFrames() {
        val key = ByteArray(32) { 1 }
        val sender = FrameCipher(key)
        val receiver = FrameCipher(key)
        val first = sender.seal("hello".toByteArray())
        val second = sender.seal("world".toByteArray())
        assertArrayEquals("hello".toByteArray(), receiver.open(first))
        assertArrayEquals("world".toByteArray(), receiver.open(second))
        assertThrows(Exception::class.java) { receiver.open(first) }
    }

    @Test
    fun frameRoundtrip() {
        val header = Protocol.header("clip").put("mime", "text/plain")
        val (decoded, body) = Protocol.decode(Protocol.encode(header, "hi".toByteArray()))
        assertEquals("clip", decoded.getString("t"))
        assertArrayEquals("hi".toByteArray(), body)
    }
}
