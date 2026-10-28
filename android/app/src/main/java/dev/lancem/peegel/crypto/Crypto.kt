package dev.lancem.peegel.crypto

import java.nio.ByteBuffer
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.Mac
import javax.crypto.spec.IvParameterSpec
import javax.crypto.spec.SecretKeySpec

/** Key derivation and frame encryption. See docs/PROTOCOL.md. */
object Crypto {
    private val random = SecureRandom()

    fun randomBytes(size: Int): ByteArray = ByteArray(size).also(random::nextBytes)

    fun b64(data: ByteArray): String = Base64.getUrlEncoder().withoutPadding().encodeToString(data)

    fun unb64(data: String): ByteArray = Base64.getUrlDecoder().decode(data)

    fun sha256Hex(data: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(data).joinToString("") { "%02x".format(it) }

    fun hkdf(ikm: ByteArray, salt: ByteArray, info: ByteArray, length: Int): ByteArray {
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(salt, "HmacSHA256"))
        val prk = mac.doFinal(ikm)
        mac.init(SecretKeySpec(prk, "HmacSHA256"))
        val out = ByteArray(length)
        var block = ByteArray(0)
        var offset = 0
        var counter = 1
        while (offset < length) {
            mac.update(block)
            mac.update(info)
            mac.update(counter.toByte())
            block = mac.doFinal()
            val n = minOf(block.size, length - offset)
            System.arraycopy(block, 0, out, offset, n)
            offset += n
            counter++
        }
        return out
    }

    fun deviceKey(pairSecret: ByteArray, deviceId: String): ByteArray =
        hkdf(pairSecret, deviceId.toByteArray(), "peegel/v1/device-key".toByteArray(), 32)

    /** Returns (client → server, server → client) keys. */
    fun sessionKeys(deviceKey: ByteArray, clientNonce: ByteArray, serverNonce: ByteArray): Pair<ByteArray, ByteArray> {
        val okm = hkdf(deviceKey, clientNonce + serverNonce, "peegel/v1/session".toByteArray(), 64)
        return okm.copyOfRange(0, 32) to okm.copyOfRange(32, 64)
    }
}

/** One direction of an encrypted session: ChaCha20-Poly1305 with a counter nonce. */
class FrameCipher(key: ByteArray) {
    private val key = SecretKeySpec(key, "ChaCha20")
    private var counter = 0L

    private fun nonce(): IvParameterSpec =
        IvParameterSpec(ByteBuffer.allocate(12).putInt(0).putLong(counter).array())

    fun seal(plaintext: ByteArray): ByteArray = synchronized(this) {
        val cipher = newCipher()
        cipher.init(Cipher.ENCRYPT_MODE, key, nonce())
        counter++
        cipher.doFinal(plaintext)
    }

    fun open(ciphertext: ByteArray): ByteArray = synchronized(this) {
        val cipher = newCipher()
        cipher.init(Cipher.DECRYPT_MODE, key, nonce())
        val plaintext = cipher.doFinal(ciphertext)
        counter++
        plaintext
    }

    private companion object {
        fun newCipher(): Cipher =
            try {
                // Android (Conscrypt) name.
                Cipher.getInstance("ChaCha20/Poly1305/NoPadding")
            } catch (_: Exception) {
                // Desktop JVM name, used by unit tests.
                Cipher.getInstance("ChaCha20-Poly1305")
            }
    }
}
