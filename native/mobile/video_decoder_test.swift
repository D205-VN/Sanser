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
        for frame in 0..<12 {
            var image:CVPixelBuffer?
            precondition(CVPixelBufferCreate(kCFAllocatorDefault,64,64,kCVPixelFormatType_32BGRA,nil,&image)==kCVReturnSuccess)
            CVPixelBufferLockBaseAddress(image!,[])
            memset(CVPixelBufferGetBaseAddress(image!)!,Int32(32+frame*12),CVPixelBufferGetBytesPerRow(image!)*64)
            CVPixelBufferUnlockBaseAddress(image!,[])
            precondition(VTCompressionSessionEncodeFrame(session,imageBuffer:image!,presentationTimeStamp:CMTime(value:Int64(frame),timescale:60),duration:CMTime(value:1,timescale:60),frameProperties:nil,sourceFrameRefcon:nil,infoFlagsOut:nil)==noErr)
        }
        VTCompressionSessionCompleteFrames(session,untilPresentationTimeStamp:.invalid);VTCompressionSessionInvalidate(session)
        precondition(encoded.frames.count==12,"Encoder did not produce the reference chain")

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
        print("VideoToolbox: reference chain, newest-image mailbox, stale-session fence and restart passed")
    }
}
