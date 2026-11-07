import Flutter
import UIKit
import UniformTypeIdentifiers
import Network
import GameController

private final class ClipboardReply {
    let reply:(UInt32,String)->Void
    init(_ reply:@escaping(UInt32,String)->Void){self.reply=reply}
}

final class NativeMobileSession {
    private let lock=NSLock()
    private let videoLock=NSLock()
    private var client:OpaquePointer?
    private var files:UInt64=0
    let fileRoot:URL?
    private let initiallyMuted:Bool
    private var videoCallback:((Data,Int,Int,Bool)->Void)?
    var video:((Data,Int,Int,Bool)->Void)?{
        get{videoLock.lock();defer{videoLock.unlock()};return videoCallback}
        set{videoLock.lock();videoCallback=newValue;videoLock.unlock()}
    }
    var audio:(([Int16])->Void)?
    var stopped:((UInt32)->Void)?
    init(endpoint:String,configuration:[String:Any],fileRoot:URL?) throws {
        self.fileRoot=fileRoot
        initiallyMuted=configuration["audioMuted"] as? Bool == true
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
            sanser_mobile_create(endpoint,UInt16(clamping:configuration["localPort"] as? Int ?? 0),token,configuration["inputEnabled"] as? Bool == true ? 1:0,configuration["sharingId"] as? String != nil ? 1:0,viewer,callbacks)
        }}}
        guard client != nil else{throw MobileError.invalidConfiguration}
        if let root=fileRoot {files=root.path.withCString{sanser_mobile_files_create($0)};sanser_mobile_files_enable(client,files != 0 ? 1:0)}
        sanser_mobile_video_limit(client,UInt32(clamping:(configuration["videoBitrateMbps"] as? Int ?? 6)*1000000),(configuration["baselineRttMs"] as? NSNumber)?.doubleValue ?? 0)
    }
    func start() throws {lock.lock();defer{lock.unlock()};guard sanser_mobile_start(client) != 0 else{throw MobileError.socket};sanser_mobile_mute(client,initiallyMuted ? 1:0)}
    func input(kind:UInt32,x:UInt32=0,y:UInt32=0,code:UInt32=0,delta:Int32=0,down:Bool=false){lock.lock();defer{lock.unlock()};sanser_mobile_input(client,kind,x,y,code,delta,down ? 1:0)}
    func mute(_ value:Bool){lock.lock();defer{lock.unlock()};sanser_mobile_mute(client,value ? 1:0)}
    func textAvailable()->Bool{lock.lock();defer{lock.unlock()};return sanser_mobile_text_available(client) != 0}
    func clipboardAvailable()->Bool{lock.lock();defer{lock.unlock()};return sanser_mobile_clipboard_available(client) != 0}
    func gamepadCapture(_ capture:Int,_ enabled:Bool)->Bool{lock.lock();defer{lock.unlock()};guard capture>0 else{return false};return sanser_mobile_gamepad_capture(client,UInt64(capture),enabled ? 1:0) != 0}
    func gamepadStatus(_ capture:Int=0)->UInt32{lock.lock();defer{lock.unlock()};return sanser_mobile_gamepad_status(client,UInt64(max(0,capture)))}
    func gamepadSample(_ capture:Int,_ snapshot:[String:Any])->Bool{
        guard capture>0,let buttons=snapshot["buttons"] as? Int,(0...0x3fff).contains(buttons)else{return false}
        func n(_ key:String)->Double{(snapshot[key] as? NSNumber)?.doubleValue ?? .nan}
        var state=SanserMobileGamepad();state.connected=snapshot["connected"] as? Bool == true ? 1:0
        state.sampled=snapshot["sample"] as? Bool == true ? 1:0;state.buttons=UInt32(buttons)
        state.lx=n("lx");state.ly=n("ly");state.rx=n("rx");state.ry=n("ry");state.lt=n("lt");state.rt=n("rt")
        lock.lock();defer{lock.unlock()};return sanser_mobile_gamepad_sample(client,UInt64(capture),&state) != 0
    }
    func clipboard(write:Bool,text:String,reply:@escaping(UInt32,String)->Void)->Bool{
        let bytes=Array(text.utf8);guard bytes.count<=8192 else{return false}
        let box=Unmanaged.passRetained(ClipboardReply(reply))
        lock.lock();defer{lock.unlock()}
        let accepted=bytes.withUnsafeBufferPointer{sanser_mobile_clipboard(client,write ? 1:0,$0.baseAddress,$0.count,box.toOpaque(),{context,status,data,size in
            guard let context=context else{return}
            let box=Unmanaged<ClipboardReply>.fromOpaque(context).takeRetainedValue()
            let text=data.map{String(decoding:UnsafeBufferPointer(start:$0,count:size),as:UTF8.self)} ?? ""
            DispatchQueue.main.async{box.reply(status,text)}
        }) != 0}
        if !accepted{box.release()};return accepted
    }
    func text(_ text:String)->Bool{
        guard text.utf16.count<=256 else{return false}
        let utf8=Array(text.utf8)
        lock.lock();defer{lock.unlock()}
        return utf8.withUnsafeBufferPointer{sanser_mobile_text(client,$0.baseAddress,$0.count) != 0}
    }
    func keyframe(){lock.lock();defer{lock.unlock()};sanser_mobile_keyframe(client)}
    func rtt()->Double{lock.lock();defer{lock.unlock()};return sanser_mobile_rtt(client)}
    func stats()->[String:UInt64]{
        lock.lock();defer{lock.unlock()};var s=SanserMobileStats();sanser_mobile_stats(client,&s)
        return ["videoPackets":s.video_packets,"videoFrames":s.video_frames,"nackRequests":s.nack_requests,"lostFragments":s.lost_fragments,
            "referenceDrops":s.reference_drops,"audioDecoded":s.audio_decoded,"audioConcealed":s.audio_concealed,"audioLate":s.audio_late]
    }
    func fileRequest(_ args:[String:Any]) throws->[String:Any]{
        lock.lock();let handle=files;lock.unlock()
        let data=try JSONSerialization.data(withJSONObject:args);guard let text=String(data:data,encoding:.utf8),let response=text.withCString({sanser_mobile_files_request(handle,$0)})else{throw MobileError.invalidConfiguration}
        defer{sanser_mobile_p2p_free(response)}
        let reply=try JSONSerialization.jsonObject(with:Data(String(cString:response).utf8)) as? [String:Any]
        guard reply?["ok"] as? Bool == true,let result=reply?["result"] as? [String:Any]else{throw NSError(domain:"SanserFiles",code:1,userInfo:[NSLocalizedDescriptionKey:reply?["error"] as? String ?? "File operation failed"])}
        return result
    }
    func pumpFiles(){
        lock.lock();defer{lock.unlock()};guard let client=client,files != 0 else{return}
        let epoch=sanser_mobile_file_generation(client);sanser_mobile_files_state(files,epoch,sanser_mobile_files_available(client))
        if sanser_mobile_file_overflow(client) != 0 {sanser_mobile_files_failed(files,epoch)}
        var bytes=[UInt8](repeating:0,count:8192)
        for _ in 0..<4 {let count=bytes.withUnsafeMutableBufferPointer{sanser_mobile_file_receive(client,epoch,$0.baseAddress,$0.count)};if count==0{break};bytes.withUnsafeBufferPointer{sanser_mobile_files_receive(files,epoch,$0.baseAddress,count)}}
        for _ in 0..<4 {let count=bytes.withUnsafeMutableBufferPointer{sanser_mobile_files_next(files,$0.baseAddress,$0.count)};if count<25{break}
            let sentEpoch=bytes.prefix(4).reduce(UInt32(0)){($0<<8)|UInt32($1)}
            let sent=bytes.withUnsafeBufferPointer{sanser_mobile_file_send(client,sentEpoch,$0.baseAddress?.advanced(by:4),count-4)}
            if sent==0{sanser_mobile_files_failed(files,sentEpoch)}
        }
    }
    func close(){lock.lock();sanser_mobile_files_destroy(files);files=0;defer{lock.unlock()};sanser_mobile_input(client,4,0,0,0,0,0);sanser_mobile_destroy(client);client=nil}
    deinit{sanser_mobile_files_destroy(files);sanser_mobile_destroy(client)}
}
final class MobileEngine:NSObject,FlutterPlugin,FlutterStreamHandler,FlutterPlatformViewFactory {
    private let work=DispatchQueue(label:"SanserSession",qos:.userInitiated)
    private let fileActions=DispatchQueue(label:"SanserFileActions",qos:.utility)
    private var session:NativeMobileSession?,relay:MobileRelay?,audio:MobileAudio?
    private weak var video:MobileVideoView?
    private var sink:FlutterEventSink?,sessionID:String?,generation=0,rendered=false
    private var engineAttempt=0,networkRevision=0
    private var networkMonitor:NWPathMonitor?,networkFingerprint:String?
    private let gamepad=MobileGamepadCapture()
    private var gamepadCapture=0,inputAllowed=false
    private var gamepadHighest=0
    private var gamepadAnalog=false,analogStatus = -1
    private func stopController(){if gamepadAnalog,gamepadCapture>0{_ = session?.gamepadCapture(gamepadCapture,false)};gamepad.stop();gamepadCapture=0;gamepadAnalog=false;analogStatus = -1}
    private var timer:Timer?,timeout:Timer?
    private var fileTimer:DispatchSourceTimer?
    private let fileDocuments=MobileFileDocuments()
    deinit { networkMonitor?.cancel() }
    static func register(with registrar:FlutterPluginRegistrar){
        let plugin=MobileEngine()
        registrar.addMethodCallDelegate(plugin,channel:FlutterMethodChannel(name:"com.sanser.mobile/engine",binaryMessenger:registrar.messenger()))
        FlutterEventChannel(name:"com.sanser.mobile/events",binaryMessenger:registrar.messenger()).setStreamHandler(plugin)
        registrar.register(plugin,withId:"com.sanser.mobile/video")
    }
    func onListen(withArguments arguments:Any?,eventSink events:@escaping FlutterEventSink)->FlutterError?{
        sink=events;networkMonitor?.cancel();networkFingerprint=nil
        let monitor=NWPathMonitor();networkMonitor=monitor
        monitor.pathUpdateHandler={[weak self,weak monitor] path in
            let interfaces=path.availableInterfaces.filter{path.usesInterfaceType($0.type)}.map{"\($0.type):\($0.index)"}.sorted().joined(separator:",")
            // Endpoint/interface details stay native; only change/reachability
            // and a local sequence counter cross Flutter.
            let signature="\(path.status)|\(interfaces)|\(path.gateways)"
            let available=path.status == .satisfied
            DispatchQueue.main.async{[weak self,weak monitor] in
                guard let self=self,let monitor=monitor,self.networkMonitor === monitor else{return}
                guard self.networkFingerprint != signature else{return}
                let changed=self.networkFingerprint != nil;self.networkFingerprint=signature;self.networkRevision+=1
                self.sink?(["state":"network","networkAvailable":available,"networkChanged":changed,"networkRevision":self.networkRevision])
            }
        }
        monitor.start(queue:DispatchQueue(label:"SanserNetworkPath",qos:.utility));return nil
    }
    func onCancel(withArguments arguments:Any?)->FlutterError?{stopController();session?.input(kind:4);networkMonitor?.cancel();networkMonitor=nil;networkFingerprint=nil;sink=nil;return nil}
    func create(withFrame frame:CGRect,viewIdentifier viewId:Int64,arguments args:Any?)->FlutterPlatformView{
        video?.reset()
        let native=MobileVideoView(frame:frame);video=native;native.externalTouch=true
        native.dimensions={[weak self] width,height in
            guard let self=self,let id=self.sessionID else{return}
            self.sink?(["sessionId":id,"engineAttempt":self.engineAttempt,"state":"dimensions","videoWidth":width,"videoHeight":height])
        }
        native.input={[weak self] x,y,down in self?.session?.input(kind:0,x:x,y:y);if let down=down{self?.session?.input(kind:1,x:x,y:y,down:down)}}
        native.keyframe={[weak self] in self?.session?.keyframe()}
        native.firstFrame={[weak self] in guard let self=self else{return};self.rendered=true;self.timeout?.invalidate();self.emit("frame")}
        // Flutter may create/replace the platform view after media has started.
        // Attach it atomically and ask for a fresh reference chain.
        session?.video=native.receiver();session?.keyframe()
        return VideoPlatform(native)
    }
    func createArgsCodec()->FlutterMessageCodec & NSObjectProtocol {FlutterStandardMessageCodec.sharedInstance()}
    func handle(_ call:FlutterMethodCall,result:@escaping FlutterResult){
        let args=call.arguments as? [String:Any] ?? [:]
        switch call.method {
        case "capabilities":result(["available":true,"audio":true,"voice":false,"files":true,"transport":"auto","direct":true,"video":"h264","networkRecovery":true,"controllerKeys":true,"controllerAnalog":true])
        case "connect":
            disconnect();generation+=1;let epoch=generation
            guard let id=args["sessionId"] as? String else{return result(FlutterError(code:"configuration",message:"Thiếu thông tin phiên.",details:nil))}
            sessionID=id;engineAttempt=args["engineAttempt"] as? Int ?? 0;rendered=false;inputAllowed=args["inputEnabled"] as? Bool == true && (args["sharingId"] as? String)==nil
            do {
                if args["transport"] as? String == "direct" {
                    guard let endpoint=args["endpoint"] as? String,endpoint.hasPrefix("127.0.0.1:"),
                          let port=args["localPort"] as? Int,(1...65535).contains(port) else{throw MobileError.invalidConfiguration}
                    startNative(endpoint,args,epoch)
                }else{
                    let bridge=try MobileRelay(args);relay=bridge
                    bridge.ready={[weak self] endpoint in DispatchQueue.main.async{[weak self] in self?.startNative(endpoint,args,epoch)}}
                    bridge.failed={[weak self] message in DispatchQueue.main.async{[weak self] in if self?.generation==epoch{self?.fail(message)}}}
                    bridge.start()
                }
                timeout=Timer.scheduledTimer(withTimeInterval:25,repeats:false){[weak self] _ in if self?.generation==epoch && self?.rendered==false{self?.fail("Không nhận được hình trong 25 giây.")}}
                timer=Timer.scheduledTimer(withTimeInterval:1,repeats:true){[weak self] _ in guard let self=self else{return};self.emit(self.rendered ? "frame":"connecting",rtt:self.session?.rtt())}
                result(nil)
            }catch{disconnect();result(FlutterError(code:"connection",message:"Không thể bắt đầu phiên mobile.",details:nil))}
        case "p2p":
            let operation={
                do{let reply=try Self.p2p(args);DispatchQueue.main.async{result(reply)}}
                catch{DispatchQueue.main.async{result(FlutterError(code:"direct",message:error.localizedDescription,details:nil))}}
            }
            // Cancellation must never wait behind the connectivity check it cancels.
            if args["operation"] as? String == "close"{operation()}else{work.async(execute:operation)}
        case "controllerCapture":
            let id=args["sessionId"] as? String,attempt=args["engineAttempt"] as? Int,capture=args["captureId"] as? Int ?? 0
            let analog=args["mode"] as? String == "analog"
            if args["enabled"] as? Bool != true {
                if id==sessionID,attempt==engineAttempt,capture==gamepadCapture {stopController()}
                result(nil)
            }else{
                guard id==sessionID,sessionID != nil,attempt==engineAttempt,capture>gamepadHighest,inputAllowed,rendered else{return result(FlutterError(code:"controller",message:"Phiên điều khiển chưa sẵn sàng.",details:nil))}
                gamepadHighest=capture
                stopController()
                if analog,session?.gamepadCapture(capture,true) != true{return result(FlutterError(code:"controller_analog",message:"Host chưa hỗ trợ tay cầm analog.",details:nil))}
                gamepadCapture=capture;gamepadAnalog=analog
                gamepad.start{[weak self] snapshot in
                    guard let self=self,self.sessionID==id,self.engineAttempt==attempt,self.gamepadCapture==capture else{return}
                    let paused=snapshot["paused"] as? Bool == true
                    let accepted = !analog || (!paused && self.session?.gamepadSample(capture,snapshot)==true)
                    let armed=analog && ((self.session?.gamepadStatus(capture) ?? 0)&4) != 0
                    if paused || !accepted{self.session?.input(kind:4);self.stopController()}
                    let connected=snapshot["connected"] as? Bool == true,status=(connected ? 1:0)|(armed ? 2:0)
                    if !analog || paused || !accepted || status != self.analogStatus{
                        self.analogStatus=status
                        let data:[String:Any]=analog ? ["connected":connected]:snapshot
                        self.sink?(data.merging(["state":"controller","sessionId":id!,"engineAttempt":attempt!,"captureId":capture,"analog":analog,"analogArmed":armed,"paused":paused || !accepted]){_,new in new})
                    }
                };result(nil)
            }
        case "disconnect":disconnect{result(nil)}
        case "mute":session?.mute(args["muted"] as? Bool == true);result(nil)
        case "resetInput":session?.input(kind:4);result(nil)
        case "text":
            guard args["sessionId"] as? String == sessionID,sessionID != nil,rendered,
                  let text=args["text"] as? String else{return result(false)}
            result(session?.text(text) == true)
        case "fileInbox":fileDocuments.inbox(result)
        case "fileExport":fileDocuments.export(args["key"] as? String ?? "",result:result)
        case "filePick":
            guard let id=sessionID,args["sessionId"] as? String==id,let root=session?.fileRoot,rendered else{return result(FlutterError(code:"files",message:"Phiên điều khiển chưa sẵn sàng.",details:nil))}
            let epoch=generation
            fileDocuments.pick(root:root,current:{[weak self] in self?.generation==epoch && self?.sessionID==id},result:result)
        case "files":
            guard let id=sessionID,args["sessionId"] as? String==id,let native=session,rendered,let action=args["action"] as? [String:Any] else{return result(FlutterError(code:"files",message:"Phiên điều khiển đã kết thúc.",details:nil))}
            let epoch=generation
            fileActions.async{do{let reply=try native.fileRequest(action);DispatchQueue.main.async{[weak self] in
                guard self?.generation==epoch && self?.sessionID==id else{return result(FlutterError(code:"files",message:"Phiên điều khiển đã kết thúc.",details:nil))};result(reply)
            }}catch{DispatchQueue.main.async{result(FlutterError(code:"files",message:error.localizedDescription,details:nil))}}}
        case "clipboard":
            guard let id=sessionID,args["sessionId"] as? String == id,rendered else{return result(FlutterError(code:"stale_session",message:"Phiên điều khiển đã kết thúc.",details:nil))}
            let epoch=generation
            let accepted=session?.clipboard(write:args["write"] as? Bool == true,text:args["text"] as? String ?? "",reply:{[weak self] status,text in
                guard let self=self,self.generation==epoch,self.sessionID==id else{return result(FlutterError(code:"stale_session",message:"Phiên điều khiển đã kết thúc.",details:nil))}
                if status==0{result(text)}else{result(FlutterError(code:"clipboard_\(status)",message:"Không thể truy cập clipboard. Kiểm tra quyền clipboard trên host rồi thử lại.",details:nil))}
            }) == true
            if !accepted{result(FlutterError(code:"clipboard_unavailable",message:"Clipboard chưa được host cho phép hoặc kết nối chưa sẵn sàng.",details:nil))}
        case "input":
            guard args["sessionId"] as? String == sessionID,sessionID != nil,
                  let kind=args["kind"] as? Int,(0...4).contains(kind) else{return result(nil)}
            session?.input(kind:UInt32(kind),x:UInt32(clamping:args["x"] as? Int ?? 0),
                y:UInt32(clamping:args["y"] as? Int ?? 0),code:UInt32(clamping:args["code"] as? Int ?? 0),
                delta:Int32(clamping:args["delta"] as? Int ?? 0),down:args["down"] as? Bool == true);result(nil)
        case "viewport":
            guard args["sessionId"] as? String == sessionID,sessionID != nil else{return result(nil)}
            video?.viewport((args["scale"] as? NSNumber)?.doubleValue ?? 1,
                (args["x"] as? NSNumber)?.doubleValue ?? 0,(args["y"] as? NSNumber)?.doubleValue ?? 0);result(nil)
        case "key":session?.input(kind:2,code:UInt32(clamping:args["code"] as? Int ?? 0),down:args["down"] as? Bool == true);result(nil)
        case "voice":result(FlutterError(code:"voice_unavailable",message:"Voice mobile chưa sẵn sàng trong bản preview.",details:nil))
        default:result(FlutterMethodNotImplemented)
        }
    }
    static func p2p(_ args:[String:Any]) throws->[String:Any]{
        let data=try JSONSerialization.data(withJSONObject:args)
        guard let request=String(data:data,encoding:.utf8),let response=request.withCString({sanser_mobile_p2p_request($0)}) else{throw MobileError.invalidConfiguration}
        defer{sanser_mobile_p2p_free(response)}
        let reply=try JSONSerialization.jsonObject(with:Data(String(cString:response).utf8)) as? [String:Any]
        guard reply?["ok"] as? Bool == true,let result=reply?["result"] as? [String:Any] else{
            throw NSError(domain:"SanserDirect",code:1,userInfo:[NSLocalizedDescriptionKey:reply?["error"] as? String ?? "Direct failed"])
        }
        return result
    }
    private func startNative(_ endpoint:String,_ args:[String:Any],_ epoch:Int){
        guard generation==epoch else{return}
        do {
            let root=(args["inputEnabled"] as? Bool == true && args["sharingId"] == nil) ? fileDocuments.workspace():nil
            let native=try NativeMobileSession(endpoint:endpoint,configuration:args,fileRoot:root),output=MobileAudio()
            self.session=native;self.audio=output
            // Capture this session's view; never route old callbacks to a new
            // session/view or serialize compressed video through the UI queue.
            native.video=video?.receiver()
            native.audio={samples in output.push(samples)}
            native.stopped={[weak self] reason in DispatchQueue.main.async{[weak self] in if self?.generation==epoch{self?.fail("Engine native đã dừng (\(reason)).")}}}
            let pulse=DispatchSource.makeTimerSource(queue:work);fileTimer=pulse;pulse.schedule(deadline:.now(),repeating:.milliseconds(80));pulse.setEventHandler{[weak native] in native?.pumpFiles()};pulse.resume()
            self.work.async{[weak self] in do{try native.start()}catch{DispatchQueue.main.async{if self?.generation==epoch{self?.fail("Không thể bắt đầu engine native.")}}}}
        }catch{self.fail("Không thể mở engine native.")}
    }
    private func emit(_ state:String,message:String?=nil,rtt:Double?=nil){guard let id=sessionID else{return}
        if gamepadAnalog,((session?.gamepadStatus(gamepadCapture) ?? 0)&2)==0{
            let capture=gamepadCapture;stopController()
            sink?(["state":"controller","sessionId":id,"engineAttempt":engineAttempt,"captureId":capture,"paused":true])
        }
        var data:[String:Any]=["sessionId":id,"engineAttempt":engineAttempt,"state":state];if let message=message{data["message"]=message};if let rtt=rtt,rtt>=0{data["rttMs"]=rtt};data["unicodeInput"]=session?.textAvailable() == true;data["clipboard"]=session?.clipboardAvailable() == true;data["gamepad"]=((session?.gamepadStatus() ?? 0)&1) != 0;if let stats=session?.stats(){for(key,value) in stats{data[key]=value}};sink?(data)}
    private func fail(_ message:String){emit("stopped",message:message);disconnect()}
    private func disconnect(_ completion:(()->Void)?=nil){
        stopController();inputAllowed=false
        generation+=1;fileTimer?.cancel();fileTimer=nil;fileDocuments.cancelPick();timer?.invalidate();timer=nil;timeout?.invalidate();timeout=nil;sessionID=nil
        let native=session,bridge=relay,output=audio;session=nil;relay=nil;audio=nil;video?.reset()
        work.async{native?.close();bridge?.close();output?.close();if let completion=completion{DispatchQueue.main.async(execute:completion)}}
    }
}

/// One extended controller, foreground capture only. Button/trigger transitions
/// are immediate; analog-only snapshots share one latest-state dispatch slot.
private final class MobileGamepadCapture {
    private var controller:GCController?,observers:[NSObjectProtocol]=[]
    private var emit:(([String:Any])->Void)?,latest:[String:Any]?
    private var epoch=0,pending=false,lastButtons=0
    private var lastLT:Float=0,lastRT:Float=0
    deinit { stop() }
    func start(_ callback:@escaping([String:Any])->Void){
        stop();emit=callback
        for name in [Notification.Name.GCControllerDidConnect,Notification.Name.GCControllerDidDisconnect] {
            observers.append(NotificationCenter.default.addObserver(forName:name,object:nil,queue:.main){[weak self] note in
                guard let self=self else{return}
                if name == .GCControllerDidDisconnect,let removed=note.object as? GCController,self.controller === removed {
                    self.controller?.extendedGamepad?.valueChangedHandler=nil;self.controller=nil;self.epoch+=1;self.pending=false;self.latest=nil
                    self.disconnected();self.choose(excluding:removed)
                }else{self.choose()}
            })
        }
        observers.append(NotificationCenter.default.addObserver(forName:UIApplication.willResignActiveNotification,object:nil,queue:.main){[weak self] _ in
            self?.emit?(["paused":true]);self?.stop()
        })
        choose()
    }
    func stop(){
        epoch+=1;pending=false;controller?.extendedGamepad?.valueChangedHandler=nil;controller=nil
        for observer in observers{NotificationCenter.default.removeObserver(observer)};observers=[]
        emit=nil;latest=nil;lastButtons=0;lastLT=0;lastRT=0
    }
    private func disconnected(){
        emit?(["connected":false,"sample":true,"buttons":0,"lx":0.0,"ly":0.0,"rx":0.0,"ry":0.0,"lt":0.0,"rt":0.0])
    }
    private func choose(excluding:GCController?=nil){
        guard emit != nil else{return}
        if let old=controller,GCController.controllers().contains(where:{$0 === old}){return}
        controller?.extendedGamepad?.valueChangedHandler=nil;controller=nil;epoch+=1;pending=false;latest=nil
        disconnected()
        guard let next=GCController.controllers().first(where:{$0 !== excluding && $0.extendedGamepad != nil}),let pad=next.extendedGamepad else{return}
        controller=next;next.handlerQueue = .main
        pad.valueChangedHandler={[weak self,weak next] pad,_ in guard let self=self,let next=next,self.controller === next else{return};self.changed(pad)}
        changed(pad,force:true)
    }
    private func changed(_ pad:GCExtendedGamepad,force:Bool=false){
        let buttons:[GCControllerButtonInput?]=[pad.buttonA,pad.buttonB,pad.buttonX,pad.buttonY,pad.leftShoulder,pad.rightShoulder,
            pad.buttonMenu,pad.buttonOptions,pad.leftThumbstickButton,pad.rightThumbstickButton,pad.dpad.up,pad.dpad.down,pad.dpad.left,pad.dpad.right]
        var mask=0;for(index,button)in buttons.enumerated(){if button?.isPressed==true{mask|=1<<index}}
        let lt=pad.leftTrigger.value,rt=pad.rightTrigger.value
        latest=["connected":true,"sample":true,"buttons":mask,"lx":Double(pad.leftThumbstick.xAxis.value),"ly":Double(pad.leftThumbstick.yAxis.value),
            "rx":Double(pad.rightThumbstick.xAxis.value),"ry":Double(pad.rightThumbstick.yAxis.value),"lt":Double(lt),"rt":Double(rt)]
        let immediate=force || mask != lastButtons || (lt>=0.55) != (lastLT>=0.55) || (rt>=0.55) != (lastRT>=0.55) || (lt<=0.35) != (lastLT<=0.35) || (rt<=0.35) != (lastRT<=0.35)
        lastButtons=mask;lastLT=lt;lastRT=rt
        if immediate{epoch+=1;pending=false;flush();return}
        if pending{return};pending=true;let token=epoch
        DispatchQueue.main.asyncAfter(deadline:.now()+0.016){[weak self] in guard let self=self,self.epoch==token else{return};self.pending=false;self.flush()}
    }
    private func flush(){if let snapshot=latest{latest=nil;emit?(snapshot)}}
}
private final class VideoPlatform:NSObject,FlutterPlatformView {
    let native:MobileVideoView
    init(_ native:MobileVideoView){self.native=native}
    func view()->UIView{native}
}

// Platform-owned document access. Only picker-selected bytes enter a private
// workspace; Flutter never supplies filesystem paths or receives file contents.
private final class MobileFileDocuments:NSObject,UIDocumentPickerDelegate {
    private let io=DispatchQueue(label:"SanserDocuments",qos:.utility)
    private let base=FileManager.default.urls(for:.applicationSupportDirectory,in:.userDomainMask)[0].appendingPathComponent("SanserFiles",isDirectory:true)
    private var pending:(UUID,String,FlutterResult,()->Bool,URL?)?
    private weak var picker:UIDocumentPickerViewController?
    func workspace()->URL{base.appendingPathComponent(UUID().uuidString.lowercased(),isDirectory:true)}
    private func controller()->UIViewController?{
        var current=UIApplication.shared.connectedScenes.compactMap{$0 as? UIWindowScene}.flatMap{$0.windows}.first(where:{$0.isKeyWindow})?.rootViewController
        while let next=current?.presentedViewController{current=next};return current
    }
    func pick(root:URL,current:@escaping()->Bool,result:@escaping FlutterResult){
        guard pending==nil,current(),let host=controller()else{return result(FlutterError(code:"files_busy",message:"Hộp thoại file chưa sẵn sàng.",details:nil))}
        let view=UIDocumentPickerViewController(forOpeningContentTypes:[.data,.content],asCopy:false)
        pending=(UUID(),"pick",result,current,root);view.delegate=self;picker=view;host.present(view,animated:true)
    }
    func cancelPick(){if let op=pending,op.1=="pick"{pending=nil;picker?.dismiss(animated:true);op.2(FlutterError(code:"files_stale",message:"Phiên điều khiển đã kết thúc.",details:nil))}}
    private func owned(_ key:String)->URL?{
        let parts=key.split(separator:"/",omittingEmptySubsequences:false).map(String.init)
        guard parts.count==4,UUID(uuidString:parts[0]) != nil,parts[1]=="received",parts[2]=="Sanser",parts[3].utf8.count>37,UUID(uuidString:String(decoding:parts[3].utf8.prefix(36),as:UTF8.self)) != nil,!parts[3].contains("\\")else{return nil}
        var url=base
        for part in parts {guard part != ".",part != ".." else{return nil};url.appendPathComponent(part);guard let attrs=try? url.resourceValues(forKeys:[.isSymbolicLinkKey]),attrs.isSymbolicLink != true else{return nil}}
        guard (try? url.resourceValues(forKeys:[.isRegularFileKey]).isRegularFile)==true else{return nil};return url
    }
    func inbox(_ result:@escaping FlutterResult){
        io.async{var entries=[[String:Any]]();var scanned=0
            if let urls=FileManager.default.enumerator(at:self.base,includingPropertiesForKeys:[.isRegularFileKey,.isSymbolicLinkKey,.fileSizeKey],options:[.skipsHiddenFiles]) {
                for case let url as URL in urls {scanned+=1;if scanned>10000 || entries.count>=256{break}
                    let key=String(url.path.dropFirst(self.base.path.count+1));guard let file=self.owned(key),let attrs=try? file.resourceValues(forKeys:[.fileSizeKey]) else{continue}
                    entries.append(["key":key,"name":String(decoding:file.lastPathComponent.utf8.dropFirst(37),as:UTF8.self),"size":attrs.fileSize ?? 0])
                }
            }
            DispatchQueue.main.async{result(entries)}
        }
    }
    func export(_ key:String,result:@escaping FlutterResult){
        guard pending==nil,let file=owned(key),let host=controller()else{return result(FlutterError(code:"files_export",message:"Không thể mở file đã nhận.",details:nil))}
        let view=UIDocumentPickerViewController(forExporting:[file],asCopy:true);pending=(UUID(),"export",result,{true},nil);view.delegate=self;picker=view;host.present(view,animated:true)
    }
    func documentPickerWasCancelled(_ controller:UIDocumentPickerViewController){guard controller===picker,let op=pending else{return};pending=nil;op.2(nil)}
    func documentPicker(_ controller:UIDocumentPickerViewController,didPickDocumentsAt urls:[URL]){
        guard controller===picker,let op=pending else{return}
        if op.1=="export"{pending=nil;op.2(true);return}
        guard let source=urls.first,let root=op.4,op.3()else{pending=nil;op.2(FlutterError(code:"files_pick",message:"Không thể đọc file đã chọn.",details:nil));return}
        io.async{
            let id=UUID().uuidString.lowercased();let target=root.appendingPathComponent("source-"+id);let temporary=target.appendingPathExtension("tmp")
            let accessed=source.startAccessingSecurityScopedResource();defer{if accessed{source.stopAccessingSecurityScopedResource()}}
            do{
                try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true)
                var coordinatorError:NSError?;var copyError:Error?
                NSFileCoordinator(filePresenter:nil).coordinate(readingItemAt:source,options:.withoutChanges,error:&coordinatorError){url in
                    do{
                        let attrs=try url.resourceValues(forKeys:[.isRegularFileKey,.fileSizeKey]);guard attrs.isRegularFile==true,(attrs.fileSize ?? 0)<=25*1024*1024 else{throw MobileError.invalidConfiguration}
                        guard FileManager.default.createFile(atPath:temporary.path,contents:nil,attributes:[.posixPermissions:0o600])else{throw MobileError.invalidConfiguration}
                        let input=try FileHandle(forReadingFrom:url);defer{try? input.close()};let output=try FileHandle(forWritingTo:temporary);defer{try? output.close()};var count=0
                        while let chunk=try input.read(upToCount:65536),!chunk.isEmpty {count+=chunk.count;guard count<=25*1024*1024 else{throw MobileError.invalidConfiguration};try output.write(contentsOf:chunk)}
                        try output.synchronize();try FileManager.default.moveItem(at:temporary,to:target)
                    }catch{copyError=error}
                }
                if let error=copyError ?? coordinatorError{throw error}
                let size=(try target.resourceValues(forKeys:[.fileSizeKey])).fileSize ?? 0
                DispatchQueue.main.async{
                    guard self.pending?.0==op.0,op.3() else{self.io.async{try? FileManager.default.removeItem(at:target)};return}
                    self.pending=nil;op.2(["sourceId":id,"name":source.lastPathComponent,"size":size])
                }
            }catch{
                try? FileManager.default.removeItem(at:temporary);try? FileManager.default.removeItem(at:target)
                DispatchQueue.main.async{guard self.pending?.0==op.0 else{return};self.pending=nil;op.2(FlutterError(code:"files_pick",message:"Không thể đọc file. Hãy chọn file thường tối đa 25 MB.",details:nil))}
            }
        }
    }
}
