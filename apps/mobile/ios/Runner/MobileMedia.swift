import UIKit
import AVFoundation
import CoreMedia
import VideoToolbox
import AVKit

private final class MobileVideoDisplayTick:NSObject {
    weak var view:MobileVideoView?
    @objc func tick(){view?.presentLatest()}
}

final class MobileVideoView: UIView {
    private let display=AVSampleBufferDisplayLayer()
    var externalTouch=false
    var dimensions:((Int,Int)->Void)?
    private var videoWidth=0,videoHeight=0
    private var zoom:CGFloat=1,panX:CGFloat=0,panY:CGFloat=0
    private let decoder=MobileVideoDecoder(automaticPresentation:false)
    private var displayLink:CADisplayLink?
    private var presented:UInt64=0,displayBusy:UInt64=0,displayFailures:UInt64=0
    private var format:CMVideoFormatDescription?
    var input: ((UInt32,UInt32,Bool?) -> Void)?
    var keyframe: (() -> Void)?
    var firstFrame: (() -> Void)?
    private var shown=false
    private var lastX:UInt32=0,lastY:UInt32=0
    override init(frame:CGRect){
        super.init(frame:frame);backgroundColor = .black;clipsToBounds=true;display.videoGravity = .resizeAspect;isMultipleTouchEnabled=false
        layer.addSublayer(display)
        decoder.imageReady={[weak self] in self?.presentLatest()}
        decoder.keyframe={[weak self] in DispatchQueue.main.async{[weak self] in self?.keyframe?()}}
        let target=MobileVideoDisplayTick();target.view=self
        // Retry a busy layer on the display clock; normal frames do not wait
        // for this tick. The decoded mailbox stays newest-only in either case.
        let clock=CADisplayLink(target:target,selector:#selector(MobileVideoDisplayTick.tick))
        // ProMotion phones refresh at up to 120 Hz while streaming, so a
        // decoded frame waits at most about 8 ms for the screen.
        if #available(iOS 15.0,*){clock.preferredFrameRateRange=CAFrameRateRange(minimum:60,maximum:120,preferred:120)}else{clock.preferredFramesPerSecond=60}
        clock.isPaused=true
        clock.add(to:.main,forMode:.common);displayLink=clock
    }
    required init?(coder:NSCoder){fatalError("init(coder:) has not been implemented")}
    override func didMoveToWindow(){super.didMoveToWindow();displayLink?.isPaused=window==nil}
    deinit{displayLink?.invalidate()}
    override func layoutSubviews(){super.layoutSubviews();layoutVideo()}
    func viewport(_ scale:Double,_ x:Double,_ y:Double){
        guard scale.isFinite,x.isFinite,y.isFinite else{return}
        zoom=CGFloat(max(1,min(3,scale)))
        let limit=(zoom-1)/2
        panX=max(-limit,min(limit,CGFloat(x)));panY=max(-limit,min(limit,CGFloat(y)));layoutVideo()
    }
    private func layoutVideo(){
        CATransaction.begin();CATransaction.setDisableActions(true)
        display.setAffineTransform(.identity);display.frame=bounds
        display.setAffineTransform(CGAffineTransform(translationX:panX*bounds.width,y:panY*bounds.height).scaledBy(x:zoom,y:zoom))
        CATransaction.commit()
    }
    func receiver()->(Data,Int,Int,Bool)->Void{
        let stream=decoder.stream()
        return {[weak decoder] data,_,_,key in decoder?.push(data,key:key,stream:stream)}
    }
    fileprivate func presentLatest(){
        if display.status == .failed{displayFailures+=1;display.flush()}
        guard display.isReadyForMoreMediaData else{displayBusy+=1;return}
        guard let image=decoder.takeLatest() else{return}
        present(image)
    }
    private func present(_ image:CVPixelBuffer){
        let width=CVPixelBufferGetWidth(image),height=CVPixelBufferGetHeight(image)
        if width != videoWidth || height != videoHeight {videoWidth=width;videoHeight=height;format=nil;dimensions?(width,height)}
        if format==nil{guard CMVideoFormatDescriptionCreateForImageBuffer(allocator:kCFAllocatorDefault,imageBuffer:image,formatDescriptionOut:&format)==noErr else{return}}
        guard let format=format else{return}
        var timing=CMSampleTimingInfo(duration:.invalid,presentationTimeStamp:CMClockGetTime(CMClockGetHostTimeClock()),decodeTimeStamp:.invalid),sample:CMSampleBuffer?
        guard CMSampleBufferCreateReadyWithImageBuffer(allocator:kCFAllocatorDefault,imageBuffer:image,formatDescription:format,sampleTiming:&timing,sampleBufferOut:&sample)==noErr,let sample=sample else{return}
        if let attachments=CMSampleBufferGetSampleAttachmentsArray(sample,createIfNecessary:true) as? [NSMutableDictionary],let attributes=attachments.first {attributes[kCMSampleAttachmentKey_DisplayImmediately]=true}
        display.enqueue(sample)
        presented+=1
        if !shown && display.status != .failed {shown=true;firstFrame?()}
    }
    private func touch(_ touches:Set<UITouch>,down:Bool?){
        guard !externalTouch else{return}
        guard let point=touches.first?.location(in:self),let format=format else{return}
        let dimensions=CMVideoFormatDescriptionGetDimensions(format),scale=min(bounds.width/CGFloat(dimensions.width),bounds.height/CGFloat(dimensions.height))
        let w=CGFloat(dimensions.width)*scale,h=CGFloat(dimensions.height)*scale,left=(bounds.width-w)/2,top=(bounds.height-h)/2
        guard point.x>=left,point.x<=left+w,point.y>=top,point.y<=top+h else{if down==false{input?(lastX,lastY,false)};return}
        lastX=UInt32(max(0,min(65535,(point.x-left)/w*65535)));lastY=UInt32(max(0,min(65535,(point.y-top)/h*65535)))
        input?(lastX,lastY,down)
    }
    override func touchesBegan(_ touches:Set<UITouch>,with event:UIEvent?){touch(touches,down:true)}
    override func touchesMoved(_ touches:Set<UITouch>,with event:UIEvent?){touch(touches,down:nil)}
    override func touchesEnded(_ touches:Set<UITouch>,with event:UIEvent?){touch(touches,down:false)}
    override func touchesCancelled(_ touches:Set<UITouch>,with event:UIEvent?){input?(lastX,lastY,false)}
    func stats()->[String:UInt64]{decoder.stats().merging(["iOSPresentedFrames":presented,"iOSDisplayBusyTicks":displayBusy,"iOSDisplayFailures":displayFailures]){_,new in new}}
    func reset(){decoder.reset();display.flushAndRemoveImage();format=nil;shown=false;presented=0;displayBusy=0;displayFailures=0}
    // Picture in picture shows this same display layer in a system window.
    private var pip:MobilePictureInPicture?
    var pictureInPictureChanged:((Bool)->Void)?
    func pictureInPicture(_ enabled:Bool){
        if !enabled{pip?.stop();pip=nil;return}
        guard pip==nil,AVPictureInPictureController.isPictureInPictureSupported() else{return}
        let controller=MobilePictureInPicture(display);controller.changed={[weak self] active in self?.pictureInPictureChanged?(active)};pip=controller
    }
}

/// Live remote screen: no timeline, no pause and no skipping.
final class MobilePictureInPicture:NSObject,AVPictureInPictureSampleBufferPlaybackDelegate,AVPictureInPictureControllerDelegate {
    private var controller:AVPictureInPictureController?
    var changed:((Bool)->Void)?
    init(_ layer:AVSampleBufferDisplayLayer){
        super.init()
        let controller=AVPictureInPictureController(contentSource:.init(sampleBufferDisplayLayer:layer,playbackDelegate:self))
        controller.delegate=self
        // Starts by itself when the user leaves Sanser while the picture shows.
        controller.canStartPictureInPictureAutomaticallyFromInline=true
        controller.requiresLinearPlayback=true
        self.controller=controller
    }
    func stop(){controller?.stopPictureInPicture();controller?.delegate=nil;controller=nil}
    func pictureInPictureController(_ controller:AVPictureInPictureController,setPlaying playing:Bool){}
    func pictureInPictureControllerTimeRangeForPlayback(_ controller:AVPictureInPictureController)->CMTimeRange{CMTimeRange(start:.negativeInfinity,duration:.positiveInfinity)}
    func pictureInPictureControllerIsPlaybackPaused(_ controller:AVPictureInPictureController)->Bool{false}
    func pictureInPictureController(_ controller:AVPictureInPictureController,didTransitionToRenderSize newRenderSize:CMVideoDimensions){}
    func pictureInPictureController(_ controller:AVPictureInPictureController,skipByInterval skipInterval:CMTime,completion completionHandler:@escaping()->Void){completionHandler()}
    func pictureInPictureControllerWillStartPictureInPicture(_ controller:AVPictureInPictureController){changed?(true)}
    func pictureInPictureControllerDidStopPictureInPicture(_ controller:AVPictureInPictureController){changed?(false)}
    func pictureInPictureController(_ controller:AVPictureInPictureController,failedToStartPictureInPictureWithError error:Error){changed?(false)}
    func pictureInPictureController(_ controller:AVPictureInPictureController,restoreUserInterfaceForPictureInPictureStopWithCompletionHandler completionHandler:@escaping(Bool)->Void){completionHandler(true)}
}
// AVAudioSourceNode continuously pulls PCM at the audio device clock. A single
// lock-free ring replaces 5ms scheduleBuffer calls and repeated engine restarts.
private final class MobilePCM {
    let pointer:OpaquePointer
    init?(){guard let p=sanser_mobile_pcm_create() else{return nil};pointer=p}
    deinit{sanser_mobile_pcm_destroy(pointer)}
}
/// One line per microphone event (no audio content), for device consoles.
func sanserVoiceLog(_ message:String){fputs("SANSER_VOICE \(message)\n",stderr);NSLog("SANSER_VOICE %@",message)}
final class MobileAudio {
    private let queue=DispatchQueue(label:"SanserAudio",qos:.userInteractive)
    private var engine:AVAudioEngine?,source:AVAudioSourceNode?,pcm:MobilePCM?
    // Group voice plays through its own ring and node; the engine mixer sums
    // it with screen sound, so voice processing cancels both from the mic.
    private var voiceSource:AVAudioSourceNode?,voicePCM:MobilePCM?
    private var microphone=false,converter:AVAudioConverter?
    // Running engine without screen sound: keeps the session alive in the
    // background. Mixes with other apps so it never silences their music.
    private var keepingAlive=false
    private var tapLogged=false,tapCallbacks:UInt64=0,tapFrames:UInt64=0
    // Mic delivered on the hardware I/O cycle (~10 ms). An input tap batches
    // ~100 ms under voice processing, which sent voice in 5-packet bursts.
    private var sink:AVAudioSinkNode?
    private var sinkPCM=[Int16](repeating:0,count:4096)
    private var lastRebuild:TimeInterval=0
    private var closed=false
    private var observers=[NSObjectProtocol]()
    /// Echo-cancelled 48 kHz mono PCM, called on the input tap thread.
    var capture:((UnsafePointer<Int16>,Int)->Void)?
    init(){
        // iOS stops the engine when it reconfigures audio (voice processing,
        // volume/route events). Without a rebuild the microphone stayed dead
        // while screen sound resumed through ensure().
        observers.append(NotificationCenter.default.addObserver(forName:.AVAudioEngineConfigurationChange,object:nil,queue:nil){[weak self] note in
            self?.queue.async{[weak self] in
                guard let self=self,!self.closed,let engine=self.engine,(note.object as AnyObject?)===engine,!engine.isRunning else{return}
                let now=ProcessInfo.processInfo.systemUptime
                guard now-self.lastRebuild>=1 else{sanserVoiceLog("engine configuration change ignored (rebuilt <1 s ago)");return}
                self.lastRebuild=now
                sanserVoiceLog("engine configuration change: rebuilding mic=\(self.microphone)")
                self.reset()
                do{try self.start(mix:self.keepingAlive && !self.microphone)}catch{sanserVoiceLog("rebuild after configuration change failed: \(error)")}
            }
        })
        for name in [AVAudioSession.routeChangeNotification,AVAudioSession.mediaServicesWereResetNotification,AVAudioSession.interruptionNotification] {
            observers.append(NotificationCenter.default.addObserver(forName:name,object:nil,queue:nil){[weak self] note in
                // Our own setCategory/voice processing posts categoryChange and
                // routeConfigurationChange. Rebuilding on those restarted the
                // engine every ~0.45 s with the mic open, so no microphone
                // buffer was ever delivered. Only a real device change rebuilds;
                // an engine iOS stopped is restarted by ensure() on the next block.
                if name==AVAudioSession.routeChangeNotification {
                    let raw=note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt ?? 0
                    let reason=AVAudioSession.RouteChangeReason(rawValue:raw)
                    guard reason == .newDeviceAvailable || reason == .oldDeviceUnavailable else{return}
                }
                self?.queue.async{[weak self] in
                    guard let self=self else{return}
                    self.reset()
                    if self.keepingAlive && !self.closed{try? self.ensure(mix:true)}
                }
            })
        }
    }
    private func reset(){
        // Touch inputNode only when the mic was open: creating it can switch
        // the audio session to recording.
        if let engine=engine{if converter != nil{engine.inputNode.removeTap(onBus:0)};engine.stop()}
        for node in [source,voiceSource,sink].compactMap({$0 as AVAudioNode?}){engine?.detach(node)}
        source=nil;voiceSource=nil;sink=nil;engine=nil;pcm=nil;voicePCM=nil;converter=nil
    }
    private static func node(_ ring:MobilePCM,_ format:AVAudioFormat)->AVAudioSourceNode{
        AVAudioSourceNode(format:format){_,_,count,list->OSStatus in
            let buffers=UnsafeMutableAudioBufferListPointer(list)
            guard buffers.count==2,let l=buffers[0].mData,let r=buffers[1].mData else{
                for buffer in buffers{if let data=buffer.mData{memset(data,0,Int(buffer.mDataByteSize))}}
                return noErr
            }
            sanser_mobile_pcm_render(ring.pointer,l.assumingMemoryBound(to:Float.self),r.assumingMemoryBound(to:Float.self),Int(count))
            return noErr
        }
    }
    private func start(mix:Bool=false) throws {
        let session=AVAudioSession.sharedInstance()
        if microphone{try session.setCategory(.playAndRecord,mode:.voiceChat,options:[.defaultToSpeaker,.allowBluetooth])}
        else{try session.setCategory(.playback,mode:.default,options:mix ? [.mixWithOthers]:[])}
        try session.setPreferredSampleRate(48000)
        try session.setPreferredIOBufferDuration(0.01)
        try session.setActive(true)
        guard let ring=MobilePCM(),let voiceRing=MobilePCM(),let format=AVAudioFormat(standardFormatWithSampleRate:48000,channels:2) else{throw MobileError.socket}
        let engine=AVAudioEngine()
        if microphone{
            // Echo cancellation, noise suppression and gain control from iOS.
            try engine.inputNode.setVoiceProcessingEnabled(true)
            if #available(iOS 17.0,*){engine.inputNode.voiceProcessingOtherAudioDuckingConfiguration=AVAudioVoiceProcessingOtherAudioDuckingConfiguration(enableAdvancedDucking:false,duckingLevel:.min)}
        }
        let node=Self.node(ring,format),voiceNode=Self.node(voiceRing,format)
        engine.attach(node);engine.connect(node,to:engine.mainMixerNode,format:format)
        engine.attach(voiceNode);engine.connect(voiceNode,to:engine.mainMixerNode,format:format)
        if microphone{
            let input=engine.inputNode,inputFormat=input.outputFormat(forBus:0)
            guard inputFormat.sampleRate>0 else{throw MobileError.socket}
            tapLogged=false;tapCallbacks=0;tapFrames=0
            let direct=inputFormat.sampleRate==48000 && inputFormat.commonFormat == .pcmFormatFloat32 && !inputFormat.isInterleaved
            sanserVoiceLog("mic start route=\(session.currentRoute.inputs.map{$0.portType.rawValue}) input=\(inputFormat) sessionRate=\(session.sampleRate) io=\(session.ioBufferDuration) path=\(direct ? "sink":"tap")")
            if direct {
                let sink=AVAudioSinkNode{[weak self] _,frames,list->OSStatus in self?.sunk(list,Int(frames));return noErr}
                engine.attach(sink);engine.connect(input,to:sink,format:inputFormat);self.sink=sink
            } else {
                guard let target=AVAudioFormat(commonFormat:.pcmFormatInt16,sampleRate:48000,channels:1,interleaved:true),
                      let converter=AVAudioConverter(from:inputFormat,to:target) else{throw MobileError.socket}
                self.converter=converter
                input.installTap(onBus:0,bufferSize:1024,format:inputFormat){[weak self] buffer,_ in self?.captured(buffer,converter,target)}
            }
        }
        self.pcm=ring;self.voicePCM=voiceRing;self.source=node;self.voiceSource=voiceNode;self.engine=engine
        try engine.start()
    }
    /// Realtime thread: 48 kHz Float32 channel 0 to Int16, no allocation.
    private func sunk(_ list:UnsafePointer<AudioBufferList>,_ frames:Int){
        tapCallbacks+=1;tapFrames+=UInt64(frames)
        if !tapLogged{tapLogged=true;sanserVoiceLog("mic first buffer frames=\(frames) rate=48000 path=sink")}
        else if tapCallbacks%3000==0{sanserVoiceLog("mic buffers=\(tapCallbacks) frames=\(tapFrames)")}
        guard let capture=capture,frames>0 else{return}
        let buffers=UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating:list))
        guard let first=buffers.first,let data=first.mData else{return}
        let samples=data.assumingMemoryBound(to:Float.self)
        var offset=0
        sinkPCM.withUnsafeMutableBufferPointer{out in
            guard let base=out.baseAddress else{return}
            while offset<frames {
                let count=min(frames-offset,out.count)
                for i in 0..<count{base[i]=Int16(max(-1,min(1,samples[offset+i]))*32767)}
                capture(base,count);offset+=count
            }
        }
    }
    private func captured(_ buffer:AVAudioPCMBuffer,_ converter:AVAudioConverter,_ target:AVAudioFormat){
        tapCallbacks+=1;tapFrames+=UInt64(buffer.frameLength)
        if !tapLogged{tapLogged=true;sanserVoiceLog("mic first buffer frames=\(buffer.frameLength) rate=\(buffer.format.sampleRate) sink=\(capture != nil)")}
        else if tapCallbacks%500==0{sanserVoiceLog("mic buffers=\(tapCallbacks) frames=\(tapFrames)")}
        guard let capture=capture,buffer.frameLength>0 else{return}
        let capacity=AVAudioFrameCount(Double(buffer.frameLength)*48000/buffer.format.sampleRate)+64
        guard let output=AVAudioPCMBuffer(pcmFormat:target,frameCapacity:capacity) else{return}
        var supplied=false
        let status=converter.convert(to:output,error:nil){_,state in
            if supplied{state.pointee = .noDataNow;return nil}
            supplied=true;state.pointee = .haveData;return buffer
        }
        guard status != .error,output.frameLength>0,let mono=output.int16ChannelData?[0] else{if tapCallbacks<5{sanserVoiceLog("mic convert failed status=\(status.rawValue) out=\(output.frameLength)")};return}
        capture(mono,Int(output.frameLength))
    }
    // A stopped engine (iOS pauses it after a configuration change) restarts
    // in place; rebuilding it would reconfigure the session and stop it again.
    private func ensure(mix:Bool=false) throws{
        if let engine=engine{
            if engine.isRunning{return}
            if (try? engine.start()) != nil{return}
            sanserVoiceLog("engine restart failed: rebuilding mic=\(microphone)")
        }
        reset();try start(mix:mix)
    }
    func push(_ samples:[Int16]){queue.async{[weak self] in
        guard let self=self,!self.closed,!samples.isEmpty,samples.count%2==0 else{return}
        do {
            try self.ensure()
            guard let pcm=self.pcm else{return}
            samples.withUnsafeBufferPointer{sanser_mobile_pcm_push(pcm.pointer,$0.baseAddress,$0.count)}
        }catch{sanserVoiceLog("audio start failed: \(error)");self.reset()}
    }}
    func pushVoice(_ samples:[Int16]){queue.async{[weak self] in
        guard let self=self,!self.closed,!samples.isEmpty,samples.count%2==0 else{return}
        do {
            try self.ensure()
            guard let pcm=self.voicePCM else{return}
            samples.withUnsafeBufferPointer{sanser_mobile_pcm_push(pcm.pointer,$0.baseAddress,$0.count)}
        }catch{self.reset()}
    }}
    /// Opening or closing the microphone rebuilds the engine: voice processing
    /// and the record category can only change while it is stopped.
    func voice(microphone:Bool){queue.async{[weak self] in
        guard let self=self,!self.closed,self.microphone != microphone else{return}
        self.microphone=microphone;self.reset()
        do{try self.start()}catch{sanserVoiceLog("mic \(microphone ? "open":"close") failed: \(error)");self.microphone=false;self.reset()}
    }}
    /// Keeps the audio session running (silence when there is no sound) so
    /// iOS does not suspend Sanser while the session continues in the background.
    func keepAlive(_ enabled:Bool){queue.async{[weak self] in
        guard let self=self,!self.closed,self.keepingAlive != enabled else{return}
        self.keepingAlive=enabled
        if enabled{do{try self.ensure(mix:true)}catch{self.reset()}}
    }}
    func close(){queue.async{[self] in
        guard !closed else{return};closed=true;microphone=false;reset()
        try? AVAudioSession.sharedInstance().setActive(false,options:.notifyOthersOnDeactivation)
    }}
    deinit{for observer in observers{NotificationCenter.default.removeObserver(observer)};if converter != nil{engine?.inputNode.removeTap(onBus:0)};engine?.stop()}
}
