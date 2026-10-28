package dev.lancem.peegel.sync

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import android.util.Log
import dev.lancem.peegel.crypto.Crypto
import dev.lancem.peegel.crypto.FrameCipher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import org.json.JSONObject
import java.util.UUID
import java.util.concurrent.TimeUnit

sealed interface Status {
    data object NotPaired : Status
    data class Pairing(val name: String) : Status
    data class Connecting(val name: String) : Status
    data class Connected(val name: String) : Status
    data class Error(val message: String) : Status
}

data class ClipItem(
    val id: String,
    val mime: String,
    val text: String,
    val size: Int,
    val fromPhone: Boolean,
    val ts: Long,
)

private class HandshakeError(val code: String) : Exception("rejected by computer: $code")
private class AuthFailed : Exception("authentication failed")

/**
 * Keeps one encrypted WebSocket connection to the paired computer alive,
 * applies incoming clips to the clipboard and sends clips on request.
 */
class Sync(private val context: Context, val store: Store) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val http = OkHttpClient.Builder()
        .connectTimeout(4, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.SECONDS)
        .build()
    private val discovery = Discovery(context)
    private val clipboard = context.getSystemService(ClipboardManager::class.java)

    private val _status = MutableStateFlow<Status>(if (store.server == null) Status.NotPaired else Status.Connecting(store.server!!.name))
    val status: StateFlow<Status> = _status.asStateFlow()

    private val _history = MutableStateFlow<List<ClipItem>>(emptyList())
    val history: StateFlow<List<ClipItem>> = _history.asStateFlow()

    private val _paused = MutableStateFlow(store.paused)
    val paused: StateFlow<Boolean> = _paused.asStateFlow()

    private var loop: Job? = null
    private val wake = Channel<Unit>(Channel.CONFLATED)

    @Volatile private var activeInbox: Channel<Event>? = null
    @Volatile private var lastHash: String? = null

    private sealed interface Event {
        data object Open : Event
        data class Text(val text: String) : Event
        data class Binary(val bytes: ByteArray) : Event
        data class Closed(val code: Int, val reason: String) : Event
        data class Failed(val error: Throwable) : Event
        data class Send(val header: JSONObject, val body: ByteArray) : Event
        data object Tick : Event
    }

    val isConnected: Boolean get() = activeInbox != null

    /** Starts (or keeps) the reconnect loop for the paired computer. */
    fun start() {
        if (loop?.isActive == true) return
        if (store.server == null) {
            _status.value = Status.NotPaired
            return
        }
        loop = scope.launch { runLoop(null) }
    }

    fun stop() {
        val job = loop
        loop = null
        scope.launch {
            job?.cancelAndJoin()
            if (_status.value !is Status.Error) {
                _status.value = store.server?.let { Status.Connecting(it.name) } ?: Status.NotPaired
            }
        }
    }

    /** Retry right away, e.g. after the network changed. */
    fun reconnectNow() {
        wake.trySend(Unit)
    }

    fun pair(info: PairInfo) {
        _status.value = Status.Pairing(info.serverName)
        val previous = loop
        loop = scope.launch {
            previous?.cancelAndJoin()
            runLoop(info)
        }
    }

    fun unpair() {
        val previous = loop
        loop = null
        scope.launch {
            previous?.cancelAndJoin()
            store.server = null
            _status.value = Status.NotPaired
        }
    }

    fun setPaused(paused: Boolean) {
        store.paused = paused
        _paused.value = paused
    }

    /** Queues a clip for the computer. Returns false if not connected. */
    fun send(mime: String, body: ByteArray): Boolean {
        val channel = activeInbox ?: return false
        val sha = Crypto.sha256Hex(body)
        lastHash = sha
        val header = Protocol.header("clip")
            .put("id", UUID.randomUUID().toString())
            .put("mime", mime)
            .put("sha256", sha)
            .put("ts", System.currentTimeMillis())
        addHistory(header, body, fromPhone = true)
        return channel.trySend(Event.Send(header, body)).isSuccess
    }

    /** Copies a history entry back onto this phone's clipboard. */
    fun recopy(item: ClipItem) {
        if (item.mime == Protocol.TEXT_MIME) {
            clipboard.setPrimaryClip(ClipData.newPlainText("Peegel", item.text))
        }
    }

    private suspend fun runLoop(initialPairing: PairInfo?) {
        var pairing = initialPairing
        var backoff = 1_000L
        while (currentCoroutineContext().isActive) {
            if (pairing != null) {
                val info = pairing
                _status.value = Status.Pairing(info.serverName)
                val key = Crypto.deviceKey(info.secret, store.deviceId)
                var paired = false
                var lastError: Throwable? = null
                for (host in info.hosts) {
                    try {
                        session(host, info.port, "pair", key) { serverId, serverName ->
                            store.server = PairedServer(serverId, serverName, (listOf(host) + info.hosts).distinct(), info.port, key)
                            paired = true
                        }
                    } catch (e: Exception) {
                        if (e is kotlinx.coroutines.CancellationException) throw e
                        lastError = e
                    }
                    if (paired) break
                }
                if (!paired) {
                    Log.w(TAG, "pairing failed", lastError)
                    _status.value = Status.Error(
                        when (lastError) {
                            is AuthFailed -> "That code has expired. Show a new code on your computer and scan again."
                            is HandshakeError -> "Your computer rejected the pairing (${lastError.code})."
                            else -> "Couldn't reach ${info.serverName}. Make sure both devices are on the same Wi‑Fi."
                        },
                    )
                    return
                }
                pairing = null
                continue
            }

            val server = store.server ?: run {
                _status.value = Status.NotPaired
                return
            }
            if (_status.value !is Status.Connected) _status.value = Status.Connecting(server.name)

            var connected = false
            val onReady: (String, String) -> Unit = { _, _ -> connected = true }
            try {
                for (host in server.hosts) {
                    connected = tryResume(host, server.port, server.key, onReady)
                    if (connected) break
                }
                if (!connected) {
                    val found = discovery.find(server.id, 6_000)
                    if (found != null) {
                        store.server = server.copy(hosts = (listOf(found.host) + server.hosts).distinct().take(4), port = found.port)
                        connected = tryResume(found.host, found.port, server.key, onReady)
                    }
                }
            } catch (e: HandshakeError) {
                if (e.code == "unknown_device") {
                    store.server = null
                    _status.value = Status.Error("${server.name} no longer knows this phone. Pair again.")
                    return
                }
            } catch (_: AuthFailed) {
                store.server = null
                _status.value = Status.Error("${server.name} no longer knows this phone. Pair again.")
                return
            }

            backoff = if (connected) 1_000L else minOf(backoff * 2, 30_000L)
            _status.value = Status.Connecting(server.name)
            withTimeoutOrNull(backoff) { wake.receive() }
        }
    }

    /** Returns true if a session was established (and has since ended). */
    private suspend fun tryResume(host: String, port: Int, key: ByteArray, onReady: (String, String) -> Unit): Boolean {
        var ready = false
        try {
            session(host, port, "resume", key) { id, name ->
                ready = true
                onReady(id, name)
            }
        } catch (e: HandshakeError) {
            throw e
        } catch (e: AuthFailed) {
            throw e
        } catch (e: Exception) {
            if (e is kotlinx.coroutines.CancellationException) throw e
            Log.d(TAG, "connection to $host failed: $e")
        }
        return ready
    }

    private suspend fun session(
        host: String,
        port: Int,
        mode: String,
        deviceKey: ByteArray,
        onReady: (serverId: String, serverName: String) -> Unit,
    ) = coroutineScope {
        val inbox = Channel<Event>(Channel.UNLIMITED)
        val listener = object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) { inbox.trySend(Event.Open) }
            override fun onMessage(webSocket: WebSocket, text: String) { inbox.trySend(Event.Text(text)) }
            override fun onMessage(webSocket: WebSocket, bytes: ByteString) { inbox.trySend(Event.Binary(bytes.toByteArray())) }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(1000, null)
                inbox.trySend(Event.Closed(code, reason))
            }
            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) { inbox.trySend(Event.Closed(code, reason)) }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) { inbox.trySend(Event.Failed(t)) }
        }
        val ws = http.newWebSocket(Request.Builder().url("ws://$host:$port/ws").build(), listener)
        var ticker: Job? = null
        try {
            suspend fun next(): Event = withTimeout(8_000) { inbox.receive() }

            when (val event = next()) {
                Event.Open -> Unit
                is Event.Failed -> throw event.error
                else -> error("unexpected $event")
            }

            val clientNonce = Crypto.randomBytes(32)
            ws.send(
                JSONObject()
                    .put("t", "hello")
                    .put("v", Protocol.VERSION)
                    .put("mode", mode)
                    .put("device_id", store.deviceId)
                    .put("device_name", store.deviceName)
                    .put("nonce", Crypto.b64(clientNonce))
                    .toString(),
            )

            val challenge = when (val event = next()) {
                is Event.Text -> JSONObject(event.text)
                is Event.Failed -> throw event.error
                else -> error("unexpected $event")
            }
            if (challenge.getString("t") == "error") throw HandshakeError(challenge.getString("code"))
            require(challenge.getString("t") == "challenge")

            val (c2s, s2c) = Crypto.sessionKeys(deviceKey, clientNonce, Crypto.unb64(challenge.getString("nonce")))
            val tx = FrameCipher(c2s)
            val rx = FrameCipher(s2c)
            fun sendFrame(header: JSONObject, body: ByteArray = ByteArray(0)) {
                ws.send(tx.seal(Protocol.encode(header, body)).toByteString())
            }

            sendFrame(Protocol.header("auth"))
            when (val event = next()) {
                is Event.Binary -> {
                    val (header, _) = Protocol.decode(rx.open(event.bytes))
                    require(header.getString("t") == "welcome")
                }
                is Event.Closed -> if (event.code == Protocol.CLOSE_AUTH_FAILED) throw AuthFailed() else error("closed: ${event.code}")
                is Event.Failed -> throw event.error
                else -> error("unexpected $event")
            }

            val serverName = challenge.getString("server_name")
            onReady(challenge.getString("server_id"), serverName)
            store.server?.takeIf { it.name != serverName }?.let { store.server = it.copy(name = serverName) }
            _status.value = Status.Connected(serverName)
            activeInbox = inbox
            Log.i(TAG, "connected to $serverName at $host")

            ticker = launch {
                while (isActive) {
                    delay(PING_INTERVAL_MS)
                    inbox.send(Event.Tick)
                }
            }

            var lastReceived = System.currentTimeMillis()
            while (true) {
                when (val event = inbox.receive()) {
                    is Event.Binary -> {
                        lastReceived = System.currentTimeMillis()
                        val (header, body) = Protocol.decode(rx.open(event.bytes))
                        when (header.getString("t")) {
                            "ping" -> sendFrame(Protocol.header("pong"))
                            "clip" -> handleIncoming(header, body)
                        }
                    }
                    is Event.Send -> sendFrame(event.header, event.body)
                    Event.Tick -> {
                        if (System.currentTimeMillis() - lastReceived > IDLE_TIMEOUT_MS) break
                        sendFrame(Protocol.header("ping"))
                    }
                    is Event.Closed, is Event.Failed -> break
                    else -> Unit
                }
            }
        } finally {
            if (activeInbox === inbox) activeInbox = null
            ticker?.cancel()
            ws.cancel()
            inbox.close()
        }
    }

    private suspend fun handleIncoming(header: JSONObject, body: ByteArray) {
        val sha = header.getString("sha256")
        if (Crypto.sha256Hex(body) != sha) return
        if (_paused.value) return
        val mime = header.getString("mime")
        if (mime != Protocol.TEXT_MIME) return
        if (sha != lastHash) {
            lastHash = sha
            val clip = ClipData.newPlainText("Peegel", String(body))
            if (Build.VERSION.SDK_INT >= 33) {
                clip.description.extras = PersistableBundle().apply {
                    putBoolean(ClipDescription.EXTRA_IS_REMOTE_DEVICE, true)
                }
            }
            withContext(Dispatchers.Main) { clipboard.setPrimaryClip(clip) }
        }
        addHistory(header, body, fromPhone = false)
    }

    private fun addHistory(header: JSONObject, body: ByteArray, fromPhone: Boolean) {
        val mime = header.getString("mime")
        val item = ClipItem(
            id = header.getString("id"),
            mime = mime,
            text = if (mime == Protocol.TEXT_MIME) String(body).take(2_000) else "",
            size = body.size,
            fromPhone = fromPhone,
            ts = System.currentTimeMillis(),
        )
        _history.update { list ->
            (listOf(item) + list.filterNot { it.mime == item.mime && it.text == item.text && it.size == item.size }).take(50)
        }
    }

    companion object {
        private const val TAG = "PeegelSync"
        private const val PING_INTERVAL_MS = 20_000L
        private const val IDLE_TIMEOUT_MS = 60_000L
    }
}
