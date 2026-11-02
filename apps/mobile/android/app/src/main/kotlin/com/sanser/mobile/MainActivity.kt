package com.sanser.mobile

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.view.View
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.EventChannel
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.StandardMessageCodec
import io.flutter.plugin.platform.PlatformView
import io.flutter.plugin.platform.PlatformViewFactory
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicLong

class MainActivity : FlutterActivity() {
    private val main = Handler(Looper.getMainLooper())
    private val work = Executors.newSingleThreadExecutor()
    private val generation = AtomicLong(0)
    @Volatile private var client: NativeClient? = null
    @Volatile private var video: RemoteVideo? = null
    private var audio: PcmOutput? = null
    private var relay: Relay? = null
    private var sink: EventChannel.EventSink? = null
    private var sessionId: String? = null
    private var frameShown = false
    private var videoWidth=0;private var videoHeight=0
    private var startup: Runnable? = null
    private var channels: MethodChannel? = null
    private var events: EventChannel? = null
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        flutterEngine.platformViewsController.registry.registerViewFactory("com.sanser.mobile/video",object:PlatformViewFactory(StandardMessageCodec.INSTANCE){
            override fun create(context:Context,viewId:Int,args:Any?):PlatformView {
                val native = RemoteVideo(context,{client}) { main.post { if(!frameShown && sessionId!=null){frameShown=true;startup?.let{main.removeCallbacks(it)};emit("frame")} } }
                video=native
                return object:PlatformView {
                    override fun getView():View = native
                    override fun dispose(){native.close();if(video===native)video=null}
                }
            }
        })
        events=EventChannel(flutterEngine.dartExecutor.binaryMessenger,"com.sanser.mobile/events").also { channel ->
            channel.setStreamHandler(object:EventChannel.StreamHandler {
                override fun onListen(arguments:Any?,eventSink:EventChannel.EventSink){sink=eventSink}
                override fun onCancel(arguments:Any?){sink=null}
            })
        }
        channels=MethodChannel(flutterEngine.dartExecutor.binaryMessenger,"com.sanser.mobile/engine").also { channel ->
            channel.setMethodCallHandler { call,result ->
                when(call.method){
                    "capabilities" -> result.success(mapOf("available" to true,"video" to "h264","audio" to true,"voice" to false,"transport" to "auto","direct" to true))
                    "connect" -> {
                        @Suppress("UNCHECKED_CAST") val configuration=call.arguments as? Map<String,Any?>
                        if(configuration==null){result.error("configuration","Thiếu cấu hình phiên.",null);return@setMethodCallHandler}
                        val epoch=generation.incrementAndGet()
                        work.execute {
                            stopNative()
                            try {
                                val id=configuration["sessionId"] as String
                                main.post {
                                    if(epoch==generation.get()){
                                        sessionId=id;frameShown=false
                                        startup=Runnable{if(epoch==generation.get() && !frameShown)fail("Không nhận được hình trong 25 giây. Kiểm tra máy Host rồi kết nối lại.")}.also{main.postDelayed(it,25000)}
                                    }
                                }
                                if(configuration["transport"]=="direct"){
                                    val endpoint=configuration["endpoint"] as? String ?: error("Missing Direct endpoint")
                                    require(endpoint.startsWith("127.0.0.1:"))
                                    require(((configuration["localPort"] as? Number)?.toInt() ?: 0) in 1..65535)
                                    startClient(endpoint,configuration,epoch)
                                }else{
                                    val sessionRelay=Relay(configuration,{ endpoint ->
                                        work.execute {
                                            if(epoch!=generation.get())return@execute
                                            try {
                                                startClient(endpoint,configuration,epoch)
                                            }catch(_:Exception){main.post{if(epoch==generation.get())fail("Không thể mở bộ giải mã native.")}}
                                        }
                                    },{message->main.post{if(epoch==generation.get())fail(message)}})
                                    relay=sessionRelay
                                    sessionRelay.start()
                                }
                                main.post{result.success(null)}
                            }catch(_:Exception){stopNative();main.post{result.error("connection","Không thể bắt đầu phiên mobile.",null)}}
                        }
                    }
                    "p2p" -> {
                        @Suppress("UNCHECKED_CAST") val args=call.arguments as? Map<String,Any?> ?: emptyMap()
                        val operation=Runnable{
                            try{val reply=MobileP2p.call(args);main.post{result.success(reply)}}
                            catch(error:Exception){main.post{result.error("direct",error.message ?: "Direct failed",null)}}
                        }
                        if(args["operation"]=="close")operation.run() else work.execute(operation)
                    }
                    "disconnect" -> {generation.incrementAndGet();startup?.let{main.removeCallbacks(it)};sessionId=null;frameShown=false;work.execute{stopNative();main.post{result.success(null)}}}
                    "mute" -> {client?.mute(call.argument<Boolean>("muted")==true);result.success(null)}
                    "key" -> {client?.input(2,code=call.argument<Int>("code")?:0,down=call.argument<Boolean>("down")==true);result.success(null)}
                    "resetInput" -> {client?.input(4);result.success(null)}
                    "input" -> {
                        if(sessionId!=null && call.argument<String>("sessionId")==sessionId){
                            val kind=call.argument<Int>("kind") ?: 4
                            if(kind in 0..4)client?.input(kind,call.argument<Int>("x")?:0,call.argument<Int>("y")?:0,
                                call.argument<Int>("code")?:0,call.argument<Int>("delta")?:0,call.argument<Boolean>("down")==true)
                        }
                        result.success(null)
                    }
                    "viewport" -> {
                        if(sessionId!=null && call.argument<String>("sessionId")==sessionId)
                            video?.viewport((call.argument<Number>("scale")?:1).toFloat(),(call.argument<Number>("x")?:0).toFloat(),(call.argument<Number>("y")?:0).toFloat())
                        result.success(null)
                    }
                    "voice" -> result.error("voice_unavailable","Voice mobile chưa sẵn sàng trong bản preview.",null)
                    else -> result.notImplemented()
                }
            }
        }
    }
    private fun startClient(endpoint:String,configuration:Map<String,Any?>,epoch:Long){
        if(epoch!=generation.get())return
        val output=PcmOutput();audio=output
        var previousWidth=0;var previousHeight=0
        val native=NativeClient({bytes,w,h,key->if(epoch==generation.get()){
            video?.push(bytes,w,h,key)
            if(previousWidth!=w || previousHeight!=h){previousWidth=w;previousHeight=h;main.post{
                if(epoch==generation.get()){videoWidth=w;videoHeight=h;emit("dimensions")}
            }}
        }},
            {samples->if(epoch==generation.get())output.push(samples)},
            {reason->main.post{if(epoch==generation.get())fail("Engine native đã dừng ($reason).")}})
        client=native
        native.start(endpoint,configuration["sessionToken"] as String,configuration["inputEnabled"]==true,configuration["sharingId"]!=null,configuration["viewerId"] as? String ?: "",(configuration["localPort"] as? Number)?.toInt() ?: 0,((configuration["videoBitrateMbps"] as? Number)?.toInt() ?: 6)*1000000,(configuration["baselineRttMs"] as? Number)?.toDouble() ?: 0.0)
        native.mute(configuration["audioMuted"]==true)
        main.post { if(epoch==generation.get()){emit("connected");poll(epoch)} }
    }
    private fun emit(state:String,message:String?=null,rtt:Double?=null){val id=sessionId?:return;sink?.success(mapOf("sessionId" to id,"state" to state,"message" to message,"rttMs" to rtt,"videoWidth" to videoWidth,"videoHeight" to videoHeight)+(client?.stats()?:emptyMap()))}
    private fun poll(epoch:Long){
        main.postDelayed({if(epoch==generation.get() && sessionId!=null){val rtt=client?.rtt();emit(if(frameShown)"frame" else "connecting",rtt=rtt?.takeIf{it>=0});poll(epoch)}},1000)
    }
    private fun fail(message:String){emit("stopped",message);generation.incrementAndGet();sessionId=null;startup?.let{main.removeCallbacks(it)};work.execute{stopNative()}}
    private fun stopNative(){
        val native=client;client=null;native?.input(4);native?.close()
        video?.resetStream()
        relay?.close();relay=null;audio?.close();audio=null
    }
    override fun onStop(){client?.input(4);super.onStop()}
    override fun onDestroy(){generation.incrementAndGet();sink=null;startup?.let{main.removeCallbacks(it)};channels?.setMethodCallHandler(null);events?.setStreamHandler(null);work.execute{stopNative()};work.shutdown();super.onDestroy()}
}
