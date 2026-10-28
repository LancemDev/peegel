package dev.lancem.peegel.ui

import android.app.Activity
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.widget.Toast
import dev.lancem.peegel.app
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
            val text = intent.getStringExtra(Intent.EXTRA_TEXT)
            if (text.isNullOrEmpty()) finishWith("Nothing to send") else send(Protocol.TEXT_MIME, text.toByteArray())
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
        val item = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)
        val text = item?.coerceToText(this)?.toString()
        if (text.isNullOrEmpty()) {
            finishWith("Clipboard is empty")
        } else {
            send(Protocol.TEXT_MIME, text.toByteArray())
        }
    }

    private fun send(mime: String, body: ByteArray) {
        val sync = app.sync
        if (sync.send(mime, body)) {
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
