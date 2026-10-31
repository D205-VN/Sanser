package com.sanser.mobile

/** No compressed video or PCM crosses Flutter method/event channels. */
class NativeClient(
    val video: (ByteArray, Int, Int, Boolean) -> Unit,
    val audio: (ShortArray) -> Unit,
    val stopped: (Int) -> Unit
) : AutoCloseable {
    companion object { init { System.loadLibrary("sanser_mobile") } }
    private var handle = 0L
    private external fun create(endpoint: String, token: String, control: Boolean, sharing: Boolean, viewer: String): Long
    private external fun destroy(handle: Long)
    private external fun input(handle: Long, kind: Int, x: Int, y: Int, code: Int, delta: Int, down: Boolean)
    private external fun mute(handle: Long, value: Boolean)
    private external fun keyframe(handle: Long)
    private external fun rtt(handle: Long): Double
    @Synchronized fun start(endpoint: String, token: String, control: Boolean, sharing: Boolean, viewer: String) {
        check(handle == 0L); handle = create(endpoint, token, control, sharing, viewer)
        check(handle != 0L) { "Không thể khởi động engine SNV2." }
    }
    @Synchronized fun input(kind: Int, x: Int = 0, y: Int = 0, code: Int = 0, delta: Int = 0, down: Boolean = false) {
        if (handle != 0L) input(handle, kind, x, y, code, delta, down)
    }
    @Synchronized fun mute(value: Boolean) { if (handle != 0L) mute(handle, value) }
    @Synchronized fun keyframe() { if (handle != 0L) keyframe(handle) }
    @Synchronized fun rtt(): Double = if (handle == 0L) -1.0 else rtt(handle)
    @Synchronized override fun close() { val old = handle; handle = 0L; if (old != 0L) destroy(old) }
    // Called from native workers; these methods never acquire the handle lock.
    fun onVideo(bytes: ByteArray, width: Int, height: Int, keyframe: Boolean) = video(bytes, width, height, keyframe)
    fun onAudio(samples: ShortArray) = audio(samples)
    fun onStopped(reason: Int) = stopped(reason)
}
