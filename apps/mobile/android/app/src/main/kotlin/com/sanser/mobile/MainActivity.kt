package com.sanser.mobile

import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.view.View
import android.view.KeyEvent
import android.view.MotionEvent
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
    private val fileActions=Executors.newSingleThreadExecutor()
    private val generation = AtomicLong(0)
    private val documents by lazy { MobileFileDocuments(this) }
    @Volatile private var filesHandle=0L
    @Volatile private var filesRoot:java.io.File?=null
    @Volatile private var client: NativeClient? = null
    @Volatile private var video: RemoteVideo? = null
    private var audio: PcmOutput? = null
    private var relay: Relay? = null
    private var sink: EventChannel.EventSink? = null
    private var sessionId: String? = null
    private var frameShown = false
    private var engineAttempt=0L
    private var networkMonitor:MobileNetworkMonitor?=null
    private val gamepad by lazy{MobileGamepad(this,main)}
    private var gamepadCapture=0L
    private var gamepadHighest=0L
    private var gamepadAnalog=false
    private var analogStatus=-1
    private fun stopController(){if(gamepadAnalog && gamepadCapture>0)client?.gamepadCapture(gamepadCapture,false);gamepad.stop();gamepadCapture=0;gamepadAnalog=false;analogStatus=-1}
    private var inputAllowed=false
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
                override fun onListen(arguments:Any?,eventSink:EventChannel.EventSink){
                    sink=eventSink;networkMonitor?.close()
                    try{networkMonitor=MobileNetworkMonitor(this@MainActivity,main){available,changed,revision->
                        sink?.success(mapOf("state" to "network","networkAvailable" to available,"networkChanged" to changed,"networkRevision" to revision))
                    }}catch(_:Exception){networkMonitor=null}
                }
                override fun onCancel(arguments:Any?){stopController();client?.input(4);networkMonitor?.close();networkMonitor=null;sink=null}
            })
        }
        channels=MethodChannel(flutterEngine.dartExecutor.binaryMessenger,"com.sanser.mobile/engine").also { channel ->
            channel.setMethodCallHandler { call,result ->
                when(call.method){
                    "capabilities" -> result.success(mapOf("available" to true,"video" to "h264","audio" to true,"voice" to false,"files" to true,"transport" to "auto","direct" to true,"networkRecovery" to true,"controllerKeys" to true,"controllerAnalog" to true))
                    "connect" -> {
                        stopController();inputAllowed=false
                        documents.cancelPick()
                        @Suppress("UNCHECKED_CAST") val configuration=call.arguments as? Map<String,Any?>
                        if(configuration==null){result.error("configuration","Thiếu cấu hình phiên.",null);return@setMethodCallHandler}
                        val epoch=generation.incrementAndGet()
                        work.execute {
                            stopNative()
                            try {
                                val id=configuration["sessionId"] as String
                                main.post {
                                    if(epoch==generation.get()){
                                        sessionId=id;engineAttempt=(configuration["engineAttempt"] as? Number)?.toLong() ?: 0;frameShown=false;inputAllowed=configuration["inputEnabled"]==true && configuration["sharingId"]==null
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
                    "disconnect" -> {stopController();inputAllowed=false;documents.cancelPick();generation.incrementAndGet();startup?.let{main.removeCallbacks(it)};sessionId=null;frameShown=false;work.execute{stopNative();main.post{result.success(null)}}}
                    "controllerCapture" -> {
                        val id=call.argument<String>("sessionId");val attempt=call.argument<Number>("engineAttempt")?.toLong()
                        val capture=call.argument<Number>("captureId")?.toLong() ?: 0
                        val analog=call.argument<String>("mode")=="analog"
                        if(call.argument<Boolean>("enabled")!=true){
                            if(id==sessionId && attempt==engineAttempt && capture==gamepadCapture)stopController()
                            result.success(null)
                        }else if(id!=sessionId || attempt!=engineAttempt || capture<=gamepadHighest || !inputAllowed || !frameShown){result.error("controller","Phiên điều khiển chưa sẵn sàng.",null)}
                        else {
                            gamepadHighest=capture
                            stopController()
                            if(analog && client?.gamepadCapture(capture,true)!=true){result.error("controller_analog","Host chưa hỗ trợ tay cầm analog.",null)}
                            else {
                                gamepadCapture=capture;gamepadAnalog=analog;gamepad.start{snapshot->
                                    if(id==sessionId && attempt==engineAttempt && gamepadCapture==capture){
                                        val paused=snapshot["paused"]==true
                                        val accepted=!analog || (!paused && client?.gamepadSample(capture,snapshot)==true)
                                        val armed=analog && ((client?.gamepadStatus(capture)?:0) and 4)!=0
                                        if(paused || !accepted){client?.input(4);stopController()}
                                        val status=(if(snapshot["connected"]==true)1 else 0) or (if(armed)2 else 0)
                                        if(!analog || paused || !accepted || status!=analogStatus){
                                            analogStatus=status
                                            val data=if(analog)mapOf("connected" to (snapshot["connected"]==true))else snapshot
                                            sink?.success(data+mapOf("state" to "controller","sessionId" to id,"engineAttempt" to attempt,"captureId" to capture,"analog" to analog,"analogArmed" to armed,"paused" to (paused || !accepted)))
                                        }
                                    }
                                };result.success(null)
                            }
                        }
                    }
                    "mute" -> {client?.mute(call.argument<Boolean>("muted")==true);result.success(null)}
                    "key" -> {client?.input(2,code=call.argument<Int>("code")?:0,down=call.argument<Boolean>("down")==true);result.success(null)}
                    "resetInput" -> {client?.input(4);result.success(null)}
                    "text" -> {
                        val matches=sessionId!=null && call.argument<String>("sessionId")==sessionId && frameShown
                        val text=call.argument<String>("text") ?: ""
                        result.success(matches && text.length<=256 && client?.text(text)==true)
                    }
                    "input" -> {
                        if(sessionId!=null && call.argument<String>("sessionId")==sessionId){
                            val kind=call.argument<Int>("kind") ?: 4
                            if(kind in 0..4)client?.input(kind,call.argument<Int>("x")?:0,call.argument<Int>("y")?:0,
                                call.argument<Int>("code")?:0,call.argument<Int>("delta")?:0,call.argument<Boolean>("down")==true)
                        }
                        result.success(null)
                    }
                    "fileInbox" -> documents.inbox(result)
                    "fileExport" -> documents.export(call.argument<String>("key")?:"",result)
                    "filePick" -> {
                        val id=sessionId;val epoch=generation.get();val root=filesRoot
                        if(id==null || call.argument<String>("sessionId")!=id || !frameShown || root==null || filesHandle==0L)result.error("files","Phiên điều khiển chưa sẵn sàng.",null)
                        else documents.pick(root,{generation.get()==epoch && sessionId==id},result)
                    }
                    "files" -> {
                        val id=sessionId;val epoch=generation.get();val handle=filesHandle
                        val action=call.argument<Map<String,Any?>>("action")
                        if(id==null || call.argument<String>("sessionId")!=id || !frameShown || action==null || handle==0L)result.error("files","Phiên điều khiển đã kết thúc.",null)
                        else fileActions.execute{try{val reply=MobileFileBridge.call(handle,action);main.post{if(generation.get()==epoch && sessionId==id)result.success(reply)else result.error("files","Phiên điều khiển đã kết thúc.",null)}}catch(error:Exception){main.post{result.error("files",error.message?:"File operation failed",null)}}}
                    }
                    "clipboard" -> {
                        val id=sessionId;val epoch=generation.get()
                        if(id==null || call.argument<String>("sessionId")!=id || !frameShown){result.error("stale_session","Phiên điều khiển đã kết thúc.",null)}
                        else {
                            val accepted=client?.clipboard(call.argument<Boolean>("write")==true,call.argument<String>("text")?:""){status,text->
                                main.post{
                                    if(generation.get()!=epoch || sessionId!=id)result.error("stale_session","Phiên điều khiển đã kết thúc.",null)
                                    else if(status==0)result.success(text)
                                    else result.error("clipboard_$status","Không thể truy cập clipboard. Kiểm tra quyền clipboard trên host rồi thử lại.",null)
                                }
                            }==true
                            if(!accepted)result.error("clipboard_unavailable","Clipboard chưa được host cho phép hoặc kết nối chưa sẵn sàng.",null)
                        }
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
        if(configuration["inputEnabled"]==true && configuration["sharingId"]==null){val root=documents.workspace();filesRoot=root;filesHandle=MobileFileBridge.open(root)}
        native.start(endpoint,configuration["sessionToken"] as String,configuration["inputEnabled"]==true,configuration["sharingId"]!=null,configuration["viewerId"] as? String ?: "",(configuration["localPort"] as? Number)?.toInt() ?: 0,((configuration["videoBitrateMbps"] as? Number)?.toInt() ?: 6)*1000000,(configuration["baselineRttMs"] as? Number)?.toDouble() ?: 0.0,filesHandle!=0L)
        native.mute(configuration["audioMuted"]==true)
        main.post { if(epoch==generation.get()){emit("connected");poll(epoch);pollFiles(epoch)} }
    }
    private fun emit(state:String,message:String?=null,rtt:Double?=null){val id=sessionId?:return
        if(gamepadAnalog && ((client?.gamepadStatus(gamepadCapture)?:0) and 2)==0){
            val capture=gamepadCapture;stopController()
            sink?.success(mapOf("state" to "controller","sessionId" to id,"engineAttempt" to engineAttempt,"captureId" to capture,"paused" to true))
        }
        sink?.success(mapOf("sessionId" to id,"engineAttempt" to engineAttempt,"state" to state,"message" to message,"rttMs" to rtt,"videoWidth" to videoWidth,"videoHeight" to videoHeight,"unicodeInput" to (client?.textAvailable()==true),"clipboard" to (client?.clipboardAvailable()==true),"gamepad" to (((client?.gamepadStatus()?:0) and 1)!=0))+(client?.stats()?:emptyMap()))}
    private fun poll(epoch:Long){
        main.postDelayed({if(epoch==generation.get() && sessionId!=null){val rtt=client?.rtt();emit(if(frameShown)"frame" else "connecting",rtt=rtt?.takeIf{it>=0});poll(epoch)}},1000)
    }
    private fun fail(message:String){stopController();inputAllowed=false;documents.cancelPick();emit("stopped",message);generation.incrementAndGet();sessionId=null;startup?.let{main.removeCallbacks(it)};work.execute{stopNative()}}
    private fun pollFiles(epoch:Long){
        main.postDelayed({if(epoch==generation.get() && sessionId!=null && filesHandle!=0L){work.execute{
            if(epoch==generation.get())client?.pumpFiles(filesHandle)
            main.post{if(epoch==generation.get())pollFiles(epoch)}
        }}},80)
    }
    override fun onActivityResult(requestCode:Int,resultCode:Int,data:Intent?){if(!documents.result(requestCode,resultCode,data))super.onActivityResult(requestCode,resultCode,data)}
    private fun stopNative(){
        val files=filesHandle;filesHandle=0;filesRoot=null;MobileFileBridge.close(files)
        val native=client;client=null;native?.input(4);native?.close()
        video?.resetStream()
        relay?.close();relay=null;audio?.close();audio=null
    }
    override fun dispatchKeyEvent(event:KeyEvent):Boolean = if(gamepad.key(event))true else super.dispatchKeyEvent(event)
    override fun dispatchGenericMotionEvent(event:MotionEvent):Boolean = if(gamepad.motion(event))true else super.dispatchGenericMotionEvent(event)
    override fun onWindowFocusChanged(hasFocus:Boolean){if(!hasFocus){gamepad.pause();stopController();client?.input(4)};super.onWindowFocusChanged(hasFocus)}
    override fun onStop(){stopController();client?.input(4);super.onStop()}
    override fun onDestroy(){stopController();networkMonitor?.close();networkMonitor=null;documents.close();fileActions.shutdown();generation.incrementAndGet();sink=null;startup?.let{main.removeCallbacks(it)};channels?.setMethodCallHandler(null);events?.setStreamHandler(null);work.execute{stopNative()};work.shutdown();super.onDestroy()}
}
