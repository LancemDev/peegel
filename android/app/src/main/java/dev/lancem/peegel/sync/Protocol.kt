package dev.lancem.peegel.sync

import android.net.Uri
import org.json.JSONObject
import java.nio.ByteBuffer

/** Wire format shared with the desktop app. See docs/PROTOCOL.md. */
object Protocol {
    const val VERSION = 1
    const val SERVICE_TYPE = "_peegel._tcp"
    const val MAX_FRAME_BYTES = 32 * 1024 * 1024
    const val CLOSE_AUTH_FAILED = 4001
    const val TEXT_MIME = "text/plain"

    fun encode(header: JSONObject, body: ByteArray = ByteArray(0)): ByteArray {
        val json = header.toString().toByteArray()
        return ByteBuffer.allocate(4 + json.size + body.size)
            .putInt(json.size)
            .put(json)
            .put(body)
            .array()
    }

    fun decode(frame: ByteArray): Pair<JSONObject, ByteArray> {
        require(frame.size >= 4) { "frame too short" }
        val length = ByteBuffer.wrap(frame, 0, 4).int
        require(length >= 0 && frame.size >= 4 + length) { "header length exceeds frame" }
        val header = JSONObject(String(frame, 4, length))
        return header to frame.copyOfRange(4 + length, frame.size)
    }

    fun header(type: String): JSONObject = JSONObject().put("t", type)
}

data class PairInfo(
    val serverId: String,
    val serverName: String,
    val hosts: List<String>,
    val port: Int,
    val secret: ByteArray,
) {
    companion object {
        /** Parses `peegel://pair?v=1&id=…&name=…&h=…&p=…&s=…`. */
        fun parse(raw: String): PairInfo? = runCatching {
            val uri = Uri.parse(raw.trim())
            require(uri.scheme == "peegel" && uri.host == "pair")
            require(uri.getQueryParameter("v")?.toInt() == Protocol.VERSION)
            PairInfo(
                serverId = uri.getQueryParameter("id")!!,
                serverName = uri.getQueryParameter("name") ?: "Computer",
                hosts = uri.getQueryParameter("h")!!.split(",").map(String::trim).filter(String::isNotEmpty),
                port = uri.getQueryParameter("p")!!.toInt(),
                secret = dev.lancem.peegel.crypto.Crypto.unb64(uri.getQueryParameter("s")!!),
            ).also { require(it.hosts.isNotEmpty() && it.secret.size == 32) }
        }.getOrNull()
    }
}
