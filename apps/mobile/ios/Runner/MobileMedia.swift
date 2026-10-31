import UIKit
import AVFoundation
import CoreMedia

final class MobileVideoView: UIView {
    override class var layerClass: AnyClass { AVSampleBufferDisplayLayer.self }
    private var display: AVSampleBufferDisplayLayer {layer as! AVSampleBufferDisplayLayer}
    private var sps: Data?,pps: Data?,format: CMVideoFormatDescription?
    var input: ((UInt32,UInt32,Bool?) -> Void)?
    var keyframe: (() -> Void)?
    var firstFrame: (() -> Void)?
    private var shown=false
    override init(frame:CGRect){super.init(frame:frame);backgroundColor = .black;display.videoGravity = .resizeAspect;isMultipleTouchEnabled=false}
    required init?(coder:NSCoder){fatalError("init(coder:) has not been implemented")}
    private func units(_ data:Data)->[Data]{
        let bytes=Array(data);var result=[Data](),start:Int?,i=0
        while i+3<=bytes.count {
            let four=i+4<=bytes.count && bytes[i...i+3].elementsEqual([0,0,0,1])
            let three=bytes[i...i+2].elementsEqual([0,0,1])
            if four || three {
                if let begin=start,i>begin{result.append(Data(bytes[begin..<i]))}
                i+=four ? 4 : 3;start=i
            }else{i+=1}
        }
        if let begin=start,begin<bytes.count{result.append(Data(bytes[begin...]))}
        return result
    }
    func frame(_ data:Data,width:Int,height:Int,key:Bool){
        let nals=units(data);guard !nals.isEmpty else{return}
        for nal in nals {
            switch (nal.first ?? 0)&31 {case 7:if sps != nal{sps=nal;format=nil};case 8:if pps != nal{pps=nal;format=nil};default:break}
        }
        if display.status == .failed {display.flush();format=nil;keyframe?()}
        if format==nil,let sps=sps,let pps=pps {
            sps.withUnsafeBytes{s in pps.withUnsafeBytes{p in
                let pointers=[s.bindMemory(to:UInt8.self).baseAddress!,p.bindMemory(to:UInt8.self).baseAddress!],sizes=[sps.count,pps.count]
                _=CMVideoFormatDescriptionCreateFromH264ParameterSets(allocator:kCFAllocatorDefault,parameterSetCount:2,parameterSetPointers:pointers,parameterSetSizes:sizes,nalUnitHeaderLength:4,formatDescriptionOut:&format)
            }}
        }
        guard let format=format,display.isReadyForMoreMediaData else {keyframe?();return}
        var packet=Data()
        for nal in nals {var size=UInt32(nal.count).bigEndian;withUnsafeBytes(of:&size){packet.append(contentsOf:$0)};packet.append(nal)}
        var block:CMBlockBuffer?
        guard CMBlockBufferCreateWithMemoryBlock(allocator:kCFAllocatorDefault,memoryBlock:nil,blockLength:packet.count,blockAllocator:kCFAllocatorDefault,customBlockSource:nil,offsetToData:0,dataLength:packet.count,flags:0,blockBufferOut:&block)==kCMBlockBufferNoErr,let block=block else{return}
        _=packet.withUnsafeBytes{CMBlockBufferReplaceDataBytes(with:$0.baseAddress!,blockBuffer:block,offsetIntoDestination:0,dataLength:packet.count)}
        var sample:CMSampleBuffer?,size=packet.count
        var timing=CMSampleTimingInfo(duration:.invalid,presentationTimeStamp:.invalid,decodeTimeStamp:.invalid)
        guard CMSampleBufferCreateReady(allocator:kCFAllocatorDefault,dataBuffer:block,formatDescription:format,sampleCount:1,sampleTimingEntryCount:1,sampleTimingArray:&timing,sampleSizeEntryCount:1,sampleSizeArray:&size,sampleBufferOut:&sample)==noErr,let sample=sample else{return}
        if let attachments=CMSampleBufferGetSampleAttachmentsArray(sample,createIfNecessary:true) as? [NSMutableDictionary],let attributes=attachments.first {attributes[kCMSampleAttachmentKey_DisplayImmediately]=true}
        display.enqueue(sample)
        if !shown && display.status != .failed {shown=true;firstFrame?()}
    }
    private func touch(_ touches:Set<UITouch>,down:Bool?){
        guard let point=touches.first?.location(in:self),let format=format else{return}
        let dimensions=CMVideoFormatDescriptionGetDimensions(format),scale=min(bounds.width/CGFloat(dimensions.width),bounds.height/CGFloat(dimensions.height))
        let w=CGFloat(dimensions.width)*scale,h=CGFloat(dimensions.height)*scale,left=(bounds.width-w)/2,top=(bounds.height-h)/2
        guard point.x>=left,point.x<=left+w,point.y>=top,point.y<=top+h else{return}
        input?(UInt32(max(0,min(65535,(point.x-left)/w*65535))),UInt32(max(0,min(65535,(point.y-top)/h*65535))),down)
    }
    override func touchesBegan(_ touches:Set<UITouch>,with event:UIEvent?){touch(touches,down:true)}
    override func touchesMoved(_ touches:Set<UITouch>,with event:UIEvent?){touch(touches,down:nil)}
    override func touchesEnded(_ touches:Set<UITouch>,with event:UIEvent?){touch(touches,down:false)}
    override func touchesCancelled(_ touches:Set<UITouch>,with event:UIEvent?){input?(0,0,false)}
    func reset(){display.flushAndRemoveImage();format=nil;sps=nil;pps=nil;shown=false}
}
final class MobileAudio {
    private let queue=DispatchQueue(label:"SanserAudio")
    private var engine:AVAudioEngine?,player:AVAudioPlayerNode?
    private var pending=0,closed=false
    private var route: NSObjectProtocol?
    init(){route=NotificationCenter.default.addObserver(forName:AVAudioSession.routeChangeNotification,object:nil,queue:nil){[weak self] _ in self?.queue.async{[weak self] in self?.reset()}}}
    private func reset(){player?.stop();engine?.stop();player=nil;engine=nil;pending=0}
    func push(_ samples:[Int16]){queue.async{[weak self] in
        guard let self=self,!self.closed else{return}
        do {
            if self.engine==nil {
                let session=AVAudioSession.sharedInstance();try session.setCategory(.playback,mode:.default);try session.setActive(true)
                let engine=AVAudioEngine(),node=AVAudioPlayerNode()
                let format=AVAudioFormat(standardFormatWithSampleRate:48000,channels:2)!
                engine.attach(node);engine.connect(node,to:engine.mainMixerNode,format:format);try engine.start();node.play()
                self.engine=engine;self.player=node
            }
            guard self.pending<12,let node=self.player,let format=AVAudioFormat(standardFormatWithSampleRate:48000,channels:2),let buffer=AVAudioPCMBuffer(pcmFormat:format,frameCapacity:AVAudioFrameCount(samples.count/2)) else {self.reset();return}
            buffer.frameLength=buffer.frameCapacity
            for i in 0..<Int(buffer.frameLength){buffer.floatChannelData?[0][i]=Float(samples[i*2])/32768;buffer.floatChannelData?[1][i]=Float(samples[i*2+1])/32768}
            self.pending+=1
            node.scheduleBuffer(buffer,completionCallbackType:.dataPlayedBack){[weak self] _ in self?.queue.async{[weak self] in self?.pending=max(0,(self?.pending ?? 0)-1)}}
        }catch{self.reset()}
    }}
    func close(){queue.async{[weak self] in guard let self=self else{return};self.closed=true;self.reset();try? AVAudioSession.sharedInstance().setActive(false)}}
    deinit{if let route=route{NotificationCenter.default.removeObserver(route)};engine?.stop()}
}
