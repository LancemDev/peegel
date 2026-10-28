package dev.lancem.peegel.sync

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.util.Log
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import java.net.Inet4Address
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.resume

/** Finds a paired computer on the local network via mDNS (DNS-SD). */
class Discovery(context: Context) {
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val wifi = context.applicationContext.getSystemService(WifiManager::class.java)
    private val executor = Executors.newSingleThreadExecutor()

    data class Found(val host: String, val port: Int)

    suspend fun find(serverId: String, timeoutMs: Long): Found? = withTimeoutOrNull(timeoutMs) {
        suspendCancellableCoroutine { cont ->
            val done = AtomicBoolean(false)
            val lock = wifi.createMulticastLock("peegel-discovery").apply {
                setReferenceCounted(false)
                acquire()
            }
            val callbacks = mutableListOf<NsdManager.ServiceInfoCallback>()

            lateinit var discoveryListener: NsdManager.DiscoveryListener
            fun cleanup() {
                runCatching { nsd.stopServiceDiscovery(discoveryListener) }
                if (Build.VERSION.SDK_INT >= 34) {
                    synchronized(callbacks) {
                        callbacks.forEach { runCatching { nsd.unregisterServiceInfoCallback(it) } }
                    }
                }
                runCatching { lock.release() }
            }

            fun check(info: NsdServiceInfo) {
                val id = info.attributes["id"]?.let { String(it) }
                if (id != serverId) return
                val host = if (Build.VERSION.SDK_INT >= 34) {
                    info.hostAddresses.firstOrNull { it is Inet4Address } ?: info.hostAddresses.firstOrNull()
                } else {
                    @Suppress("DEPRECATION") info.host
                } ?: return
                if (done.compareAndSet(false, true)) {
                    cleanup()
                    cont.resume(Found(host.hostAddress!!, info.port))
                }
            }

            discoveryListener = object : NsdManager.DiscoveryListener {
                override fun onServiceFound(info: NsdServiceInfo) {
                    if (done.get()) return
                    if (Build.VERSION.SDK_INT >= 34) {
                        val callback = object : NsdManager.ServiceInfoCallback {
                            override fun onServiceUpdated(updated: NsdServiceInfo) = check(updated)
                            override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {}
                            override fun onServiceLost() {}
                            override fun onServiceInfoCallbackUnregistered() {}
                        }
                        synchronized(callbacks) { callbacks += callback }
                        runCatching { nsd.registerServiceInfoCallback(info, executor, callback) }
                    } else {
                        @Suppress("DEPRECATION")
                        nsd.resolveService(info, object : NsdManager.ResolveListener {
                            override fun onServiceResolved(resolved: NsdServiceInfo) = check(resolved)
                            override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {}
                        })
                    }
                }

                override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                    Log.w(TAG, "discovery failed to start: $errorCode")
                }

                override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
                override fun onDiscoveryStarted(serviceType: String) {}
                override fun onDiscoveryStopped(serviceType: String) {}
                override fun onServiceLost(info: NsdServiceInfo) {}
            }

            nsd.discoverServices(Protocol.SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, discoveryListener)
            cont.invokeOnCancellation {
                if (done.compareAndSet(false, true)) cleanup()
            }
        }
    }

    private companion object {
        const val TAG = "PeegelDiscovery"
    }
}
