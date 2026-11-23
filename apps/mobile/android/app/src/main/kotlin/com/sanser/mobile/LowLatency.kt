package com.sanser.mobile

import android.content.Context
import android.media.MediaCodec
import android.media.MediaCodecList
import android.media.MediaFormat
import android.net.wifi.WifiManager
import android.os.Build
import android.view.Surface

/**
 * Keeps phone Wi-Fi out of power saving while a session runs. Without it the
 * radio dozes between packets and frames arrive in 50-200 ms bursts.
 */
internal class StreamingWifiLock(context: Context) {
    private val lock: WifiManager.WifiLock? = try {
        val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
        @Suppress("DEPRECATION")
        val mode = if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY
            else WifiManager.WIFI_MODE_FULL_HIGH_PERF
        wifi?.createWifiLock(mode, "Sanser:streaming")?.apply { setReferenceCounted(false) }
    } catch (_: Exception) { null }

    @Synchronized fun hold(on: Boolean) {
        val current = lock ?: return
        try { if (on && !current.isHeld) current.acquire() else if (!on && current.isHeld) current.release() }
        catch (_: Exception) {}
    }
}

/**
 * Creates an H.264 decoder with the low-latency options that each chip family
 * understands (the same keys Moonlight uses). Some decoders reject unknown
 * keys, so each step falls back to fewer options, ending with the plain format.
 */
internal object LowLatencyDecoder {
    fun create(width: Int, height: Int, surface: Surface): MediaCodec {
        val name = decoderName()
        val lower = name?.lowercase() ?: ""
        val attempts = listOf(4, 3, 2, 1, 0)
        var last: Exception? = null
        for (level in attempts) {
            val codec = if (name != null) MediaCodec.createByCodecName(name) else MediaCodec.createDecoderByType("video/avc")
            try {
                codec.configure(format(width, height, lower, level), surface, null, 0)
                codec.start()
                return codec
            } catch (error: Exception) {
                last = error
                try { codec.release() } catch (_: Exception) {}
            }
        }
        throw last ?: IllegalStateException("No H.264 decoder")
    }

    private fun decoderName(): String? = try {
        val probe = MediaFormat.createVideoFormat("video/avc", 1920, 1080)
        MediaCodecList(MediaCodecList.REGULAR_CODECS).findDecoderForFormat(probe)
    } catch (_: Exception) { null }

    /** level 0: plain format; each higher level adds more options. */
    internal fun format(width: Int, height: Int, decoder: String, level: Int): MediaFormat =
        MediaFormat.createVideoFormat("video/avc", width, height).apply {
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 4 * 1024 * 1024)
            if (level == 0) return@apply
            if (Build.VERSION.SDK_INT >= 30) setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            setInteger("low-latency", 1)
            if (level >= 2 && Build.VERSION.SDK_INT >= 23) {
                val qualcomm = decoder.startsWith("omx.qcom") || decoder.startsWith("c2.qti")
                // Qualcomm runs flat out with an operating rate; others get
                // real-time priority. Both together can crash some decoders.
                if (qualcomm) setInteger(MediaFormat.KEY_OPERATING_RATE, Short.MAX_VALUE.toInt())
                else setInteger(MediaFormat.KEY_PRIORITY, 0)
            }
            if (level >= 3 && Build.VERSION.SDK_INT >= 26) vendorKeys(this, decoder)
            if (level >= 4) setInteger("vdec-lowlatency", 1)
        }

    private fun vendorKeys(format: MediaFormat, decoder: String) {
        when {
            decoder.startsWith("omx.qcom") || decoder.startsWith("c2.qti") -> {
                format.setInteger("vendor.qti-ext-dec-picture-order.enable", 1)
                format.setInteger("vendor.qti-ext-dec-low-latency.enable", 1)
            }
            decoder.startsWith("omx.hisi") || decoder.startsWith("c2.hisi") -> {
                format.setInteger("vendor.hisi-ext-low-latency-video-dec.video-scene-for-low-latency-req", 1)
                format.setInteger("vendor.hisi-ext-low-latency-video-dec.video-scene-for-low-latency-rdy", -1)
            }
            decoder.startsWith("omx.exynos") || decoder.startsWith("c2.exynos") ->
                format.setInteger("vendor.rtc-ext-dec-low-latency.enable", 1)
            decoder.startsWith("omx.amlogic") || decoder.startsWith("c2.amlogic") ->
                format.setInteger("vendor.low-latency.enable", 1)
        }
    }
}
