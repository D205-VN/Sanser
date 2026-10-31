package com.sanser.mobile

import android.content.Context
import android.graphics.Color
import android.media.*
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.view.*
import android.widget.FrameLayout
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlin.concurrent.thread
import kotlin.math.min

/** Bounded native PCM output. Decoder callbacks never wait for the device. */
internal class PcmOutput : AutoCloseable {
    private val running = AtomicBoolean(true)
    private val queue = ArrayBlockingQueue<ShortArray>(16)
    private val reset = AtomicBoolean(false)
    private val writer = thread(name="SanserAudio") {
        android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_AUDIO)
        var output: AudioTrack? = null
        try {
            while (running.get()) {
                val samples = queue.poll(100,java.util.concurrent.TimeUnit.MILLISECONDS) ?: continue
                if (output == null) {
                    val minimum = AudioTrack.getMinBufferSize(48000,AudioFormat.CHANNEL_OUT_STEREO,AudioFormat.ENCODING_PCM_16BIT)
                    output = AudioTrack.Builder().setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                        .setAudioFormat(AudioFormat.Builder().setSampleRate(48000).setChannelMask(AudioFormat.CHANNEL_OUT_STEREO).setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
                        .setBufferSizeInBytes(maxOf(minimum,19200)).setTransferMode(AudioTrack.MODE_STREAM).build()
                    output.play()
                }
                if (reset.getAndSet(false)) { output.pause();output.flush();output.play() }
                val track = output ?: continue
                var offset = 0
                while (running.get() && offset < samples.size) {
                    val n = track.write(samples,offset,samples.size-offset,AudioTrack.WRITE_NON_BLOCKING)
                    if(n < 0){ track.release();output=null;break }
                    if(n == 0) Thread.sleep(2) else offset += n
                }
            }
        } finally { output?.release() }
    }
    fun push(samples: ShortArray) { if(!queue.offer(samples)){queue.clear();reset.set(true);queue.offer(samples)} }
    override fun close() { running.set(false);queue.clear();writer.join(1500) }
}

internal class RemoteVideo(context: Context, private val client: () -> NativeClient?, private val rendered: () -> Unit) : FrameLayout(context), SurfaceHolder.Callback, AutoCloseable {
    private val surface = SurfaceView(context)
    private val decoderThread = HandlerThread("SanserVideo").apply { start() }
    private val decoder = Handler(decoderThread.looper)
    private var codec: MediaCodec? = null
    private var width = 0;private var height = 0
    @Volatile private var attached = false
    private val pending = AtomicLong(0)
    private val streamEpoch = AtomicLong(0)
    private val closing = AtomicBoolean(false)
    private val needsKeyframe = AtomicBoolean(true)
    private var firstFrame = false
    init {
        setBackgroundColor(Color.BLACK);addView(surface, LayoutParams(LayoutParams.MATCH_PARENT,LayoutParams.MATCH_PARENT,Gravity.CENTER))
        surface.holder.addCallback(this)
        setOnTouchListener { _, event ->
            if(width<=0 || height<=0)return@setOnTouchListener true
            val bounds = surface.layoutParams
            val left = (getWidth()-bounds.width)/2f;val top=(getHeight()-bounds.height)/2f
            if(event.x !in left..left+bounds.width || event.y !in top..top+bounds.height) return@setOnTouchListener true
            val x=((event.x-left)/bounds.width*65535).toInt().coerceIn(0,65535)
            val y=((event.y-top)/bounds.height*65535).toInt().coerceIn(0,65535)
            val native=client() ?: return@setOnTouchListener true
            native.input(0,x,y)
            when(event.actionMasked){
                MotionEvent.ACTION_DOWN -> native.input(1,x,y,code=0,down=true)
                MotionEvent.ACTION_UP,MotionEvent.ACTION_CANCEL -> native.input(1,x,y,code=0,down=false)
            }
            true
        }
    }
    override fun onSizeChanged(w:Int,h:Int,oldw:Int,oldh:Int){super.onSizeChanged(w,h,oldw,oldh);fit()}
    private fun fit(){
        if(width<=0 || height<=0 || getWidth()<=0 || getHeight()<=0)return
        val scale=min(getWidth().toFloat()/width,getHeight().toFloat()/height)
        surface.layoutParams=LayoutParams((width*scale).toInt(),(height*scale).toInt(),Gravity.CENTER)
    }
    override fun surfaceCreated(holder: SurfaceHolder){attached=true;needsKeyframe.set(true);client()?.keyframe()}
    override fun surfaceChanged(holder: SurfaceHolder,format:Int,width:Int,height:Int){}
    override fun surfaceDestroyed(holder: SurfaceHolder){attached=false;decoder.post{releaseCodec()};client()?.input(4)}
    fun push(bytes:ByteArray,w:Int,h:Int,keyframe:Boolean){
        if(closing.get() || !attached || bytes.size>4*1024*1024 || w !in 16..8192 || h !in 16..8192)return
        // Do not overwrite dependent H.264 frames and then feed broken references.
        if(pending.get()>=3){streamEpoch.incrementAndGet();needsKeyframe.set(true);decoder.post{client()?.keyframe()};return}
        val epoch = streamEpoch.get()
        pending.incrementAndGet()
        decoder.post {
            try {
                if(!attached || closing.get() || epoch!=streamEpoch.get())return@post
                if(needsKeyframe.get() && !keyframe)return@post
                if(codec==null || width!=w || height!=h){
                    releaseCodec();width=w;height=h
                    post{fit()}
                    val format=MediaFormat.createVideoFormat("video/avc",w,h).apply {
                        setInteger(MediaFormat.KEY_MAX_INPUT_SIZE,4*1024*1024)
                        if(Build.VERSION.SDK_INT>=30)setInteger(MediaFormat.KEY_LOW_LATENCY,1)
                    }
                    codec=MediaCodec.createDecoderByType("video/avc").apply {
                        configure(format,surface.holder.surface,null,0);start()
                        if(Build.VERSION.SDK_INT>=23)setOnFrameRenderedListener({_,_,_->if(!firstFrame){firstFrame=true;rendered()}},decoder)
                    }
                }
                val current=codec ?: return@post
                drain(current)
                val index=current.dequeueInputBuffer(2000)
                if(index<0){needsKeyframe.set(true);client()?.keyframe();return@post}
                val buffer=current.getInputBuffer(index) ?: return@post
                if(buffer.capacity()<bytes.size)throw IllegalArgumentException("Encoded frame too large")
                buffer.clear();buffer.put(bytes);current.queueInputBuffer(index,0,bytes.size,SystemClock.elapsedRealtimeNanos()/1000,0)
                if(keyframe && epoch==streamEpoch.get())needsKeyframe.set(false)
                drain(current)
                decoder.postDelayed({try{codec?.let{drain(it)}}catch(_:Exception){needsKeyframe.set(true)}},2)
            }catch(_:Exception){releaseCodec();needsKeyframe.set(true);client()?.keyframe()}
            finally {pending.decrementAndGet()}
        }
    }
    private fun drain(current:MediaCodec){val info=MediaCodec.BufferInfo();var count=0;while(count++<8){val index=current.dequeueOutputBuffer(info,0);if(index<0)return;current.releaseOutputBuffer(index,true)}}
    private fun releaseCodec(){try{codec?.stop()}catch(_:Exception){};try{codec?.release()}catch(_:Exception){};codec=null}
    override fun close(){if(closing.getAndSet(true))return;attached=false;surface.holder.removeCallback(this);decoder.post{releaseCodec();decoderThread.quitSafely()}}
}
