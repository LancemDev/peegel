package dev.lancem.peegel.ui

import android.app.Activity
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.widget.Toast
import androidx.core.content.IntentCompat
import dev.lancem.peegel.app
import dev.lancem.peegel.sync.ClipFiles
import dev.lancem.peegel.sync.Protocol
import dev.lancem.peegel.sync.Status
import dev.lancem.peegel.sync.SyncService

/**
 * Invisible activity that sends the clipboard (or shared content) to the
 * computer. Android 10+ only lets the app in focus read the clipboard, so the
 * Quick Settings tile and notification action open this briefly.
 */
class SendActivity : Activity() {
    private val handler = Handler(Looper.getMainLooper())
    private var handled = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (intent?.action == Intent.ACTION_SEND) {
            handled = true
            val stream = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
            val text = intent.getStringExtra(Intent.EXTRA_TEXT)
            val content = when {
                stream != null -> ClipFiles.readImage(this, stream)
                !text.isNullOrEmpty() -> ClipFiles.Content(Protocol.TEXT_MIME, text.toByteArray())
                else -> null
            }
            if (content == null) finishWith("Nothing to send (images up to 20 MB)") else send(content)
        } else {
            // Reading the clipboard has to wait until this window has focus.
            handler.postDelayed({ if (!handled) finishWith("Couldn't read the clipboard") }, 1_500)
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || handled) return
        handled = true
        val clipboard = getSystemService(ClipboardManager::class.java)
        val content = ClipFiles.read(this, clipboard.primaryClip)
        if (content == null) finishWith("Clipboard is empty") else send(content)
    }

    private fun send(content: ClipFiles.Content) {
        val sync = app.sync
        if (sync.send(content.mime, content.body)) {
            val name = (sync.status.value as? Status.Connected)?.name ?: "computer"
            finishWith("Sent to $name")
        } else {
            if (sync.store.server != null) SyncService.start(this)
            finishWith("Not connected to your computer")
        }
    }

    private fun finishWith(message: String) {
        handler.removeCallbacksAndMessages(null)
        Toast.makeText(applicationContext, message, Toast.LENGTH_SHORT).show()
        finish()
    }

    companion object {
        private const val ACTION_SEND_CLIPBOARD = "dev.lancem.peegel.SEND_CLIPBOARD"

        fun clipboardIntent(context: Context): Intent =
            Intent(context, SendActivity::class.java)
                .setAction(ACTION_SEND_CLIPBOARD)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)
    }
}
