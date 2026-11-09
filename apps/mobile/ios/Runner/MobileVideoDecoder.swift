import Foundation
import CoreMedia
import VideoToolbox

// Keep compressed reference frames in decode order. Only decoded images may be
// replaced freely; a busy Flutter/main thread must not force another IDR.
final class MobileVideoDecoder {
    private let queue=DispatchQueue(label:"SanserVideoDecode",qos:.userInteractive)
    private let lock=NSLock()
    private var generation=0,streamGeneration=0
    private struct EncodedImage {
        let data:Data,key:Bool,epoch:Int,arrived:TimeInterval
    }
    private var pending=[EncodedImage](),pendingBytes=0,workerScheduled=false
    private var resetPending=false,recoveryPending=false,requireKeyframe=false
    // The transport may release up to 24 repaired reference pictures together.
    // Bound compressed history separately from the single decoded-image slot.
    private let maxPendingFrames=24,maxPendingBytes=8*1024*1024
    private let decodeDeadline:TimeInterval=0.16
    private var sps:Data?,pps:Data?,format:CMVideoFormatDescription?,session:VTDecompressionSession?
    private var waiting=true,lastRequest:TimeInterval=0
    private var latest:(CVPixelBuffer,Int,TimeInterval)?,presentationPending=false
    private let automaticPresentation:Bool
    private var decoded:UInt64=0,replaced:UInt64=0,expired:UInt64=0,overflows:UInt64=0
    private var decodeSamples:UInt64=0,decodeWorkUs:UInt64=0,decodeQueueUs:UInt64=0
    private var presented:UInt64=0,presentWaitUs:UInt64=0
    var output:((CVPixelBuffer)->Void)?
    // Coalesced main-thread readiness; the presenter consumes only when ready.
    var imageReady:(()->Void)?
    var keyframe:(()->Void)?

    init(automaticPresentation:Bool=true){self.automaticPresentation=automaticPresentation}

    func reset(){
        lock.lock();generation+=1;streamGeneration+=1;latest=nil
        pending.removeAll();pendingBytes=0;resetPending=true;recoveryPending=false;requireKeyframe=false
        decoded=0;replaced=0;expired=0;overflows=0;decodeSamples=0;decodeWorkUs=0;decodeQueueUs=0;presented=0;presentWaitUs=0
        scheduleWorker();lock.unlock()
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
        guard !requireKeyframe || key else{lock.unlock();return}
        if data.count>maxPendingBytes || pending.count>=maxPendingFrames || data.count>maxPendingBytes-pendingBytes {
            overflows+=1;generation+=1;latest=nil;pending.removeAll();pendingBytes=0
            resetPending=true;recoveryPending=true;requireKeyframe=true
            // A fitting independent picture can restart immediately. Otherwise
            // reject dependent pictures until a fresh IDR, once per episode.
            guard key && data.count<=maxPendingBytes else{scheduleWorker();lock.unlock();return}
        }
        if key && requireKeyframe{requireKeyframe=false;recoveryPending=false}
        pending.append(EncodedImage(data:data,key:key,epoch:generation,arrived:ProcessInfo.processInfo.systemUptime));pendingBytes+=data.count
        scheduleWorker();lock.unlock()
    }
    // Called with lock held. One worker drains a bounded queue; never dispatch
    // a reset/request job for each rejected P-picture in the same burst.
    private func scheduleWorker(){
        guard !workerScheduled else{return};workerScheduled=true
        queue.async{[self] in drain()}
    }
    private func recover(epoch:Int,queueExpired:Bool=false){
        lock.lock();defer{lock.unlock()};guard epoch==generation else{return}
        if queueExpired{expired+=1}
        generation+=1;pending.removeAll();pendingBytes=0;latest=nil
        resetPending=true;recoveryPending=true;requireKeyframe=true;scheduleWorker()
    }
    private func drain(){
        while true {
            lock.lock()
            if resetPending || recoveryPending {
                let reset=resetPending,repair=recoveryPending;resetPending=false;recoveryPending=false;lock.unlock()
                if reset{invalidate();sps=nil;pps=nil;format=nil;waiting=true}
                if repair{request()}
                continue
            }
            guard !pending.isEmpty else{workerScheduled=false;lock.unlock();return}
            let item=pending.removeFirst();pendingBytes-=item.data.count
            let valid=item.epoch==generation;lock.unlock();guard valid else{continue}
            let began=ProcessInfo.processInfo.systemUptime
            if began-item.arrived>decodeDeadline{recover(epoch:item.epoch,queueExpired:true);continue}
            if waiting && !item.key{continue}
            decode(item.data,key:item.key,epoch:item.epoch)
            let finished=ProcessInfo.processInfo.systemUptime
            lock.lock()
            if item.epoch==generation{decodeSamples+=1;decodeWorkUs+=UInt64(max(0,(finished-began)*1000000));decodeQueueUs+=UInt64(max(0,(began-item.arrived)*1000000))}
            lock.unlock()
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
                if status==noErr,let image=image{owner.publish(image,epoch:epoch)}else{owner.recover(epoch:epoch)}
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
        if status != noErr{recover(epoch:epoch)}else if key{waiting=false}
    }
    private func publish(_ image:CVPixelBuffer,epoch:Int){
        lock.lock();guard epoch==generation else{lock.unlock();return}
        decoded+=1;if latest != nil{replaced+=1}
        latest=(image,epoch,ProcessInfo.processInfo.systemUptime)
        // Wake the presenter immediately, with at most one main-thread job.
        // Display-link polling alone imposed an extra half-refresh of delay.
        guard automaticPresentation || imageReady != nil else{lock.unlock();return}
        if presentationPending{lock.unlock();return}
        presentationPending=true;lock.unlock()
        DispatchQueue.main.async{[weak self] in
            guard let self=self else{return}
            self.lock.lock();self.presentationPending=false;self.lock.unlock()
            if self.automaticPresentation {
                if let image=self.takeLatest(){self.output?(image)}
            }else{self.imageReady?()}
        }
    }
    // Caller consumes only when the display layer can accept an image. While
    // busy, newer decoded images replace the slot without losing references.
    func takeLatest()->CVPixelBuffer?{
        lock.lock();defer{lock.unlock()}
        guard let (image,epoch,at)=latest else{return nil};latest=nil
        guard epoch==generation else{return nil}
        presented+=1;presentWaitUs+=UInt64(max(0,(ProcessInfo.processInfo.systemUptime-at)*1000000))
        return image
    }
    func stats()->[String:UInt64]{
        lock.lock();defer{lock.unlock()}
        return ["iOSDecodedFrames":decoded,"iOSDecodedReplaced":replaced,"iOSDecodeExpired":expired,"iOSDecodeOverflows":overflows,
            "iOSDecodeAvgUs":decodeSamples>0 ? decodeWorkUs/decodeSamples:0,"iOSDecodeQueueAvgUs":decodeSamples>0 ? decodeQueueUs/decodeSamples:0,
            "iOSPresentWaitAvgUs":presented>0 ? presentWaitUs/presented:0]
    }
    deinit{invalidate()}
}

// Bounded local numeric evidence, readable during a device test. Never retain
// endpoints, identifiers, input, compressed pictures or audio samples.
final class MobileVideoHealthLog {
    private let queue=DispatchQueue(label:"SanserVideoHealth",qos:.utility)
    private var samples=[[String:Double]]()
    private let url:URL?
    private static let fields:Set<String>=["sampleTimeMs","rttMs","videoFrames","videoPackets","nackRequests","lostFragments","referenceDrops",
        "iOSDecodedFrames","iOSDecodedReplaced","iOSDecodeExpired","iOSDecodeOverflows","iOSDecodeAvgUs","iOSDecodeQueueAvgUs","iOSPresentWaitAvgUs",
        "iOSPresentedFrames","iOSDisplayBusyTicks","iOSDisplayFailures"]
    init(url:URL?=nil){self.url=url ?? FileManager.default.urls(for:.cachesDirectory,in:.userDomainMask).first?.appendingPathComponent("sanser-video-health.json")}
    func reset(){queue.async{[self] in samples.removeAll();write()}}
    func record(_ values:[String:Any]){
        var sample=[String:Double]()
        for key in Self.fields{if let value=values[key] as? NSNumber,value.doubleValue.isFinite{sample[key]=value.doubleValue}}
        guard !sample.isEmpty else{return}
        queue.async{[self] in samples.append(sample);if samples.count>120{samples.removeFirst(samples.count-120)};write()}
    }
    private func write(){
        let build=Int(Bundle.main.object(forInfoDictionaryKey:"CFBundleVersion") as? String ?? "") ?? 0
        guard let url=url,let data=try? JSONSerialization.data(withJSONObject:["schemaVersion":1,"appBuild":build,"samples":samples]) else{return}
        try? data.write(to:url,options:.atomic)
    }
    func flush(){queue.sync{}}
}
