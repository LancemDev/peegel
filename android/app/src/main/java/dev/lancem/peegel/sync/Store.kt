package dev.lancem.peegel.sync

import android.content.Context
import android.os.Build
import android.provider.Settings
import androidx.core.content.edit
import dev.lancem.peegel.crypto.Crypto

data class PairedServer(
    val id: String,
    val name: String,
    val hosts: List<String>,
    val port: Int,
    val key: ByteArray,
)

/** Small persistent store backed by private SharedPreferences. */
class Store(private val context: Context) {
    private val prefs = context.getSharedPreferences("peegel", Context.MODE_PRIVATE)

    val deviceId: String =
        prefs.getString("device_id", null) ?: Crypto.b64(Crypto.randomBytes(16)).also {
            prefs.edit { putString("device_id", it) }
        }

    val deviceName: String
        get() = Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)
            ?.takeIf { it.isNotBlank() }
            ?: Build.MODEL

    var paused: Boolean
        get() = prefs.getBoolean("paused", false)
        set(value) = prefs.edit { putBoolean("paused", value) }

    var server: PairedServer?
        get() {
            val id = prefs.getString("server_id", null) ?: return null
            return PairedServer(
                id = id,
                name = prefs.getString("server_name", null) ?: "Computer",
                hosts = prefs.getString("server_hosts", "")!!.split(",").filter(String::isNotEmpty),
                port = prefs.getInt("server_port", 47820),
                key = Crypto.unb64(prefs.getString("server_key", null) ?: return null),
            )
        }
        set(value) = prefs.edit {
            if (value == null) {
                remove("server_id"); remove("server_name"); remove("server_hosts")
                remove("server_port"); remove("server_key")
            } else {
                putString("server_id", value.id)
                putString("server_name", value.name)
                putString("server_hosts", value.hosts.joinToString(","))
                putInt("server_port", value.port)
                putString("server_key", Crypto.b64(value.key))
            }
        }
}
