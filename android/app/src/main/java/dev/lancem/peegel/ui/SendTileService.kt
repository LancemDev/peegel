package dev.lancem.peegel.ui

import android.annotation.SuppressLint
import android.app.PendingIntent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import dev.lancem.peegel.app
import dev.lancem.peegel.sync.Status

/** Quick Settings tile: one tap sends the current clipboard to the computer. */
class SendTileService : TileService() {

    override fun onStartListening() {
        super.onStartListening()
        val tile = qsTile ?: return
        val status = app.sync.status.value
        tile.state = if (status is Status.Connected) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.subtitle = when (status) {
            is Status.Connected -> status.name
            is Status.Connecting -> "Not connected"
            else -> "Not paired"
        }
        tile.updateTile()
    }

    @SuppressLint("StartActivityAndCollapseDeprecated")
    override fun onClick() {
        super.onClick()
        val intent = SendActivity.clipboardIntent(this)
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(
                PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE),
            )
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }
}
