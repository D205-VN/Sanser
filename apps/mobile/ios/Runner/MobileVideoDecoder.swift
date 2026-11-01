import Foundation
import CoreMedia
import VideoToolbox

// Keep compressed reference frames in decode order. Only decoded images may be
// replaced freely; a busy Flutter/main thread must not force another IDR.
final class MobileVideoDecoder {
    private let queue=DispatchQueue(label:"SanserVideoDecode",qos:.userInteractive)
    private let lock=NSLock()
    private var generation=0,streamGeneration=0,pending=0
    private var sps:Data?,pps:Data?,format:CMVideoFormatDescription?,session:VTDecompressionSession?
    private var waiting=true,lastRequest:TimeInterval=0
    private var latest:(CVPixelBuffer,Int)?,presentationPending=false
    var output:((CVPixelBuffer)->Void)?
    var keyframe:(()->Void)?

    func reset(){
        lock.lock();generation+=1;streamGeneration+=1;latest=nil;lock.unlock()
        queue.async{[self] in invalidate();sps=nil;pps=nil;format=nil;waiting=true}
    }
    private func invalidate(){
        if let session=session{VTDecompressionSessionWaitForAsynchronousFrames(session);VTDecompressionSessionInvalidate(session)}
        session=nil
    }
    private func request(){
        waiting=true
        let now=ProcessInfo.processInfo.systemUptime
        guard now-lastRequest>=0.25 else{return};lastRequest=now;keyframe?()
    }
    func stream()->Int{lock.lock();defer{lock.unlock()};return streamGeneration}
    func push(_ data:Data,key:Bool,stream:Int){
        lock.lock()
        guard stream==streamGeneration else{lock.unlock();return}
        // A bounded decode backlog is separate from the one-image render slot.
        if pending>=6{generation+=1;latest=nil;lock.unlock();queue.async{[self] in request()};return}
        let epoch=generation;pending+=1;lock.unlock()
        let arrived=ProcessInfo.processInfo.systemUptime
        queue.async{[self] in
            defer{lock.lock();pending-=1;lock.unlock()}
            lock.lock();let valid=epoch==generation;lock.unlock();guard valid else{return}
            if ProcessInfo.processInfo.systemUptime-arrived>0.08{request();return}
            if waiting && !key{return}
            decode(data,key:key,epoch:epoch)
        }
    }
    private func units(_ data:Data)->[Data]{
        data.withUnsafeBytes{raw in
            let b=raw.bindMemory(to:UInt8.self);var result=[Data](),start:Int?,i=0
            while i+2<b.count{
                let three=b[i]==0 && b[i+1]==0 && b[i+2]==1
                let four=i+3<b.count && b[i]==0 && b[i+1]==0 && b[i+2]==0 && b[i+3]==1
                if three || four{if let s=start,i>s{result.append(data.subdata(in:s..<i))};i+=four ? 4:3;start=i}else{i+=1}
            }
            if let s=start,s<b.count{result.append(data.subdata(in:s..<b.count))};return result
        }
    }
    private func decode(_ data:Data,key:Bool,epoch:Int){
        let nals=units(data);guard !nals.isEmpty else{return}
        for nal in nals{
            switch (nal.first ?? 0)&31{
            case 7:if sps != nal{invalidate();sps=nal;format=nil}
            case 8:if pps != nal{invalidate();pps=nal;format=nil}
            default:break
            }
        }
        if format==nil,let sps=sps,let pps=pps{
            sps.withUnsafeBytes{s in pps.withUnsafeBytes{p in
                let pointers=[s.bindMemory(to:UInt8.self).baseAddress!,p.bindMemory(to:UInt8.self).baseAddress!],sizes=[sps.count,pps.count]
                _=CMVideoFormatDescriptionCreateFromH264ParameterSets(allocator:kCFAllocatorDefault,parameterSetCount:2,parameterSetPointers:pointers,parameterSetSizes:sizes,nalUnitHeaderLength:4,formatDescriptionOut:&format)
            }}
        }
        guard let format=format else{request();return}
        if session==nil{
            var callback=VTDecompressionOutputCallbackRecord(decompressionOutputCallback:{context,frame,status,_,image,_,_ in
                guard let context=context,let frame=frame else{return}
                let owner=Unmanaged<MobileVideoDecoder>.fromOpaque(context).takeUnretainedValue()
                let epoch=Int(bitPattern:frame)-1
                if status==noErr,let image=image{owner.publish(image,epoch:epoch)}else{owner.queue.async{[weak owner] in owner?.request()}}
            },decompressionOutputRefCon:Unmanaged.passUnretained(self).toOpaque())
            let attributes:[String:Any]=[kCVPixelBufferPixelFormatTypeKey as String:kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,kCVPixelBufferIOSurfacePropertiesKey as String:[:]]
            guard VTDecompressionSessionCreate(allocator:kCFAllocatorDefault,formatDescription:format,decoderSpecification:nil,imageBufferAttributes:attributes as CFDictionary,outputCallback:&callback,decompressionSessionOut:&session)==noErr,let session=session else{request();return}
            VTSessionSetProperty(session,key:kVTDecompressionPropertyKey_RealTime,value:kCFBooleanTrue)
        }
        var packet=Data();packet.reserveCapacity(data.count)
        for nal in nals{var length=UInt32(nal.count).bigEndian;withUnsafeBytes(of:&length){packet.append(contentsOf:$0)};packet.append(nal)}
        var block:CMBlockBuffer?
        guard CMBlockBufferCreateWithMemoryBlock(allocator:kCFAllocatorDefault,memoryBlock:nil,blockLength:packet.count,blockAllocator:kCFAllocatorDefault,customBlockSource:nil,offsetToData:0,dataLength:packet.count,flags:0,blockBufferOut:&block)==noErr,let block=block else{request();return}
        _=packet.withUnsafeBytes{CMBlockBufferReplaceDataBytes(with:$0.baseAddress!,blockBuffer:block,offsetIntoDestination:0,dataLength:packet.count)}
        var sample:CMSampleBuffer?,size=packet.count
        guard CMSampleBufferCreateReady(allocator:kCFAllocatorDefault,dataBuffer:block,formatDescription:format,sampleCount:1,sampleTimingEntryCount:0,sampleTimingArray:nil,sampleSizeEntryCount:1,sampleSizeArray:&size,sampleBufferOut:&sample)==noErr,let sample=sample,let session=session else{request();return}
        // Synchronous submission on our decode worker bounds VideoToolbox work
        // as well as compressed input; temporal postprocessing adds no value here.
        let status=VTDecompressionSessionDecodeFrame(session,sampleBuffer:sample,flags:[],frameRefcon:UnsafeMutableRawPointer(bitPattern:epoch+1),infoFlagsOut:nil)
        if status != noErr{invalidate();request()}else if key{waiting=false}
    }
    private func publish(_ image:CVPixelBuffer,epoch:Int){
        lock.lock();guard epoch==generation else{lock.unlock();return}
        latest=(image,epoch)
        if presentationPending{lock.unlock();return}
        presentationPending=true;lock.unlock()
        DispatchQueue.main.async{[weak self] in
            guard let self=self else{return}
            self.lock.lock();let latest=self.latest;self.latest=nil;self.presentationPending=false;let generation=self.generation;self.lock.unlock()
            if let (image,epoch)=latest,epoch==generation{self.output?(image)}
        }
    }
    deinit{invalidate()}
}
