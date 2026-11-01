import UIKit
import AVFoundation
import CoreMedia
import VideoToolbox

final class MobileVideoView: UIView {
    override class var layerClass: AnyClass { AVSampleBufferDisplayLayer.self }
    private var display: AVSampleBufferDisplayLayer {layer as! AVSampleBufferDisplayLayer}
    private let decoder=MobileVideoDecoder()
    private var format:CMVideoFormatDescription?
    var input: ((UInt32,UInt32,Bool?) -> Void)?
    var keyframe: (() -> Void)?
    var firstFrame: (() -> Void)?
    private var shown=false
    private var lastX:UInt32=0,lastY:UInt32=0
    override init(frame:CGRect){
        super.init(frame:frame);backgroundColor = .black;display.videoGravity = .resizeAspect;isMultipleTouchEnabled=false
        decoder.output={[weak self] image in self?.present(image)}
        decoder.keyframe={[weak self] in DispatchQueue.main.async{[weak self] in self?.keyframe?()}}
    }
    required init?(coder:NSCoder){fatalError("init(coder:) has not been implemented")}
    func receiver()->(Data,Int,Int,Bool)->Void{
        let stream=decoder.stream()
        return {[weak decoder] data,_,_,key in decoder?.push(data,key:key,stream:stream)}
    }
    private func present(_ image:CVPixelBuffer){
        if display.status == .failed{display.flush()}
        // Dropping a decoded image cannot invalidate the H.264 reference chain.
        guard display.isReadyForMoreMediaData else{return}
        guard CMVideoFormatDescriptionCreateForImageBuffer(allocator:kCFAllocatorDefault,imageBuffer:image,formatDescriptionOut:&format)==noErr,let format=format else{return}
        var timing=CMSampleTimingInfo(duration:.invalid,presentationTimeStamp:.zero,decodeTimeStamp:.invalid),sample:CMSampleBuffer?
        guard CMSampleBufferCreateReadyWithImageBuffer(allocator:kCFAllocatorDefault,imageBuffer:image,formatDescription:format,sampleTiming:&timing,sampleBufferOut:&sample)==noErr,let sample=sample else{return}
        if let attachments=CMSampleBufferGetSampleAttachmentsArray(sample,createIfNecessary:true) as? [NSMutableDictionary],let attributes=attachments.first {attributes[kCMSampleAttachmentKey_DisplayImmediately]=true}
        display.enqueue(sample)
        if !shown && display.status != .failed {shown=true;firstFrame?()}
    }
    private func touch(_ touches:Set<UITouch>,down:Bool?){
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
    func reset(){decoder.reset();display.flushAndRemoveImage();format=nil;shown=false}
}
// AVAudioSourceNode continuously pulls PCM at the audio device clock. A single
// lock-free ring replaces 5ms scheduleBuffer calls and repeated engine restarts.
private final class MobilePCM {
    let pointer:OpaquePointer
    init?(){guard let p=sanser_mobile_pcm_create() else{return nil};pointer=p}
    deinit{sanser_mobile_pcm_destroy(pointer)}
}
final class MobileAudio {
    private let queue=DispatchQueue(label:"SanserAudio")
    private var engine:AVAudioEngine?,source:AVAudioSourceNode?,pcm:MobilePCM?
    private var closed=false
    private var observers=[NSObjectProtocol]()
    init(){
        for name in [AVAudioSession.routeChangeNotification,AVAudioSession.mediaServicesWereResetNotification,AVAudioSession.interruptionNotification] {
            observers.append(NotificationCenter.default.addObserver(forName:name,object:nil,queue:nil){[weak self] _ in
                self?.queue.async{[weak self] in self?.reset()}
            })
        }
    }
    private func reset(){
        engine?.stop()
        if let source=source{engine?.detach(source)}
        source=nil;engine=nil;pcm=nil
    }
    private func start() throws {
        let session=AVAudioSession.sharedInstance()
        try session.setCategory(.playback,mode:.default)
        try session.setPreferredSampleRate(48000)
        try session.setPreferredIOBufferDuration(0.01)
        try session.setActive(true)
        guard let ring=MobilePCM(),let format=AVAudioFormat(standardFormatWithSampleRate:48000,channels:2) else{throw MobileError.socket}
        let node=AVAudioSourceNode(format:format){_,_,count,list->OSStatus in
            let buffers=UnsafeMutableAudioBufferListPointer(list)
            guard buffers.count==2,let l=buffers[0].mData,let r=buffers[1].mData else{
                for buffer in buffers{if let data=buffer.mData{memset(data,0,Int(buffer.mDataByteSize))}}
                return noErr
            }
            sanser_mobile_pcm_render(ring.pointer,l.assumingMemoryBound(to:Float.self),r.assumingMemoryBound(to:Float.self),Int(count))
            return noErr
        }
        let engine=AVAudioEngine()
        engine.attach(node);engine.connect(node,to:engine.mainMixerNode,format:format)
        self.pcm=ring;self.source=node;self.engine=engine
        try engine.start()
    }
    func push(_ samples:[Int16]){queue.async{[weak self] in
        guard let self=self,!self.closed,!samples.isEmpty,samples.count%2==0 else{return}
        do {
            if self.engine==nil || self.engine?.isRunning==false{self.reset();try self.start()}
            guard let pcm=self.pcm else{return}
            samples.withUnsafeBufferPointer{sanser_mobile_pcm_push(pcm.pointer,$0.baseAddress,$0.count)}
        }catch{self.reset()}
    }}
    func close(){queue.async{[self] in
        guard !closed else{return};closed=true;reset()
        try? AVAudioSession.sharedInstance().setActive(false,options:.notifyOthersOnDeactivation)
    }}
    deinit{for observer in observers{NotificationCenter.default.removeObserver(observer)};engine?.stop()}
}
