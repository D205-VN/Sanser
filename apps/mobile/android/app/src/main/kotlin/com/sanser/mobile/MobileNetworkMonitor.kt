package com.sanser.mobile

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Handler

/** Default-route notifications, independent of media lifetime. Never expose
 * addresses, network handles, interface names or Wi-Fi identifiers to Dart. */
internal class MobileNetworkMonitor(context: Context, private val main: Handler,
    private val changed: (Boolean, Boolean, Long) -> Unit) : AutoCloseable {
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private var current: Network? = null
    private var internet = false
    private var links: String? = null
    private var previous: String? = null
    private var revision = 0L
    private var closed = false
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            current = network; internet = false; links = null
            // API 26+ delivers capabilities/link properties immediately after
            // this callback. Do not query synchronous snapshots here.
        }
        override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
            if (closed || network != current) return
            internet = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            publish()
        }
        override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) {
            if (closed || network != current) return
            links = "${properties.interfaceName}|${properties.linkAddresses.sortedBy { it.toString() }}|${properties.routes}"
            publish()
        }
        override fun onLost(network: Network) {
            if (closed || network != current) return
            current = null; internet = false; links = null; publish()
        }
    }
    init { manager.registerDefaultNetworkCallback(callback, main) }
    private fun publish() {
        if (closed || (current != null && links == null)) return
        val fingerprint = "$current|$internet|$links"
        if (fingerprint == previous) return
        val wasInitialized = previous != null
        previous = fingerprint; revision++
        changed(current != null && internet, wasInitialized, revision)
    }
    override fun close() {
        closed = true
        try { manager.unregisterNetworkCallback(callback) } catch (_: IllegalArgumentException) { }
    }
}
