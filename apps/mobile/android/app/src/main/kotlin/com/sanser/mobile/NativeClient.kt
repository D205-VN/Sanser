package com.sanser.mobile
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/** No compressed video or PCM crosses Flutter method/event channels. */
class NativeClient(
    val video: (ByteArray, Int, Int, Boolean) -> Unit,
    val audio: (ShortArray) -> Unit,
    val stopped: (Int) -> Unit
) : AutoCloseable {
    companion object { init { System.loadLibrary("sanser_mobile") } }
    private var handle = 0L
    private val clipboardReplies=ConcurrentHashMap<Long,(Int,String)->Unit>()
    private val clipboardSequence=AtomicLong()
    private external fun create(endpoint: String, port: Int, token: String, control: Boolean, sharing: Boolean, viewer: String, files: Boolean): Long
    private external fun destroy(handle: Long)
    private external fun videoLimit(handle:Long,bitrate:Int,baseline:Double)
    private external fun input(handle: Long, kind: Int, x: Int, y: Int, code: Int, delta: Int, down: Boolean)
    private external fun mute(handle: Long, value: Boolean)
    private external fun text(handle: Long, utf8: ByteArray): Boolean
    private external fun textAvailable(handle: Long): Boolean
    private external fun clipboardAvailable(handle: Long): Boolean
    private external fun gamepadCapture(handle:Long,capture:Long,enabled:Boolean):Boolean
    private external fun gamepadStatus(handle:Long,capture:Long):Int
    private external fun gamepadSample(handle:Long,capture:Long,connected:Boolean,sampled:Boolean,buttons:Int,lx:Double,ly:Double,rx:Double,ry:Double,lt:Double,rt:Double):Boolean
    @Synchronized fun gamepadCapture(capture:Long,enabled:Boolean):Boolean=handle!=0L && gamepadCapture(handle,capture,enabled)
    @Synchronized fun gamepadStatus(capture:Long=0):Int=if(handle!=0L)gamepadStatus(handle,capture)else 0
    @Synchronized fun gamepadSample(capture:Long,s:Map<String,Any>):Boolean {
        if(handle==0L)return false
        fun n(key:String)=(s[key] as? Number)?.toDouble() ?: Double.NaN
        return gamepadSample(handle,capture,s["connected"]==true,s["sample"]==true,(s["buttons"] as? Number)?.toInt() ?: -1,
            n("lx"),n("ly"),n("rx"),n("ry"),n("lt"),n("rt"))
    }
    private external fun clipboard(handle:Long,request:Long,write:Boolean,utf8:ByteArray):Boolean
    private external fun filePump(handle:Long,files:Long)
    @Synchronized fun pumpFiles(files:Long){if(handle!=0L && files!=0L)filePump(handle,files)}
    private external fun keyframe(handle: Long)
    private external fun rtt(handle: Long): Double
    private external fun stats(handle:Long):LongArray
    @Synchronized fun start(endpoint: String, token: String, control: Boolean, sharing: Boolean, viewer: String, port: Int = 0, bitrate:Int = 6000000, baseline:Double = 0.0, files:Boolean = false) {
        require(port in 0..65535)
        check(handle == 0L); handle = create(endpoint, port, token, control, sharing, viewer, files)
        check(handle != 0L) { "Không thể khởi động engine SNV2." }
        videoLimit(handle,bitrate,baseline)
    }
    @Synchronized fun input(kind: Int, x: Int = 0, y: Int = 0, code: Int = 0, delta: Int = 0, down: Boolean = false) {
        if (handle != 0L) input(handle, kind, x, y, code, delta, down)
    }
    @Synchronized fun mute(value: Boolean) { if (handle != 0L) mute(handle, value) }
    @Synchronized fun text(value: String): Boolean = handle != 0L && text(handle,value.toByteArray(Charsets.UTF_8))
    @Synchronized fun textAvailable(): Boolean = handle != 0L && textAvailable(handle)
    @Synchronized fun clipboardAvailable():Boolean=handle!=0L && clipboardAvailable(handle)
    @Synchronized fun clipboard(write:Boolean,text:String,reply:(Int,String)->Unit):Boolean {
        if(handle==0L)return false
        val bytes=text.toByteArray(Charsets.UTF_8);if(bytes.size>8192)return false
        val id=clipboardSequence.incrementAndGet();clipboardReplies[id]=reply
        val accepted=clipboard(handle,id,write,bytes)
        if(!accepted)clipboardReplies.remove(id)
        return accepted
    }
    @Synchronized fun keyframe() { if (handle != 0L) keyframe(handle) }
    @Synchronized fun rtt(): Double = if (handle == 0L) -1.0 else rtt(handle)
    @Synchronized fun stats():Map<String,Long> {
        if(handle==0L)return emptyMap()
        val values=stats(handle);val names=listOf("videoPackets","videoFrames","nackRequests","lostFragments","referenceDrops","audioDecoded","audioConcealed","audioLate")
        return names.mapIndexed{index,name->name to values[index]}.toMap()
    }
    @Synchronized override fun close() { val old = handle; handle = 0L; if (old != 0L) destroy(old) }
    // Called from native workers; these methods never acquire the handle lock.
    fun onVideo(bytes: ByteArray, width: Int, height: Int, keyframe: Boolean) = video(bytes, width, height, keyframe)
    fun onAudio(samples: ShortArray) = audio(samples)
    fun onStopped(reason: Int) = stopped(reason)
    fun onClipboard(request:Long,status:Int,utf8:ByteArray){clipboardReplies.remove(request)?.invoke(status,String(utf8,Charsets.UTF_8))}
}
