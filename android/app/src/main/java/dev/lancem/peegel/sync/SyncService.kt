package dev.lancem.peegel.sync

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import dev.lancem.peegel.R
import dev.lancem.peegel.app
import dev.lancem.peegel.ui.MainActivity
import dev.lancem.peegel.ui.SendActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach

/** Foreground service that keeps the connection to the computer alive. */
class SyncService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
        ServiceCompat.startForeground(
            this,
            NOTIFICATION_ID,
            buildNotification(app.sync.status.value),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
        )
        app.sync.status.onEach { status ->
            if (app.sync.store.server == null && status !is Status.Pairing) {
                stopSelf()
            } else {
                getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, buildNotification(status))
            }
        }.launchIn(scope)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            app.sync.stop()
            stopSelf()
            return START_NOT_STICKY
        }
        app.sync.start()
        app.sync.reconnectNow()
        return START_STICKY
    }

    override fun onDestroy() {
        scope.cancel()
        app.sync.stop()
        super.onDestroy()
    }

    private fun createChannel() {
        val channel = NotificationChannel(CHANNEL_ID, getString(R.string.channel_sync), NotificationManager.IMPORTANCE_LOW).apply {
            description = getString(R.string.channel_sync_description)
            setShowBadge(false)
        }
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun buildNotification(status: Status): Notification {
        val (title, text) = when (status) {
            is Status.Connected -> "Connected to ${status.name}" to "Copies from your computer land here automatically"
            is Status.Connecting -> "Looking for ${status.name}…" to "Make sure both devices are on the same Wi‑Fi"
            is Status.Pairing -> "Pairing with ${status.name}…" to ""
            is Status.Error -> "Peegel" to status.message
            Status.NotPaired -> "Peegel" to "Not paired"
        }
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
        )
        val send = PendingIntent.getActivity(
            this, 1, SendActivity.clipboardIntent(this), PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_send_clipboard)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .setSilent(true)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .addAction(R.drawable.ic_send_clipboard, getString(R.string.action_send_clipboard), send)
            .build()
    }

    companion object {
        private const val CHANNEL_ID = "sync"
        private const val NOTIFICATION_ID = 1
        private const val ACTION_STOP = "dev.lancem.peegel.STOP"

        fun start(context: Context) {
            ContextCompat.startForegroundService(context, Intent(context, SyncService::class.java))
        }
    }
}
