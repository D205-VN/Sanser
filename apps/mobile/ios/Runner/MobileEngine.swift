import Flutter
import UIKit

final class NativeMobileSession {
    private let lock=NSLock()
    private var client:OpaquePointer?
    var video:((Data,Int,Int,Bool)->Void)?
    var audio:(([Int16])->Void)?
    var stopped:((UInt32)->Void)?
    init(endpoint:String,configuration:[String:Any]) throws {
        var callbacks=SanserMobileCallbacks()
        callbacks.context=Unmanaged.passUnretained(self).toOpaque()
        callbacks.video={context,bytes,size,width,height,key in
            guard let context=context,let bytes=bytes else{return}
            let owner=Unmanaged<NativeMobileSession>.fromOpaque(context).takeUnretainedValue()
            owner.video?(Data(bytes:bytes,count:size),Int(width),Int(height),key != 0)
        }
        callbacks.audio={context,bytes,size in
            guard let context=context,let bytes=bytes else{return}
            Unmanaged<NativeMobileSession>.fromOpaque(context).takeUnretainedValue().audio?(Array(UnsafeBufferPointer(start:bytes,count:size)))
        }
        callbacks.stopped={context,reason in
            guard let context=context else{return}
            Unmanaged<NativeMobileSession>.fromOpaque(context).takeUnretainedValue().stopped?(reason)
        }
        guard let token=configuration["sessionToken"] as? String else {throw MobileError.invalidConfiguration}
        let viewer=configuration["viewerId"] as? String ?? ""
        client=endpoint.withCString{endpoint in token.withCString{token in viewer.withCString{viewer in
            sanser_mobile_create(endpoint,0,token,configuration["inputEnabled"] as? Bool == true ? 1:0,configuration["sharingId"] as? String != nil ? 1:0,viewer,callbacks)
        }}}
        guard client != nil else{throw MobileError.invalidConfiguration}
    }
    func start() throws {lock.lock();defer{lock.unlock()};guard sanser_mobile_start(client) != 0 else{throw MobileError.socket}}
    func input(kind:UInt32,x:UInt32=0,y:UInt32=0,code:UInt32=0,down:Bool=false){lock.lock();defer{lock.unlock()};sanser_mobile_input(client,kind,x,y,code,0,down ? 1:0)}
    func mute(_ value:Bool){lock.lock();defer{lock.unlock()};sanser_mobile_mute(client,value ? 1:0)}
    func keyframe(){lock.lock();defer{lock.unlock()};sanser_mobile_keyframe(client)}
    func rtt()->Double{lock.lock();defer{lock.unlock()};return sanser_mobile_rtt(client)}
    func close(){lock.lock();defer{lock.unlock()};sanser_mobile_input(client,4,0,0,0,0,0);sanser_mobile_destroy(client);client=nil}
    deinit{sanser_mobile_destroy(client)}
}
final class MobileEngine:NSObject,FlutterPlugin,FlutterStreamHandler,FlutterPlatformViewFactory {
    private let work=DispatchQueue(label:"SanserSession")
    private var session:NativeMobileSession?,relay:MobileRelay?,audio:MobileAudio?
    private weak var video:MobileVideoView?
    private var sink:FlutterEventSink?,sessionID:String?,generation=0,rendered=false
    private var timer:Timer?,timeout:Timer?
    private let frames=NSLock();private var pendingFrames=0;private var frameEpoch=0;private var awaitKeyframe=true
    static func register(with registrar:FlutterPluginRegistrar){
        let plugin=MobileEngine()
        registrar.addMethodCallDelegate(plugin,channel:FlutterMethodChannel(name:"com.sanser.mobile/engine",binaryMessenger:registrar.messenger()))
        FlutterEventChannel(name:"com.sanser.mobile/events",binaryMessenger:registrar.messenger()).setStreamHandler(plugin)
        registrar.register(plugin,withId:"com.sanser.mobile/video")
    }
    func onListen(withArguments arguments:Any?,eventSink events:@escaping FlutterEventSink)->FlutterError?{sink=events;return nil}
    func onCancel(withArguments arguments:Any?)->FlutterError?{sink=nil;return nil}
    func create(withFrame frame:CGRect,viewIdentifier viewId:Int64,arguments args:Any?)->FlutterPlatformView{
        let native=MobileVideoView(frame:frame);video=native
        native.input={[weak self] x,y,down in self?.session?.input(kind:0,x:x,y:y);if let down=down{self?.session?.input(kind:1,x:x,y:y,down:down)}}
        native.keyframe={[weak self] in self?.session?.keyframe()}
        native.firstFrame={[weak self] in guard let self=self else{return};self.rendered=true;self.timeout?.invalidate();self.emit("frame")}
        return VideoPlatform(native)
    }
    func createArgsCodec()->FlutterMessageCodec & NSObjectProtocol {FlutterStandardMessageCodec.sharedInstance()}
    func handle(_ call:FlutterMethodCall,result:@escaping FlutterResult){
        let args=call.arguments as? [String:Any] ?? [:]
        switch call.method {
        case "capabilities":result(["available":true,"audio":true,"voice":false,"transport":"wss","video":"h264"])
        case "connect":
            disconnect();generation+=1;let epoch=generation
            guard let id=args["sessionId"] as? String else{return result(FlutterError(code:"configuration",message:"Thiếu thông tin phiên.",details:nil))}
            sessionID=id;rendered=false
            do {
                let bridge=try MobileRelay(args);relay=bridge
                bridge.ready={[weak self] endpoint in DispatchQueue.main.async{[weak self] in
                    guard let self=self,self.generation==epoch else{return}
                    do {
                        let native=try NativeMobileSession(endpoint:endpoint,configuration:args),output=MobileAudio()
                        self.session=native;self.audio=output
                        native.video={[weak self] data,w,h,key in
                            guard let self=self else{return}
                            self.frames.lock()
                            if self.pendingFrames>=3 {self.frameEpoch+=1;self.awaitKeyframe=true;self.frames.unlock();DispatchQueue.main.async{[weak self] in self?.session?.keyframe()};return}
                            let frameEpoch=self.frameEpoch
                            self.pendingFrames+=1;self.frames.unlock()
                            DispatchQueue.main.async{[weak self] in
                                guard let self=self else{return};defer{self.frames.lock();self.pendingFrames-=1;self.frames.unlock()}
                                self.frames.lock();let valid=self.frameEpoch==frameEpoch && (!self.awaitKeyframe || key);if valid && key{self.awaitKeyframe=false};self.frames.unlock()
                                if self.generation==epoch && valid{self.video?.frame(data,width:w,height:h,key:key)}
                            }
                        }
                        native.audio={samples in output.push(samples)}
                        native.stopped={[weak self] reason in DispatchQueue.main.async{[weak self] in if self?.generation==epoch{self?.fail("Engine native đã dừng (\(reason)).")}}}
                        self.work.async{[weak self] in do{try native.start()}catch{DispatchQueue.main.async{if self?.generation==epoch{self?.fail("Không thể bắt đầu engine native.")}}}}
                    }catch{self.fail("Không thể mở engine native.")}
                }}
                bridge.failed={[weak self] message in DispatchQueue.main.async{[weak self] in if self?.generation==epoch{self?.fail(message)}}}
                bridge.start()
                timeout=Timer.scheduledTimer(withTimeInterval:25,repeats:false){[weak self] _ in if self?.generation==epoch && self?.rendered==false{self?.fail("Không nhận được hình trong 25 giây.")}}
                timer=Timer.scheduledTimer(withTimeInterval:1,repeats:true){[weak self] _ in guard let self=self else{return};self.emit(self.rendered ? "frame":"connecting",rtt:self.session?.rtt())}
                result(nil)
            }catch{disconnect();result(FlutterError(code:"connection",message:"Không thể bắt đầu phiên mobile.",details:nil))}
        case "disconnect":disconnect();result(nil)
        case "mute":session?.mute(args["muted"] as? Bool == true);result(nil)
        case "resetInput":session?.input(kind:4);result(nil)
        case "key":session?.input(kind:2,code:UInt32(clamping:args["code"] as? Int ?? 0),down:args["down"] as? Bool == true);result(nil)
        case "voice":result(FlutterError(code:"voice_unavailable",message:"Voice mobile chưa sẵn sàng trong bản preview.",details:nil))
        default:result(FlutterMethodNotImplemented)
        }
    }
    private func emit(_ state:String,message:String?=nil,rtt:Double?=nil){guard let id=sessionID else{return};var data:[String:Any]=["sessionId":id,"state":state];if let message=message{data["message"]=message};if let rtt=rtt,rtt>=0{data["rttMs"]=rtt};sink?(data)}
    private func fail(_ message:String){emit("stopped",message:message);disconnect()}
    private func disconnect(){
        generation+=1;timer?.invalidate();timer=nil;timeout?.invalidate();timeout=nil;sessionID=nil
        frames.lock();frameEpoch+=1;awaitKeyframe=true;frames.unlock()
        let native=session,bridge=relay,output=audio;session=nil;relay=nil;audio=nil;video?.reset()
        work.async{native?.close();bridge?.close();output?.close()}
    }
}
private final class VideoPlatform:NSObject,FlutterPlatformView {
    let native:MobileVideoView
    init(_ native:MobileVideoView){self.native=native}
    func view()->UIView{native}
}
