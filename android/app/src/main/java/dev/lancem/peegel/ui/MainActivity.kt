package dev.lancem.peegel.ui

import android.Manifest
import android.annotation.SuppressLint
import android.app.StatusBarManager
import android.content.ClipboardManager
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import dev.lancem.peegel.R
import dev.lancem.peegel.app
import dev.lancem.peegel.sync.PairInfo
import dev.lancem.peegel.sync.Protocol
import dev.lancem.peegel.sync.Status
import dev.lancem.peegel.sync.SyncService

class MainActivity : ComponentActivity() {
    private var setup by mutableStateOf(SetupState())
    private var pendingPair by mutableStateOf<PairInfo?>(null)

    private val notificationPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { refreshSetup() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        handleDeepLink(intent)
        setContent {
            PeegelTheme {
                val status by app.sync.status.collectAsStateWithLifecycle()
                val history by app.sync.history.collectAsStateWithLifecycle()
                val paused by app.sync.paused.collectAsStateWithLifecycle()
                PeegelScreen(
                    status = status,
                    paired = status !is Status.NotPaired && app.sync.store.server != null,
                    history = history,
                    paused = paused,
                    setup = setup,
                    pendingPair = pendingPair,
                    actions = ScreenActions(
                        scan = ::scanQr,
                        pairWithLink = ::pairWithLink,
                        confirmPair = { info -> pendingPair = null; startPairing(info) },
                        cancelPair = { pendingPair = null },
                        sendClipboard = ::sendClipboard,
                        recopy = { item ->
                            app.sync.recopy(item)
                        },
                        setPaused = app.sync::setPaused,
                        unpair = app.sync::unpair,
                        allowNotifications = {
                            notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
                        },
                        allowBackground = ::requestIgnoreBatteryOptimizations,
                        addTile = ::requestAddTile,
                    ),
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleDeepLink(intent)
    }

    override fun onResume() {
        super.onResume()
        refreshSetup()
        if (app.sync.store.server != null) SyncService.start(this)
    }

    /** Pairing links opened from the camera app ask for confirmation first. */
    private fun handleDeepLink(intent: Intent?) {
        val data = intent?.data?.toString() ?: return
        PairInfo.parse(data)?.let { pendingPair = it }
    }

    private fun refreshSetup() {
        val notificationsGranted = Build.VERSION.SDK_INT < 33 ||
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        val power = getSystemService(PowerManager::class.java)
        setup = SetupState(
            notificationsGranted = notificationsGranted,
            backgroundAllowed = power.isIgnoringBatteryOptimizations(packageName),
            canAddTile = Build.VERSION.SDK_INT >= 33,
        )
    }

    private fun scanQr() {
        val options = GmsBarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .enableAutoZoom()
            .build()
        GmsBarcodeScanning.getClient(this, options).startScan()
            .addOnSuccessListener { barcode -> pairWithLink(barcode.rawValue.orEmpty()) }
            .addOnFailureListener { error ->
                Toast.makeText(this, "Scanner unavailable: ${error.message}", Toast.LENGTH_LONG).show()
            }
    }

    private fun pairWithLink(link: String): Boolean {
        val info = PairInfo.parse(link)
        if (info == null) {
            Toast.makeText(this, "That isn't a Peegel pairing code", Toast.LENGTH_LONG).show()
            return false
        }
        startPairing(info)
        return true
    }

    private fun startPairing(info: PairInfo) {
        if (Build.VERSION.SDK_INT >= 33 && !setup.notificationsGranted) {
            notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        app.sync.pair(info)
        SyncService.start(this)
    }

    private fun sendClipboard() {
        val clipboard = getSystemService(ClipboardManager::class.java)
        val text = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this)?.toString()
        val message = when {
            text.isNullOrEmpty() -> "Clipboard is empty"
            app.sync.send(Protocol.TEXT_MIME, text.toByteArray()) -> "Sent"
            else -> "Not connected to your computer"
        }
        Toast.makeText(this, message, Toast.LENGTH_SHORT).show()
    }

    @SuppressLint("BatteryLife")
    private fun requestIgnoreBatteryOptimizations() {
        val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:$packageName"))
        runCatching { startActivity(intent) }
            .onFailure { startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)) }
    }

    private fun requestAddTile() {
        if (Build.VERSION.SDK_INT < 33) return
        getSystemService(StatusBarManager::class.java).requestAddTileService(
            ComponentName(this, SendTileService::class.java),
            getString(R.string.tile_label),
            Icon.createWithResource(this, R.drawable.ic_send_clipboard),
            mainExecutor,
        ) { result ->
            val message = when (result) {
                StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ADDED -> "Tile added to Quick Settings"
                StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ALREADY_ADDED -> "The tile is already in Quick Settings"
                else -> null
            }
            message?.let { Toast.makeText(this, it, Toast.LENGTH_SHORT).show() }
        }
    }
}
