import Foundation
import CoreMedia
import VideoToolbox

// Runs the production iPhone decoder on macOS with real H.264, including a
// deliberately stalled UI run loop. No network or Flutter mock is involved.
private final class EncodedFrames {
    var frames=[Data]()
    func append(_ sample:CMSampleBuffer){
        guard let format=CMSampleBufferGetFormatDescription(sample),let block=CMSampleBufferGetDataBuffer(sample) else{return}
        var packet=Data()
        for index in 0..<2 {
            var pointer:UnsafePointer<UInt8>?,size=0,count=0
            guard CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format,parameterSetIndex:index,parameterSetPointerOut:&pointer,parameterSetSizeOut:&size,parameterSetCountOut:&count,nalUnitHeaderLengthOut:nil)==noErr,let pointer=pointer else{continue}
            packet.append(contentsOf:[0,0,0,1]);packet.append(pointer,count:size)
        }
        let size=CMBlockBufferGetDataLength(block);var bytes=[UInt8](repeating:0,count:size)
        CMBlockBufferCopyDataBytes(block,atOffset:0,dataLength:size,destination:&bytes)
        var offset=0
        while offset+4<=size {
            let n=Int(bytes[offset])<<24|Int(bytes[offset+1])<<16|Int(bytes[offset+2])<<8|Int(bytes[offset+3]);offset+=4
            precondition(n>0 && offset+n<=size)
            packet.append(contentsOf:[0,0,0,1]);packet.append(contentsOf:bytes[offset..<offset+n]);offset+=n
        }
        frames.append(packet)
    }
}

@main struct VideoDecoderTest {
    static func pump(_ seconds:Double){RunLoop.main.run(until:Date(timeIntervalSinceNow:seconds))}
    static func main(){
        let encoded=EncodedFrames();var encoder:VTCompressionSession?
        precondition(VTCompressionSessionCreate(allocator:kCFAllocatorDefault,width:64,height:64,codecType:kCMVideoCodecType_H264,encoderSpecification:nil,imageBufferAttributes:nil,compressedDataAllocator:nil,outputCallback:{context,_,status,_,sample in
            guard status==noErr,let context=context,let sample=sample else{return}
            Unmanaged<EncodedFrames>.fromOpaque(context).takeUnretainedValue().append(sample)
        },refcon:Unmanaged.passUnretained(encoded).toOpaque(),compressionSessionOut:&encoder)==noErr)
        let session=encoder!
        VTSessionSetProperty(session,key:kVTCompressionPropertyKey_RealTime,value:kCFBooleanTrue)
        VTSessionSetProperty(session,key:kVTCompressionPropertyKey_AllowFrameReordering,value:kCFBooleanFalse)
        VTSessionSetProperty(session,key:kVTCompressionPropertyKey_ProfileLevel,value:kVTProfileLevel_H264_Baseline_AutoLevel)
        VTCompressionSessionPrepareToEncodeFrames(session)
        for frame in 0..<24 {
            var image:CVPixelBuffer?
            precondition(CVPixelBufferCreate(kCFAllocatorDefault,64,64,kCVPixelFormatType_32BGRA,nil,&image)==kCVReturnSuccess)
            CVPixelBufferLockBaseAddress(image!,[])
            memset(CVPixelBufferGetBaseAddress(image!)!,Int32(32+min(frame,17)*12),CVPixelBufferGetBytesPerRow(image!)*64)
            CVPixelBufferUnlockBaseAddress(image!,[])
            precondition(VTCompressionSessionEncodeFrame(session,imageBuffer:image!,presentationTimeStamp:CMTime(value:Int64(frame),timescale:60),duration:CMTime(value:1,timescale:60),frameProperties:nil,sourceFrameRefcon:nil,infoFlagsOut:nil)==noErr)
        }
        VTCompressionSessionCompleteFrames(session,untilPresentationTimeStamp:.invalid);VTCompressionSessionInvalidate(session)
        precondition(encoded.frames.count==24,"Encoder did not produce the reference chain")

        let decoder=MobileVideoDecoder();var presented=0
        decoder.output={image in
            precondition(Thread.isMainThread && CVPixelBufferGetWidth(image)==64)
            presented+=1
        }
        let stream=decoder.stream(),done=DispatchSemaphore(value:0)
        DispatchQueue.global(qos:.userInitiated).async {
            for (index,frame) in encoded.frames.enumerated(){decoder.push(frame,key:index==0,stream:stream);Thread.sleep(forTimeInterval:0.01)}
            done.signal()
        }
        precondition(done.wait(timeout:.now()+5) == .success)
        // The UI has not run while all frames decoded. Only one image is pending.
        Thread.sleep(forTimeInterval:0.1);pump(0.15)
        precondition(presented==1,"Decoded frames accumulated on the UI queue: \(presented)")
        decoder.reset();pump(0.05)
        decoder.push(encoded.frames[0],key:true,stream:stream);pump(0.05)
        precondition(presented==1,"An old session frame reached the new stream")
        decoder.push(encoded.frames[0],key:true,stream:decoder.stream());pump(0.15)
        precondition(presented==2,"Decoder did not restart after reset")
        decoder.reset();pump(0.05)
        // Production iOS presentation polls on its display link. A busy layer
        // must leave one NEWEST image available without asking for another IDR.
        let manual=MobileVideoDecoder(automaticPresentation:false);var callbacks=0,requests=0,notifications=0
        manual.output={_ in callbacks+=1};manual.keyframe={requests+=1}
        manual.imageReady={notifications+=1}
        let manualStream=manual.stream()
        manual.push(encoded.frames[0],key:true,stream:manualStream)
        let warm=Date(timeIntervalSinceNow:5)
        while (manual.stats()["iOSDecodedFrames"] ?? 0)<1 && Date()<warm{Thread.sleep(forTimeInterval:0.001)}
        // A repaired reference can release the entire transport history in one
        // callback burst. Six queued dispatch jobs used to abort this chain.
        for frame in encoded.frames.dropFirst(){manual.push(frame,key:false,stream:manualStream)}
        let until=Date(timeIntervalSinceNow:5)
        while (manual.stats()["iOSDecodedFrames"] ?? 0)<24 && Date()<until{Thread.sleep(forTimeInterval:0.001)}
        precondition(manual.stats()["iOSDecodedFrames"]==24 && manual.stats()["iOSDecodedReplaced"]==23,"Repaired frame burst damaged reference decoding or accumulated images")
        precondition(manual.stats()["iOSDecodeOverflows"]==0,"The transport's bounded repair burst overflowed the decode worker")
        precondition(notifications==0,"Readiness notifications must execute on main")
        pump(0.03)
        precondition(notifications==1,"A stalled main loop accumulated readiness jobs instead of one coalesced notification")
        precondition(callbacks==0 && requests==0,"A busy presenter must retain its image instead of consuming it or requesting IDRs")
        guard let newest=manual.takeLatest() else{fatalError("Busy display lost its newest pending image")}
        CVPixelBufferLockBaseAddress(newest,.readOnly)
        let luminance=CVPixelBufferGetBaseAddressOfPlane(newest,0)!.load(as:UInt8.self)
        CVPixelBufferUnlockBaseAddress(newest,.readOnly)
        precondition(luminance>130,"Presenter retained an old decoded picture instead of the newest")
        precondition(manual.takeLatest()==nil,"Presenter duplicated the same pending picture")
        manual.reset();pump(0.05)
        precondition(manual.takeLatest()==nil && manual.stats()["iOSDecodedFrames"]==0,"Session reset retained an old image or old timing counters")

        let recovery=DispatchSemaphore(value:0)
        manual.keyframe={requests+=1;recovery.signal()}
        manual.push(Data(repeating:0,count:8*1024*1024+1),key:false,stream:manual.stream())
        for _ in 0..<100{manual.push(encoded.frames[1],key:false,stream:manual.stream())}
        precondition(recovery.wait(timeout:.now()+2) == .success,"A genuinely over-budget decode queue did not request recovery")
        precondition(manual.stats()["iOSDecodeOverflows"]==1 && requests==1,"One overload episode generated repeated resets/keyframe requests")
        manual.push(encoded.frames[0],key:true,stream:manual.stream())
        let restarted=Date(timeIntervalSinceNow:5)
        while manual.takeLatest()==nil && Date()<restarted{Thread.sleep(forTimeInterval:0.001)}
        precondition(manual.stats()["iOSDecodedFrames"]==1,"A fresh keyframe did not resume after bounded overflow recovery")
        manual.reset();pump(0.05)

        let healthURL=FileManager.default.temporaryDirectory.appendingPathComponent("sanser-video-health-test-\(UUID().uuidString).json")
        defer{try? FileManager.default.removeItem(at:healthURL)}
        let health=MobileVideoHealthLog(url:healthURL)
        for i in 0..<125{health.record(["sampleTimeMs":i,"iOSPresentedFrames":i,"rttMs":Double.nan,"token":123,"endpoint":"private","screen":"private"])}
        health.flush()
        let document=try! JSONSerialization.jsonObject(with:Data(contentsOf:healthURL)) as! [String:Any]
        let samples=document["samples"] as! [[String:Double]]
        precondition(samples.count==120 && samples.first?["sampleTimeMs"]==5,"Video diagnostics must be bounded to 120 numeric samples")
        precondition(samples.last?.keys.sorted()==["iOSPresentedFrames","sampleTimeMs"],"Video diagnostics retained credentials/content or nonfinite timing")
        health.reset();health.flush()
        print("VideoToolbox: reference chain, newest-image mailbox, stale-session fence and restart passed")
        print("Presentation: coalesced immediate readiness, newest-only backpressure, no duplicate/stale image; bounded numeric diagnostics passed")
        print("Repair burst: 24 ordered references, bounded compressed memory, one reset/request per overload and fresh-IDR recovery passed")
    }
}
