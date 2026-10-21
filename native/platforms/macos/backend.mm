#include "telemetry.h"
#import <Cocoa/Cocoa.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <VideoToolbox/VideoToolbox.h>
#import <CoreImage/CoreImage.h>
#import <ApplicationServices/ApplicationServices.h>
#include "transport.h"
#include "broadcast.h"
#include "voice_runtime.h"
#include "voice_panel.h"
#include "engine.h"
#include "capture_startup.h"
#include "audio_capture.h"
#include "audio_player.h"
#include "input_injector.h"
#include "latest_frame_worker.h"
#include "decode_worker.h"
#include "h264_low_delay.h"
#import "metal_presenter.h"
#include "keyboard.h"
#include <algorithm>
#include <iostream>
#include <memory>
#include <set>
#include <random>
#include <fstream>
#include <cstdlib>

using namespace sanser::desktop;
namespace {
void requireStatus(OSStatus status, const char* operation) { if(status!=noErr) throw std::runtime_error(std::string(operation)+" failed ("+std::to_string(status)+")"); }
std::vector<Bytes> nals(const Bytes& bytes) {
  std::vector<Bytes> result;
  auto start=[&](std::size_t i) -> std::size_t { if(i+3<=bytes.size() && bytes[i]==0 && bytes[i+1]==0) { if(bytes[i+2]==1) return 3; if(i+4<=bytes.size() && bytes[i+2]==0 && bytes[i+3]==1) return 4; } return 0; };
  std::size_t i=0;
  while(i<bytes.size()) { const auto prefix=start(i); if(!prefix) { ++i; continue; } const auto begin=i+prefix; i=begin; while(i<bytes.size() && !start(i)) ++i; if(i>begin) result.emplace_back(bytes.begin()+static_cast<std::ptrdiff_t>(begin),bytes.begin()+static_cast<std::ptrdiff_t>(i)); }
  return result;
}
void appendNal(Bytes& out,const std::uint8_t* data,std::size_t size) { out.insert(out.end(),{0,0,0,1}); out.insert(out.end(),data,data+size); }
struct CapturedFrame {
  std::shared_ptr<__CVBuffer> pixels;
  CMTime time;
  Clock::time_point captured;
  CapturedFrame(CVPixelBufferRef value,CMTime timestamp):pixels(CVPixelBufferRetain(value),[](CVPixelBufferRef p) { CVPixelBufferRelease(p); }),time(timestamp),captured(Clock::now()) {}
};
class Encoder {
public:
  Encoder(const Options& options,Peer& peer) : Encoder(options,[&peer] { return peer.ready(); },[&peer](Frame frame) { peer.video(frame); }) {}
  Encoder(const Options& options,HostOutput& output) : Encoder(options,[&output] { return output.ready(); },[&output](Frame frame) { output.video(std::move(frame)); }) {}
  Encoder(const Options& options,std::function<bool()> ready,std::function<void(Frame)> publish)
      : options_(options),ready_(std::move(ready)),publish_(std::move(publish)) {
    requireStatus(VTCompressionSessionCreate(nullptr,options.width,options.height,kCMVideoCodecType_H264,nullptr,nullptr,nullptr,output,this,&session_),"Create H.264 encoder");
    VTSessionSetProperty(session_,kVTCompressionPropertyKey_RealTime,kCFBooleanTrue);
    requireStatus(VTSessionSetProperty(session_,kVTCompressionPropertyKey_AllowFrameReordering,kCFBooleanFalse),"Disable H.264 frame reordering");
    // RealTime alone is a recommendation. Explicitly disable the compression
    // window as well; the per-picture completion below bounds older encoders
    // that reject this optional property.
    const auto delayStatus=VTSessionSetProperty(session_,kVTCompressionPropertyKey_MaxFrameDelayCount,(__bridge CFNumberRef)@0);
    requireStatus(VTSessionSetProperty(session_,kVTCompressionPropertyKey_ProfileLevel,kVTProfileLevel_H264_Baseline_AutoLevel),"Select H.264 Baseline");
    NSDictionary* values=@{(__bridge NSString*)kVTCompressionPropertyKey_AverageBitRate:@(options.bitrate),(__bridge NSString*)kVTCompressionPropertyKey_ExpectedFrameRate:@(options.fps),(__bridge NSString*)kVTCompressionPropertyKey_MaxKeyFrameInterval:@(options.fps)};
    requireStatus(VTSessionSetProperties(session_,(__bridge CFDictionaryRef)values),"Configure H.264 encoder");
    requireStatus(VTCompressionSessionPrepareToEncodeFrames(session_),"Prepare encoder");
    std::cout<<"SNV2_ENCODER maxFrameDelay="<<(delayStatus==noErr ? 0 : -1)<<" pendingCaptureLimit=1 completionPerFrame=1"<<std::endl;
    frames_=std::make_unique<LatestFrameWorker<CapturedFrame>>([this](CapturedFrame frame) {
      @autoreleasepool {
        try { submit(frame); } catch(const std::exception& error) { std::cerr<<"SNV2_ENCODE failed: "<<error.what()<<std::endl; failed_=true; }
      }
    });
  }
  ~Encoder() { if(frames_) frames_->stop(); if(session_) { VTCompressionSessionCompleteFrames(session_,kCMTimeInvalid); VTCompressionSessionInvalidate(session_); CFRelease(session_); } if(lastPixels_) CVPixelBufferRelease(lastPixels_); }
  void encode(CVPixelBufferRef pixels,CMTime time) {
    std::lock_guard lock(encodeMutex_);
    if(lastPixels_) CVPixelBufferRelease(lastPixels_);
    lastPixels_=CVPixelBufferRetain(pixels);
    if(CMTimeCompare(time,lastTime_)<=0) time=CMTimeAdd(lastTime_,CMTimeMake(1,options_.fps));
    lastTime_=time;
    if(ready_()) { ++captured_; frames_->submit(CapturedFrame(pixels,time)); }
  }
  void refresh() {
    std::lock_guard lock(encodeMutex_);
    if(!lastPixels_ || !ready_()) return;
    lastTime_=CMTimeAdd(lastTime_,CMTimeMake(1,options_.fps));
    ++captured_; frames_->submit(CapturedFrame(lastPixels_,lastTime_));
  }
  void submit(const CapturedFrame& frame) {
    const auto began=Clock::now();
    queueUs_+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(began-frame.captured).count());
    ++submitted_;
    std::lock_guard sessionLock(sessionMutex_);
    const bool force=force_.exchange(false);
    const auto properties=force ? (__bridge CFDictionaryRef)@{(__bridge NSString*)kVTEncodeFrameOptionKey_ForceKeyFrame:@YES} : nullptr;
    requireStatus(VTCompressionSessionEncodeFrame(session_,frame.pixels.get(),frame.time,CMTimeMake(1,options_.fps),properties,nullptr,nullptr),"Encode screen frame");
    // Only one encoded picture may be in flight. Discard superseded RAW
    // captures in frames_, never encoded P-frame references. This also delivers
    // the final idle-desktop picture without waiting for another capture.
    requireStatus(VTCompressionSessionCompleteFrames(session_,kCMTimeInvalid),"Complete realtime frame");
    encodeAndPublishUs_+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(Clock::now()-began).count());
    ++completed_;
  }
  void keyframe() { force_=true; }
  bool setBitrate(std::uint32_t bitrate) {
    std::lock_guard lock(sessionMutex_);
    return VTSessionSetProperty(session_,kVTCompressionPropertyKey_AverageBitRate,(__bridge CFNumberRef)@(bitrate))==noErr;
  }
  bool failed() const { return failed_; }
  bool takeRetryRequest() { return retry_.exchange(false); }
  void flush() { frames_->drain(); }
  void logProgress() {
    if(!std::getenv("SANSER_TELEMETRY")) return;
    const auto captured=captured_.load(),submitted=submitted_.load(),completed=completed_.load();
    std::ostringstream line; line<<"SNV2_HOST_TIMING captured="<<captured<<" submitted="<<submitted<<" completed="<<completed<<" encoderDropped="<<encoderDropped_.load();
    if(submitted) line<<" encodeQueueAvgMs="<<queueUs_.load()/1000.0/submitted;
    if(completed) line<<" encodeAndPublishAvgMs="<<encodeAndPublishUs_.load()/1000.0/completed;
    writeTelemetry(line);
  }
private:
  static void output(void* context,void*,OSStatus status,VTEncodeInfoFlags flags,CMSampleBufferRef sample) {
    auto& self=*static_cast<Encoder*>(context);
    if(status==noErr && (flags&kVTEncodeInfo_FrameDropped)) { ++self.encoderDropped_; self.retry_=true; return; }
    if(status!=noErr || !sample || !CMSampleBufferDataIsReady(sample)) { self.failed_=true; return; }
    try {
      Frame frame; frame.width=self.options_.width; frame.height=self.options_.height;
      const auto attachments=CMSampleBufferGetSampleAttachmentsArray(sample,false);
      frame.keyframe=!attachments || CFArrayGetCount(attachments)==0 || !CFDictionaryContainsKey(static_cast<CFDictionaryRef>(CFArrayGetValueAtIndex(attachments,0)),kCMSampleAttachmentKey_NotSync);
      const auto format=CMSampleBufferGetFormatDescription(sample);
      if(frame.keyframe) {
        for(std::size_t index=0;index<2;++index) { const std::uint8_t* data=nullptr; std::size_t size=0,count=0; int header=0;
          if(CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format,index,&data,&size,&count,&header)!=noErr) { self.failed_=true; return; }
          const auto parameter=sanser::lowDelayBaselineSps(std::span(data,size));
          appendNal(frame.data,parameter.data(),parameter.size());
        }
      }
      const auto block=CMSampleBufferGetDataBuffer(sample); const auto size=CMBlockBufferGetDataLength(block);
      if(size>4*1024*1024) return;
      Bytes data(size); requireStatus(CMBlockBufferCopyDataBytes(block,0,size,data.data()),"Read encoded frame");
      std::size_t offset=0;
      while(offset+4<=data.size()) { std::uint32_t length=0; for(int i=0;i<4;++i) length=(length<<8)|data[offset++]; if(length==0 || length>data.size()-offset) return; appendNal(frame.data,data.data()+offset,length); offset+=length; }
      self.publish_(std::move(frame));
    } catch(...) { self.failed_=true; }
  }
  Options options_; std::function<bool()> ready_; std::function<void(Frame)> publish_;
  VTCompressionSessionRef session_=nullptr;
  CVPixelBufferRef lastPixels_=nullptr; CMTime lastTime_=kCMTimeZero; std::mutex encodeMutex_;
  std::mutex sessionMutex_;
  std::unique_ptr<LatestFrameWorker<CapturedFrame>> frames_;
  std::atomic<std::uint64_t> captured_{0},submitted_{0},completed_{0},encoderDropped_{0},queueUs_{0},encodeAndPublishUs_{0};
  std::atomic<bool> force_{true},failed_{false},retry_{false};
};
class Decoder {
public:
  ~Decoder() { clear(); std::lock_guard lock(mutex_); if(latest_) CVPixelBufferRelease(latest_); }
  void decode(const Frame& frame) {
    checkOutput();
    Bytes data; auto parts=nals(frame.data); Bytes sps=sps_,pps=pps_;
    for(auto& nal:parts) { const auto type=nal[0]&31; if(type==7) sps=nal; else if(type==8) pps=nal; else if(type==1 || type==5) { const auto size=static_cast<std::uint32_t>(nal.size()); for(int i=3;i>=0;--i) data.push_back(static_cast<std::uint8_t>(size>>(i*8))); data.insert(data.end(),nal.begin(),nal.end()); } }
    if(sps.empty() || pps.empty() || data.empty()) return;
    if(!session_ || sps!=sps_ || pps!=pps_) {
      clear(); sps_=sps; pps_=pps;
      const std::uint8_t* parameters[]={sps_.data(),pps_.data()}; const std::size_t sizes[]={sps_.size(),pps_.size()};
      requireStatus(CMVideoFormatDescriptionCreateFromH264ParameterSets(nullptr,2,parameters,sizes,4,&format_),"Read H.264 parameters");
      const auto dimensions=CMVideoFormatDescriptionGetDimensions(format_);
      if(dimensions.width<64 || dimensions.height<64 || dimensions.width>3840 || dimensions.height>2160) throw std::runtime_error("Invalid H.264 dimensions");
      VTDecompressionOutputCallbackRecord callback{output,this};
      NSDictionary* attributes=@{(__bridge NSString*)kCVPixelBufferPixelFormatTypeKey:@(kCVPixelFormatType_32BGRA),(__bridge NSString*)kCVPixelBufferIOSurfacePropertiesKey:@{}};
      requireStatus(VTDecompressionSessionCreate(nullptr,format_,nullptr,(__bridge CFDictionaryRef)attributes,&callback,&session_),"Create H.264 decoder");
    }
    CMBlockBufferRef block=nullptr; CMSampleBufferRef sample=nullptr;
    requireStatus(CMBlockBufferCreateWithMemoryBlock(nullptr,nullptr,data.size(),nullptr,nullptr,0,data.size(),0,&block),"Allocate frame");
    auto status=CMBlockBufferReplaceDataBytes(data.data(),block,0,data.size());
    const auto size=data.size();
    if(status==noErr) status=CMSampleBufferCreateReady(nullptr,block,format_,1,0,nullptr,1,&size,&sample);
    if(status==noErr) status=VTDecompressionSessionDecodeFrame(session_,sample,0,nullptr,nullptr);
    if(sample) CFRelease(sample); CFRelease(block); requireStatus(status,"Decode frame");
    // With decode flags zero, VideoToolbox finishes the output callback before
    // returning. A successful submit can still have an unsuccessful output.
    // Surface that failure to DecodeWorker's reset/IDR recovery path.
    checkOutput();
  }
  CVPixelBufferRef take() { std::lock_guard lock(mutex_); auto value=latest_; latest_=nullptr; return value; }
  void reset() {
    clear(); sps_.clear(); pps_.clear(); outputStatus_=noErr;
    std::lock_guard lock(mutex_); if(latest_) { CVPixelBufferRelease(latest_); latest_=nullptr; }
  }
  std::uint64_t decoded() const { return decoded_.load(); }
  std::uint64_t outputErrors() const { return outputErrors_.load(); }
  std::uint64_t outputDrops() const { return outputDrops_.load(); }
#ifdef SANSER_CODEC_TEST
  // Exercise the real output callback without corrupting captured user data.
  void testOutputFailure(OSStatus status,VTDecodeInfoFlags flags) { output(this,nullptr,status,flags,nullptr,kCMTimeInvalid,kCMTimeInvalid); }
#endif
private:
  void checkOutput() { requireStatus(outputStatus_.exchange(noErr),"Decode output"); }
  void clear() { if(session_) { VTDecompressionSessionWaitForAsynchronousFrames(session_); VTDecompressionSessionInvalidate(session_); CFRelease(session_); session_=nullptr; } if(format_) { CFRelease(format_); format_=nullptr; } }
  static void output(void* context,void*,OSStatus status,VTDecodeInfoFlags flags,CVImageBufferRef image,CMTime,CMTime) {
    auto& self=*static_cast<Decoder*>(context);
    if(status!=noErr || !image || (flags&kVTDecodeInfo_FrameDropped)) {
      if(status!=noErr) ++self.outputErrors_; else ++self.outputDrops_;
      self.outputStatus_=status!=noErr ? status : kVTVideoDecoderBadDataErr;
      return;
    }
    ++self.decoded_; std::lock_guard lock(self.mutex_); if(self.latest_) CVPixelBufferRelease(self.latest_); self.latest_=CVPixelBufferRetain(image); if(!self.reported_) { self.reported_=true; std::cout<<"SNV2_FIRST_FRAME decoded=1"<<std::endl; }
  }
  Bytes sps_,pps_; VTDecompressionSessionRef session_=nullptr; CMVideoFormatDescriptionRef format_=nullptr; CVPixelBufferRef latest_=nullptr; std::mutex mutex_; bool reported_=false;
  std::atomic<std::uint64_t> decoded_{0};
  std::atomic<OSStatus> outputStatus_{noErr};
  std::atomic<std::uint64_t> outputErrors_{0},outputDrops_{0};
};

}

API_AVAILABLE(macos(12.3))
@interface SanserCapture : NSObject <SCStreamOutput,SCStreamDelegate>
@property(nonatomic,assign) Encoder* encoder;
@property(nonatomic,assign) AudioPacketizer* audio;
@property(atomic,assign) BOOL failed;
@end
@implementation SanserCapture
- (void)stream:(SCStream*)stream didOutputSampleBuffer:(CMSampleBufferRef)sample ofType:(SCStreamOutputType)type {
  (void)stream;
  if(@available(macOS 13.0,*)) {
    if(type==SCStreamOutputTypeAudio) { if(self.audio) captureSystemAudio(sample,*self.audio); return; }
  }
  if(type!=SCStreamOutputTypeScreen || !CMSampleBufferIsValid(sample) || !self.encoder) return;
  auto image=CMSampleBufferGetImageBuffer(sample); if(image) self.encoder->encode(image,CMSampleBufferGetPresentationTimeStamp(sample));
}
- (void)stream:(SCStream*)stream didStopWithError:(NSError*)error { (void)stream; std::cerr<<captureFailure(error,"Screen capture stopped")<<std::endl; self.failed=YES; }
@end

@interface SanserSharingStop : NSObject
@property(nonatomic,assign) HostOutput* output;
- (void)stopSharing:(id)sender;
@end
@implementation SanserSharingStop
- (void)stopSharing:(id)sender { (void)sender; if(self.output) self.output->stop(); }
@end

@interface SanserSnv2RemoteView : SanserSnv2MetalView
@property(nonatomic,assign) Peer* peer;
@property(nonatomic,assign) BOOL inputEnabled;
@property(nonatomic,assign) BOOL released;
- (void)sendInput:(Input)input;
- (void)pointer:(NSEvent*)event kind:(Input::Kind)kind down:(BOOL)down;
@end
@implementation SanserSnv2RemoteView
- (BOOL)acceptsFirstResponder { return self.inputEnabled; }
- (void)sendInput:(Input)input { if(self.inputEnabled && self.peer) self.peer->input(input); }
- (void)pointer:(NSEvent*)event kind:(Input::Kind)kind down:(BOOL)down {
  if(!self.inputEnabled || self.released || self.imageSize.width<=0 || self.imageSize.height<=0 || self.bounds.size.width<=0 || self.bounds.size.height<=0) return;
  auto point=[self convertPoint:event.locationInWindow fromView:nil]; const auto bounds=self.bounds;
  const auto size=self.imageSize; const auto scale=std::min(bounds.size.width/size.width,bounds.size.height/size.height);
  const auto width=size.width*scale,height=size.height*scale; point.x-=(bounds.size.width-width)/2; point.y-=(bounds.size.height-height)/2;
  const double y=std::clamp(point.y/height,0.0,1.0);
  Input input; input.kind=kind; input.x=static_cast<std::uint16_t>(std::clamp(point.x/width,0.0,1.0)*65535); input.y=static_cast<std::uint16_t>((self.isFlipped ? y : 1-y)*65535); input.down=down;
  // AppKit asserts if a scroll-only property is read on a mouse-move/button
  // event. View-only must return before inspecting any input-only fields too.
  if(kind==Input::Scroll) {
    if(event.type!=NSEventTypeScrollWheel) return;
    input.delta=static_cast<std::int16_t>(std::clamp(event.scrollingDeltaY,-120.0,120.0));
  } else if(kind==Input::Button) {
    switch(event.type) {
      case NSEventTypeLeftMouseDown: case NSEventTypeLeftMouseUp: input.code=0; break;
      case NSEventTypeRightMouseDown: case NSEventTypeRightMouseUp: input.code=1; break;
      case NSEventTypeOtherMouseDown: case NSEventTypeOtherMouseUp:
        if(event.buttonNumber!=2) return;
        input.code=2; break;
      default: return;
    }
  }
  [self sendInput:input];
}
- (void)mouseMoved:(NSEvent*)event { [self pointer:event kind:Input::Move down:NO]; }
- (void)mouseDragged:(NSEvent*)event { [self pointer:event kind:Input::Move down:NO]; }
- (void)rightMouseDragged:(NSEvent*)event { [self pointer:event kind:Input::Move down:NO]; }
- (void)otherMouseDragged:(NSEvent*)event { [self pointer:event kind:Input::Move down:NO]; }
- (void)mouseDown:(NSEvent*)event { if(!self.inputEnabled) return; if(self.released) { self.released=NO; self.window.title=@"Sanser · Remote desktop · Control+Option+Escape releases input"; } [self.window makeFirstResponder:self]; [self pointer:event kind:Input::Button down:YES]; }
- (void)mouseUp:(NSEvent*)event { [self pointer:event kind:Input::Button down:NO]; }
- (void)rightMouseDown:(NSEvent*)event { [self pointer:event kind:Input::Button down:YES]; }
- (void)rightMouseUp:(NSEvent*)event { [self pointer:event kind:Input::Button down:NO]; }
- (void)otherMouseDown:(NSEvent*)event { [self pointer:event kind:Input::Button down:YES]; }
- (void)otherMouseUp:(NSEvent*)event { [self pointer:event kind:Input::Button down:NO]; }
- (void)scrollWheel:(NSEvent*)event { [self pointer:event kind:Input::Scroll down:NO]; }
- (void)keyDown:(NSEvent*)event {
  if(!self.inputEnabled) return;
  if(event.keyCode==53 && (event.modifierFlags & NSEventModifierFlagControl) && (event.modifierFlags & NSEventModifierFlagOption)) { self.released=YES; [self sendInput:Input{}]; self.window.title=@"Sanser · Input released · Click the stream to resume"; return; }
  if(self.released) return; const auto hid=hidFromMac(event.keyCode); if(hid) [self sendInput:Input{Input::Key,0,0,hid,0,true}];
}
- (void)keyUp:(NSEvent*)event { if(self.inputEnabled && !self.released) { const auto hid=hidFromMac(event.keyCode); if(hid) [self sendInput:Input{Input::Key,0,0,hid,0,false}]; } }
- (void)flagsChanged:(NSEvent*)event {
  if(!self.inputEnabled || self.released) return; const auto hid=hidFromMac(event.keyCode); NSEventModifierFlags flag=0;
  if(hid==224||hid==228) flag=NSEventModifierFlagControl; if(hid==225||hid==229) flag=NSEventModifierFlagShift; if(hid==226||hid==230) flag=NSEventModifierFlagOption; if(hid==227||hid==231) flag=NSEventModifierFlagCommand;
  if(flag) [self sendInput:Input{Input::Key,0,0,hid,0,(event.modifierFlags&flag)!=0}];
}
@end

#ifdef SANSER_CODEC_TEST
// Exercise the real AppKit callbacks without sending packets or OS input.
@interface SanserSnv2InputTestView : SanserSnv2RemoteView {
@public
  std::vector<Input> events;
}
@end
@implementation SanserSnv2InputTestView
- (void)sendInput:(Input)input { events.push_back(input); }
@end
#endif

namespace sanser::desktop {
bool macHostAvailable() { if (@available(macOS 12.3, *)) return true; return false; }
bool macSystemAudioAvailable() { if (@available(macOS 13.0, *)) return true; return false; }
struct API_AVAILABLE(macos(12.3)) CaptureStop {
  SCStream* __strong stream;
  SanserCapture* __strong capture;
  dispatch_queue_t queue;
  dispatch_queue_t audioQueue;
  ~CaptureStop() {
    dispatch_sync(queue, ^{ capture.encoder=nullptr; });
    dispatch_sync(audioQueue, ^{ capture.audio=nullptr; });
    [stream stopCaptureWithCompletionHandler:^(NSError*) {}];
  }
};
struct SharingNotice {
  NSPanel* __strong panel=nil;
  SanserSharingStop* __strong action=nil;
  SharingNotice(HostOutput& output,bool enabled) {
    if(!enabled) return;
    [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    const auto area=NSScreen.mainScreen.visibleFrame;
    panel=[[NSPanel alloc] initWithContentRect:NSMakeRect(NSMaxX(area)-380,NSMaxY(area)-76,360,56) styleMask:NSWindowStyleMaskBorderless|NSWindowStyleMaskNonactivatingPanel backing:NSBackingStoreBuffered defer:NO];
    panel.releasedWhenClosed=NO; panel.level=NSFloatingWindowLevel; panel.movableByWindowBackground=YES; panel.hidesOnDeactivate=NO;
    panel.collectionBehavior=NSWindowCollectionBehaviorCanJoinAllSpaces|NSWindowCollectionBehaviorFullScreenAuxiliary;
    NSTextField* label=[NSTextField labelWithString:@"Sanser · Screen sharing active"]; label.frame=NSMakeRect(12,18,240,22); [panel.contentView addSubview:label];
    action=[SanserSharingStop new]; action.output=&output;
    NSButton* stop=[NSButton buttonWithTitle:@"Stop" target:action action:@selector(stopSharing:)]; stop.frame=NSMakeRect(258,10,88,34); [panel.contentView addSubview:stop];
    [panel orderFrontRegardless];
  }
  ~SharingNotice() { if(action) action.output=nullptr; [panel close]; }
  void pump() { if(!panel) return; NSEvent* event; while((event=[NSApp nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:YES])) [NSApp sendEvent:event]; }
};
#ifdef SANSER_CODEC_TEST
void testMacEncoderBackpressure() {
  @autoreleasepool {
    Options options; options.width=640; options.height=360;
    const auto pixels=[](std::uint8_t blue,std::uint8_t red) {
      CVPixelBufferRef value=nullptr;
      requireStatus(CVPixelBufferCreate(nullptr,640,360,kCVPixelFormatType_32BGRA,nullptr,&value),"Create encoder-pressure image");
      std::shared_ptr<__CVBuffer> owner(value,[](CVPixelBufferRef p) { CVPixelBufferRelease(p); });
      CVPixelBufferLockBaseAddress(value,0);
      auto* data=static_cast<std::uint8_t*>(CVPixelBufferGetBaseAddress(value)); const auto stride=CVPixelBufferGetBytesPerRow(value);
      for(unsigned y=0;y<360;++y) for(unsigned x=0;x<640;++x) { auto* p=data+y*stride+x*4; p[0]=blue; p[1]=20; p[2]=red; p[3]=255; }
      CVPixelBufferUnlockBaseAddress(value,0); return owner;
    };
    auto red=pixels(20,210),blue=pixels(210,20);
    std::mutex mutex; std::condition_variable wake; bool entered=false,release=false;
    std::vector<Frame> delivered;
    Encoder encoder(options,[] { return true; },[&](Frame frame) {
      std::unique_lock lock(mutex); delivered.push_back(std::move(frame));
      if(!entered) { entered=true; wake.notify_all(); wake.wait_for(lock,std::chrono::seconds(3),[&] { return release; }); }
    });
    struct Release { std::mutex& mutex; std::condition_variable& wake; bool& release; ~Release() { { std::lock_guard lock(mutex); release=true; } wake.notify_all(); } } cleanup{mutex,wake,release};
    encoder.encode(red.get(),CMTimeMake(1,60));
    { std::unique_lock lock(mutex); if(!wake.wait_for(lock,std::chrono::seconds(2),[&] { return entered; })) throw std::runtime_error("Encoder did not publish its first frame"); }
    const auto began=Clock::now();
    for(unsigned i=2;i<102;++i) encoder.encode(red.get(),CMTimeMake(i,60));
    encoder.encode(blue.get(),CMTimeMake(102,60));
    if(Clock::now()-began>std::chrono::milliseconds(100)) throw std::runtime_error("Blocked encoded delivery stalled the capture callback");
    { std::lock_guard lock(mutex); release=true; } wake.notify_all();
    encoder.flush();
    if(encoder.failed() || delivered.size()!=2 || !delivered.front().keyframe)
      throw std::runtime_error("Encoder replayed stale captures or lost its reference keyframe");
    Decoder decoder; for(const auto& frame:delivered) decoder.decode(frame);
    auto latest=decoder.take(); if(!latest) throw std::runtime_error("Newest capture did not decode after encoder pressure");
    CVPixelBufferLockBaseAddress(latest,kCVPixelBufferLock_ReadOnly);
    const auto* color=static_cast<const std::uint8_t*>(CVPixelBufferGetBaseAddress(latest));
    const bool matches=color[0]>150 && color[1]<80 && color[2]<80;
    CVPixelBufferUnlockBaseAddress(latest,kCVPixelBufferLock_ReadOnly); CVPixelBufferRelease(latest);
    if(!matches) throw std::runtime_error("Final idle-desktop picture remained stale after encoder pressure");
    for(const auto failure : {std::pair<OSStatus,VTDecodeInfoFlags>{kVTVideoDecoderBadDataErr,0},
        {noErr,kVTDecodeInfo_FrameDropped},{noErr,0}}) {
      Decoder damaged; std::atomic<unsigned> calls{0},requests{0};
      DecodeWorker recovery([&](Frame frame) {
        if(++calls==1) damaged.testOutputFailure(failure.first,failure.second);
        damaged.decode(frame);
      },[&] { damaged.reset(); },[&] { ++requests; },true);
      recovery.submit(delivered.front());
      const auto failedBy=Clock::now()+std::chrono::seconds(2);
      while(requests==0 && Clock::now()<failedBy) std::this_thread::sleep_for(std::chrono::milliseconds(1));
      if(requests!=1) throw std::runtime_error("Decoder callback failure left the viewer frozen without requesting recovery");
      recovery.submit(delivered.front()); // Fresh, independently decodable IDR.
      const auto recoveredBy=Clock::now()+std::chrono::seconds(2);
      while(damaged.decoded()==0 && Clock::now()<recoveredBy) std::this_thread::sleep_for(std::chrono::milliseconds(1));
      if(damaged.decoded()!=1 || requests!=1) throw std::runtime_error("Decoder output failure did not recover at the next keyframe");
      recovery.stop();
    }
    std::cout<<"Mac encoder: blocked delivery retained only newest raw capture; reference chain and final idle picture verified\n";
  }
}
void writeMacInteropFixture(const char* path) {
  @autoreleasepool {
    Options options; options.width=1846; options.height=1200; options.bitrate=6000000;
    std::mutex mutex; std::vector<Frame> frames;
    {
      Encoder encoder(options,[] { return true; },[&](Frame frame) { std::lock_guard lock(mutex); frames.push_back(std::move(frame)); });
      for(unsigned index=0;index<45;++index) {
        CVPixelBufferRef pixels=nullptr;
        NSDictionary* attributes=@{(__bridge NSString*)kCVPixelBufferIOSurfacePropertiesKey:@{}};
        requireStatus(CVPixelBufferCreate(nullptr,options.width,options.height,kCVPixelFormatType_32BGRA,(__bridge CFDictionaryRef)attributes,&pixels),"Create interop frame");
        CVPixelBufferLockBaseAddress(pixels,0);
        auto* data=static_cast<std::uint8_t*>(CVPixelBufferGetBaseAddress(pixels)); const auto stride=CVPixelBufferGetBytesPerRow(pixels);
        for(unsigned y=0;y<options.height;++y) for(unsigned x=0;x<options.width;++x) {
          auto* color=data+y*stride+x*4; color[0]=index>=30 ? 210 : 20;
          color[1]=index>=15 && index<30 ? 210 : 20; color[2]=index<15 ? 210 : 20; color[3]=255;
        }
        CVPixelBufferUnlockBaseAddress(pixels,0);
        encoder.encode(pixels,CMTimeMake(index+1,60)); CVPixelBufferRelease(pixels); encoder.flush();
      }
    } // Flush all VideoToolbox callbacks before writing the synthetic fixture.
    if(frames.size()!=45 || !frames.front().keyframe) throw std::runtime_error("Incomplete Mac interop fixture");
    std::ofstream file(path,std::ios::binary);
    const auto put=[&](std::uint32_t value) { for(int i=3;i>=0;--i) file.put(static_cast<char>(value>>(i*8))); };
    file.write("SVC1",4); put(options.width); put(options.height); put(static_cast<std::uint32_t>(frames.size()));
    for(const auto& frame:frames) { put(static_cast<std::uint32_t>(frame.data.size())); file.put(frame.keyframe ? 1 : 0); file.write(reinterpret_cast<const char*>(frame.data.data()),static_cast<std::streamsize>(frame.data.size())); }
    if(!file) throw std::runtime_error("Unable to write interop fixture");
  }
}
// Link this test with the real legacy client too: standalone codec tests cannot
// detect a presenter class symbol resolved to the legacy implementation.
void testMacRemoteView() {
  @autoreleasepool {
    [NSApplication sharedApplication];
    SanserSnv2InputTestView* view=[[SanserSnv2InputTestView alloc] initWithFrame:NSMakeRect(0,0,320,200)];
    if(![view respondsToSelector:@selector(presentPixels:)] || ![view respondsToSelector:@selector(imageSize)])
      throw std::runtime_error("SNV2 presenter resolved to a legacy class without presentPixels:/imageSize");
    CVPixelBufferRef pixels=nullptr;
    requireStatus(CVPixelBufferCreate(nullptr,64,32,kCVPixelFormatType_32BGRA,nullptr,&pixels),"Create presenter regression frame");
    [view presentPixels:pixels]; CVPixelBufferRelease(pixels);
    if(view.imageSize.width!=64 || view.imageSize.height!=32) throw std::runtime_error("SNV2 view did not retain the decoded frame");
    const auto mouse=[](NSEventType type) {
      return [NSEvent mouseEventWithType:type location:NSMakePoint(160,100) modifierFlags:0 timestamp:0 windowNumber:0 context:nil eventNumber:1 clickCount:1 pressure:1];
    };
    auto move=mouse(NSEventTypeMouseMoved),left=mouse(NSEventTypeLeftMouseDown),up=mouse(NSEventTypeLeftMouseUp);
    auto right=mouse(NSEventTypeRightMouseDown),drag=mouse(NSEventTypeLeftMouseDragged);
    CGEventRef scrollEvent=CGEventCreateScrollWheelEvent(nullptr,kCGScrollEventUnitLine,1,3);
    NSEvent* scroll=[NSEvent eventWithCGEvent:scrollEvent]; CFRelease(scrollEvent);
    auto key=[NSEvent keyEventWithType:NSEventTypeKeyDown location:NSZeroPoint modifierFlags:0 timestamp:0 windowNumber:0 context:nil characters:@"a" charactersIgnoringModifiers:@"a" isARepeat:NO keyCode:0];
    @try {
      // A group viewer must neither send input nor enter the released/resume
      // state, even if the cursor is moved/clicked over its video.
      view.inputEnabled=NO; view.released=NO;
      [view mouseDown:left]; [view mouseMoved:move]; [view scrollWheel:scroll]; [view keyDown:key];
      if(!view->events.empty() || view.released || view.acceptsFirstResponder)
        throw std::runtime_error("View-only captured input or changed control state");
      view.released=YES; [view mouseDown:left];
      if(!view->events.empty() || !view.released)
        throw std::runtime_error("View-only captured input or resumed control");
      view.inputEnabled=YES; view.released=NO;
      [view mouseMoved:move]; [view mouseDragged:drag]; [view mouseDown:left]; [view mouseUp:up]; [view rightMouseDown:right]; [view scrollWheel:scroll]; [view keyDown:key]; [view keyUp:key];
      const auto& events=view->events;
      if(events.size()!=8 || events[0].kind!=Input::Move || events[0].delta!=0 || events[1].kind!=Input::Move || events[2].kind!=Input::Button || !events[2].down || events[2].code!=0 || events[3].down || events[4].code!=1 || events[5].kind!=Input::Scroll || events[5].delta!=3 || events[6].code!=4 || !events[6].down || events[7].down)
        throw std::runtime_error("AppKit input type/button/wheel/key mapping is incorrect");
      [view pointer:move kind:Input::Scroll down:NO];
      [view pointer:move kind:Input::Button down:NO];
      if(events.size()!=8) throw std::runtime_error("Mismatched event accessed a type-specific field");
      view.released=YES; [view mouseMoved:move]; [view keyDown:key];
      if(events.size()!=8) throw std::runtime_error("Released input was forwarded");
      [view mouseDown:left]; [view mouseUp:up];
      if(events.size()!=10 || !events[8].down || events[9].down || view.released)
        throw std::runtime_error("Resume click lost its down/up pair");
    } @catch(NSException* exception) {
      throw std::runtime_error(std::string("AppKit event callback threw: ")+exception.reason.UTF8String);
    }
    std::cout<<"Mac remote view: real move/drag/button/wheel/key callbacks and view-only gate verified\n";
  }
}
// Exercises the actual VideoToolbox encoder/decoder without capturing a screen
// or injecting OS input. Used only by the native test executable.
void testMacVideoCodec() {
  @autoreleasepool {
    Options hostOptions; hostOptions.host=true; hostOptions.width=640; hostOptions.height=360;
    hostOptions.token=std::string(48,'t');
    std::unique_ptr<Peer> sender,receiver; std::random_device random;
    for(unsigned attempt=0;attempt<10 && !receiver;++attempt) {
      hostOptions.port=static_cast<std::uint16_t>(30000+random()%20000);
      hostOptions.peer="127.0.0.1:"+std::to_string(hostOptions.port+1);
      Options clientOptions=hostOptions; clientOptions.host=false; clientOptions.port=hostOptions.port+1;
      clientOptions.peer="127.0.0.1:"+std::to_string(hostOptions.port);
      try { sender=std::make_unique<Peer>(hostOptions); receiver=std::make_unique<Peer>(clientOptions); }
      catch(...) { sender.reset(); }
    }
    if(!receiver) throw std::runtime_error("Codec test could not bind loopback sockets");
    Encoder encoder(hostOptions,*sender); Decoder decoder;
    receiver->onFrame=[&](Frame frame) { decoder.decode(frame); };
    sender->onKeyframe=[&] { encoder.keyframe(); };
    PeerStop stopSender{*sender},stopReceiver{*receiver}; sender->start(); receiver->start();
    const auto deadline=Clock::now()+std::chrono::seconds(10);
    while((!sender->ready() || !receiver->ready()) && Clock::now()<deadline) std::this_thread::sleep_for(std::chrono::milliseconds(10));
    struct Pixel { CVPixelBufferRef value=nullptr; ~Pixel() { if(value) CVPixelBufferRelease(value); } } source,decoded;
    NSDictionary* attributes=@{(__bridge NSString*)kCVPixelBufferIOSurfacePropertiesKey:@{}};
    requireStatus(CVPixelBufferCreate(nullptr,640,360,kCVPixelFormatType_32BGRA,(__bridge CFDictionaryRef)attributes,&source.value),"Create test frame");
    CVPixelBufferLockBaseAddress(source.value,0);
    auto* bytes=static_cast<std::uint8_t*>(CVPixelBufferGetBaseAddress(source.value));
    const auto stride=CVPixelBufferGetBytesPerRow(source.value);
    for(unsigned y=0;y<360;++y) for(unsigned x=0;x<640;++x) { auto* pixel=bytes+y*stride+x*4; pixel[0]=y<180 ? 30 : 200; pixel[1]=100; pixel[2]=y<180 ? 200 : 30; pixel[3]=255; }
    CVPixelBufferUnlockBaseAddress(source.value,0);
    int frame=0;
    while(!decoded.value && Clock::now()<deadline) {
      encoder.encode(source.value,CMTimeMake(frame++,60));
      std::this_thread::sleep_for(std::chrono::milliseconds(20)); decoded.value=decoder.take();
    }
    if(!decoded.value || encoder.failed()) throw std::runtime_error("VideoToolbox loopback did not decode a frame");
    if(CVPixelBufferGetWidth(decoded.value)!=640 || CVPixelBufferGetHeight(decoded.value)!=360) throw std::runtime_error("Decoded frame dimensions differ");
    CVPixelBufferLockBaseAddress(decoded.value,kCVPixelBufferLock_ReadOnly);
    const auto* pixel=static_cast<const std::uint8_t*>(CVPixelBufferGetBaseAddress(decoded.value));
    const bool matches=std::abs(static_cast<int>(pixel[0])-30)<25 && std::abs(static_cast<int>(pixel[1])-100)<25 && std::abs(static_cast<int>(pixel[2])-200)<25;
    CVPixelBufferUnlockBaseAddress(decoded.value,kCVPixelBufferLock_ReadOnly);
    if(!matches) throw std::runtime_error("Decoded test frame colors differ");
    testMacMetalDecodedFrame(decoded.value);
  }
}

// One VideoToolbox session, two initial viewers and a third joining an idle
// desktop. Each decoder must receive the same freshly encoded keyframe.
void testMacBroadcastCodec() {
  @autoreleasepool {
    Options options; options.host=true; options.input=false; options.sharing=true; options.ultraLowLatency=true; options.width=640; options.height=360; options.bitrate=25000000;
    Broadcaster group(options);
    std::array<Decoder,3> decoders;
    std::array<std::unique_ptr<DecodeWorker>,3> workers;
    std::array<std::atomic<unsigned>,3> audioBlocks{};
    std::vector<std::unique_ptr<Peer>> clients;
    std::random_device random;
    auto join=[&](unsigned index) {
      for(unsigned attempt=0;attempt<20;++attempt) {
        const auto port=static_cast<std::uint16_t>(30000+random()%20000);
        const auto token=std::string(48,static_cast<char>('a'+index));
        Options client=options; client.host=false; client.sharing=false; client.port=port+1; client.peer="127.0.0.1:"+std::to_string(port); client.token=token;
        std::unique_ptr<Peer> receiver;
        try { receiver=std::make_unique<Peer>(client); group.add({std::to_string(index),"127.0.0.1:"+std::to_string(port+1),port,token,options.bitrate}); }
        catch(...) { continue; }
        workers[index]=std::make_unique<DecodeWorker>([&,index](Frame frame) { decoders[index].decode(frame); },
          [&,index] { decoders[index].reset(); },[p=receiver.get()] { p->requestKeyframe(); },true,true);
        receiver->onFrame=[&,index](Frame frame) { workers[index]->submit(std::move(frame)); };
        receiver->onAudio=[&,index](AudioSamples) { ++audioBlocks[index]; };
        receiver->start(); clients.push_back(std::move(receiver)); return;
      }
      throw std::runtime_error("Broadcast codec test could not bind sockets");
    };
    std::atomic<unsigned> encoded{0};
    Encoder encoder(options,[&] { return group.ready(); },[&](Frame frame) { ++encoded; group.publish(std::move(frame)); });
    struct Stop {
      Broadcaster& group; std::vector<std::unique_ptr<Peer>>& clients; std::array<std::unique_ptr<DecodeWorker>,3>& workers;
      ~Stop() { for(auto& client:clients) client->stop(); for(auto& worker:workers) if(worker) worker->stop(); group.stop(); }
    } stop{group,clients,workers};
    struct Pixel { CVPixelBufferRef value=nullptr; ~Pixel() { if(value) CVPixelBufferRelease(value); } } source;
    NSDictionary* attributes=@{(__bridge NSString*)kCVPixelBufferIOSurfacePropertiesKey:@{}};
    requireStatus(CVPixelBufferCreate(nullptr,640,360,kCVPixelFormatType_32BGRA,(__bridge CFDictionaryRef)attributes,&source.value),"Create sharing test frame");
    CVPixelBufferLockBaseAddress(source.value,0);
    auto* bytes=static_cast<std::uint8_t*>(CVPixelBufferGetBaseAddress(source.value));
    const auto stride=CVPixelBufferGetBytesPerRow(source.value);
    for(unsigned y=0;y<360;++y) for(unsigned x=0;x<640;++x) { auto* pixel=bytes+y*stride+x*4; pixel[0]=30; pixel[1]=100; pixel[2]=200; pixel[3]=255; }
    CVPixelBufferUnlockBaseAddress(source.value,0);
    // Cache before a viewer is ready, then refresh without a new capture.
    encoder.encode(source.value,CMTimeMake(1,60));
    auto waitReady=[&](std::size_t count) {
      const auto deadline=Clock::now()+std::chrono::seconds(3);
      while(Clock::now()<deadline) {
        const auto peers=group.stats();
        if(peers.size()==count && std::all_of(peers.begin(),peers.end(),[](const auto& peer) { return peer.ready; })) return;
        std::this_thread::sleep_for(std::chrono::milliseconds(2));
      }
      throw std::runtime_error("Broadcast codec handshake timed out");
    };
    auto verify=[&](unsigned index) {
      Pixel decoded;
      const auto deadline=Clock::now()+std::chrono::seconds(3);
      while(!decoded.value && Clock::now()<deadline) { decoded.value=decoders[index].take(); std::this_thread::sleep_for(std::chrono::milliseconds(2)); }
      if(!decoded.value || encoder.failed() || CVPixelBufferGetWidth(decoded.value)!=640 || CVPixelBufferGetHeight(decoded.value)!=360)
        throw std::runtime_error("Sharing viewer did not decode the keyframe");
      CVPixelBufferLockBaseAddress(decoded.value,kCVPixelBufferLock_ReadOnly);
      const auto* pixel=static_cast<const std::uint8_t*>(CVPixelBufferGetBaseAddress(decoded.value));
      const bool matches=std::abs(static_cast<int>(pixel[0])-30)<25 && std::abs(static_cast<int>(pixel[1])-100)<25 && std::abs(static_cast<int>(pixel[2])-200)<25;
      CVPixelBufferUnlockBaseAddress(decoded.value,kCVPixelBufferLock_ReadOnly);
      if(!matches) throw std::runtime_error("Sharing decoded colors differ");
    };
    join(0); join(1); waitReady(2);
    encoder.keyframe(); encoder.refresh(); verify(0); verify(1);
    if(encoded!=1 || group.publishedFrames()!=1) throw std::runtime_error("Sharing encoded separately per viewer");
    join(2); waitReady(3);
    encoder.keyframe(); encoder.refresh(); verify(0); verify(1); verify(2);
    if(encoded!=2 || group.publishedFrames()!=2) throw std::runtime_error("Late join encoded separately per viewer");
    group.remove("1"); clients[1]->stop(); workers[1]->stop();
    encoder.keyframe(); encoder.refresh(); verify(0); verify(2);
    if(encoded!=3) throw std::runtime_error("Removing a viewer restarted or duplicated encoding");
    // Receiving one IDR is not a streaming test. Change the actual source and
    // require both surviving viewers to decode each following P-picture.
    for(unsigned frame=0;frame<24;++frame) {
      const auto red=frame%2 ? 200 : 30,blue=frame%2 ? 30 : 200;
      CVPixelBufferLockBaseAddress(source.value,0);
      for(unsigned y=0;y<360;++y) for(unsigned x=0;x<640;++x) {
        auto* pixel=bytes+y*stride+x*4; pixel[0]=blue; pixel[2]=red;
      }
      CVPixelBufferUnlockBaseAddress(source.value,0);
      encoder.encode(source.value,CMTimeMake(frame+10,60));
      for(unsigned viewer : {0U,2U}) {
        Pixel decoded; const auto until=Clock::now()+std::chrono::seconds(2);
        while(!decoded.value && Clock::now()<until) { decoded.value=decoders[viewer].take(); std::this_thread::sleep_for(std::chrono::milliseconds(2)); }
        if(!decoded.value) throw std::runtime_error("Sharing stopped updating after its initial keyframe");
        CVPixelBufferLockBaseAddress(decoded.value,kCVPixelBufferLock_ReadOnly);
        const auto* pixel=static_cast<const std::uint8_t*>(CVPixelBufferGetBaseAddress(decoded.value));
        const bool changed=std::abs(static_cast<int>(pixel[0])-blue)<25 && std::abs(static_cast<int>(pixel[2])-red)<25;
        CVPixelBufferUnlockBaseAddress(decoded.value,kCVPixelBufferLock_ReadOnly);
        if(!changed) throw std::runtime_error("Sharing viewer replayed an old picture instead of the changed desktop");
      }
    }
    // Run beyond the old short test and the six/eight-second isolation windows.
    // Exercise the actual encoder, encrypted group transport and decode workers,
    // while checking changed pixels rather than only completed UDP packets.
    const auto began=Clock::now(); auto nextVideo=began,nextAudio=began;
    std::array<Clock::time_point,3> progress; progress.fill(began);
    std::array<int,3> colors{-1,-1,-1}; std::array<unsigned,3> changes{};
    AudioSamples sound; sound.fill(4000); unsigned frame=0;
    while(Clock::now()-began<std::chrono::seconds(12)) {
      const auto now=Clock::now(); group.updateHealth(now);
      if(now>=nextAudio) { group.audio(sound); nextAudio=now+std::chrono::milliseconds(5); }
      if(now>=nextVideo) {
        const auto blue=(frame/15)%2 ? 30 : 200,red=(frame/15)%2 ? 200 : 30;
        CVPixelBufferLockBaseAddress(source.value,0);
        for(unsigned y=0;y<360;++y) for(unsigned x=0;x<640;++x) { auto* pixel=bytes+y*stride+x*4; pixel[0]=blue; pixel[2]=red; }
        CVPixelBufferUnlockBaseAddress(source.value,0);
        if(group.takeKeyframeRequest() || frame%60==0) encoder.keyframe();
        encoder.encode(source.value,CMTimeMake(frame+100,60));
        encoder.flush(); // Test owns one mutable source buffer; finish before reuse.
        ++frame; nextVideo=now+std::chrono::microseconds(16667);
      }
      for(unsigned viewer : {0U,2U}) {
        Pixel image; image.value=decoders[viewer].take();
        if(image.value) {
          progress[viewer]=Clock::now(); CVPixelBufferLockBaseAddress(image.value,kCVPixelBufferLock_ReadOnly);
          const auto* pixel=static_cast<const std::uint8_t*>(CVPixelBufferGetBaseAddress(image.value));
          const int color=pixel[0]>pixel[2] ? 1 : 0;
          if(colors[viewer]>=0 && colors[viewer]!=color) ++changes[viewer]; colors[viewer]=color;
          CVPixelBufferUnlockBaseAddress(image.value,kCVPixelBufferLock_ReadOnly);
        }
        if(Clock::now()-progress[viewer]>std::chrono::seconds(2) || workers[viewer]->failures()>=10)
          throw std::runtime_error("Group viewer stopped presenting decoded pictures while source and audio continued");
      }
      std::this_thread::sleep_for(std::chrono::milliseconds(1));
    }
    for(unsigned viewer : {0U,2U}) if(changes[viewer]<4 || audioBlocks[viewer]<100)
      throw std::runtime_error("Group continuity must verify changing decoded pixels and live sound together");
    for(const auto& peer:group.stats()) if(peer.videoState!="active" || peer.pauses)
      throw std::runtime_error("Healthy codec viewers entered an unintended video pause");
    std::cout<<"Group codec: twelve seconds of changing pixels, Opus audio, decode workers and health updates passed\n";
  }
}

#endif
int runMac(bool host,int argc,char** argv) {
  @autoreleasepool {
    auto options=parseOptions(host,argc,argv); [NSApplication sharedApplication];
    if(host) {
      if(@available(macOS 12.3,*)) {
        // ScreenCaptureKit is the authority for this capture process. A stale
        // CoreGraphics preflight must not abort a newly granted capture or
        // trigger a second permission request before ScreenCaptureKit runs.
        __block SCShareableContent* content=nil;
        auto readContent=^(CaptureCompletion complete) {
          [SCShareableContent getShareableContentExcludingDesktopWindows:YES onScreenWindowsOnly:YES completionHandler:^(SCShareableContent* value,NSError* error) {
            dispatch_async(dispatch_get_main_queue(), ^{ content=value; complete(error); });
          }];
        };
        awaitCaptureOperation([readContent](CaptureCompletion complete) { readContent(complete); },"Read available screens");
        if(!content || content.displays.count==0) throw std::runtime_error("No display is available for capture");
        if(options.input && (!AXIsProcessTrusted() || !CGPreflightPostEventAccess())) { NSDictionary* request=@{(__bridge NSString*)kAXTrustedCheckOptionPrompt:@YES}; AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)request); throw std::runtime_error("On the host Mac, allow Accessibility for Sanser, then quit and reopen Sanser before reconnecting; or turn remote input off"); }
        SCDisplay* display=content.displays.firstObject; for(SCDisplay* candidate in content.displays) if(candidate.displayID==CGMainDisplayID()) display=candidate;
        const double scale=std::min(static_cast<double>(options.width)/display.width,static_cast<double>(options.height)/display.height);
        options.width=std::max(64U,static_cast<std::uint32_t>(display.width*scale)&~1U);
        options.height=std::max(64U,static_cast<std::uint32_t>(display.height*scale)&~1U);
        HostOutput peer(options); Encoder encoder(options,peer); MacInputInjector injector(CGDisplayBounds(display.displayID));
        VoiceRuntime voice(options.groupVoice,[&](voice::Packet packet) { peer.voicePacket(std::move(packet)); },[&](voice::State state) { peer.setVoiceState(state); });
        VoicePanel voicePanel(options.groupVoice,voice);
        peer.onVoice=[&](voice::Packet packet) { voice.receive(std::move(packet)); };
        AudioPacketizer audio([&](const AudioSamples& samples) { peer.audio(samples); });
        peer.onInput=[&](Input event) { injector.apply(event); };
        SanserCapture* capture=[SanserCapture new]; capture.encoder=&encoder;
        capture.audio=&audio;
        SCContentFilter* filter=[[SCContentFilter alloc] initWithDisplay:display excludingWindows:@[]];
        SCStreamConfiguration* config=[SCStreamConfiguration new]; config.width=options.width; config.height=options.height; config.minimumFrameInterval=CMTimeMake(1,options.fps); config.queueDepth=options.ultraLowLatency ? 2 : 3;
        // Control viewers already draw an immediate local cursor. Only a
        // view-only capture includes the host pointer in the encoded picture.
        config.showsCursor=options.sharing || !options.input; config.pixelFormat=kCVPixelFormatType_32BGRA;
        if(@available(macOS 13.0,*)) {
          config.capturesAudio=options.audio;
          config.sampleRate=audioRate; config.channelCount=audioChannels;
          config.excludesCurrentProcessAudio=YES;
        }
        SCStream* stream=[[SCStream alloc] initWithFilter:filter configuration:config delegate:capture];
        dispatch_queue_t queue=dispatch_queue_create("sanser.capture",DISPATCH_QUEUE_SERIAL);
        dispatch_queue_t audioQueue=dispatch_queue_create("sanser.audio.capture",DISPATCH_QUEUE_SERIAL);
        CaptureStop captureStop{stream,capture,queue,audioQueue};
        NSError* failure=nil; if(![stream addStreamOutput:capture type:SCStreamOutputTypeScreen sampleHandlerQueue:queue error:&failure]) throw std::runtime_error(failure ? captureFailure(failure,"Create screen capture output") : "Unable to create screen capture output");
        if(@available(macOS 13.0,*)) {
          if(config.capturesAudio && ![stream addStreamOutput:capture type:SCStreamOutputTypeAudio sampleHandlerQueue:audioQueue error:&failure])
            throw std::runtime_error(failure ? captureFailure(failure,"Create system audio output") : "Unable to create system audio output");
        }
        // Stream startup gets its own deadline, independent of time spent on
        // display enumeration or the user's permission dialog.
        awaitCaptureOperation([&](CaptureCompletion complete) {
          [stream startCaptureWithCompletionHandler:complete];
        },"Start screen capture");
        SharingNotice notice(peer,options.sharing);
        HostOutputStop shutdown{peer}; peer.start();
        auto bitrate=options.bitrate;
        auto lastProgress=Clock::now();
        while(peer.running() && !capture.failed && !encoder.failed()) {
          peer.pumpControl();
          notice.pump(); if(!peer.running()) break;
          const auto target=peer.bitrate();
          if(target!=bitrate && encoder.setBitrate(target)) bitrate=target;
          const bool retry=encoder.takeRetryRequest();
          if(peer.takeKeyframeRequest() || retry) { encoder.keyframe(); encoder.refresh(); }
          if(Clock::now()-lastProgress>=std::chrono::seconds(1)) { encoder.logProgress(); lastProgress=Clock::now(); }
          [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
        }
        peer.stop(); return capture.failed || encoder.failed() ? 1 : 0;
      } else throw std::runtime_error("Hosting requires macOS 12.3 or newer");
    }
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    Peer peer(options); Decoder decoder;
    VoiceRuntime voice(options.groupVoice,[&](voice::Packet packet) { peer.voicePacket(std::move(packet)); },[&](voice::State state) { peer.setVoiceState(state); });
    VoicePanel voicePanel(options.groupVoice,voice);
    peer.onVoice=[&](voice::Packet packet) { voice.receive(std::move(packet)); };
    AudioPlayer audio(options.audio);
    if(options.audio) peer.onAudio=[&](AudioSamples samples) { audio.submit(std::move(samples)); };
    if(options.audio) peer.onAudioReset=[&] { audio.reset(); };
    DecodeWorker decode([&](Frame frame) { @autoreleasepool { decoder.decode(frame); } },
      [&] { decoder.reset(); },[&] { peer.requestKeyframe(); },true,options.ultraLowLatency);
    peer.onFrame=[&](Frame frame) { decode.submit(std::move(frame)); };
    NSWindow* window=[[NSWindow alloc] initWithContentRect:NSMakeRect(0,0,1280,720) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskResizable|NSWindowStyleMaskMiniaturizable backing:NSBackingStoreBuffered defer:NO];
    window.releasedWhenClosed=NO; window.title=options.input ? @"Sanser · Remote desktop · Control+Option+Escape releases input" : @"Sanser · View only";
    SanserSnv2RemoteView* view=[[SanserSnv2RemoteView alloc] initWithFrame:window.contentView.bounds]; view.peer=&peer; view.inputEnabled=options.input; view.autoresizingMask=NSViewWidthSizable|NSViewHeightSizable;
    if (options.ultraLowLatency) { ((CAMetalLayer*)view.layer).displaySyncEnabled=NO; }
    window.contentView=view; window.acceptsMouseMovedEvents=options.input; [window center]; [window makeKeyAndOrderFront:nil]; [window makeFirstResponder:view]; [NSApp activateIgnoringOtherApps:YES];
    Peer* peerPointer=&peer;
    id observer=[[NSNotificationCenter defaultCenter] addObserverForName:NSWindowDidResignKeyNotification object:window queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification*) { if(view.inputEnabled) { peerPointer->input(Input{}); view.released=YES; } }];
    __block bool userClosed=false;
    id closeObserver=[[NSNotificationCenter defaultCenter] addObserverForName:NSWindowWillCloseNotification object:window queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification*) { userClosed=true; }];
    PeerStop shutdown{peer};
    peer.start();
    auto lastHealth=Clock::now(); std::uint64_t presented=0;
    while(window.visible && peer.running() && decode.failures()<10) {
      @autoreleasepool {
        if(options.input && view.inputEnabled!=peer.inputAvailable()) {
          view.inputEnabled=peer.inputAvailable(); view.released=YES;
          window.title=view.inputEnabled ? @"Sanser · Input restored · Click to resume" : peer.capturePaused() ? @"Sanser · Host capture temporarily unavailable · Check UAC on the host" : @"Sanser · Input paused · Waiting for authenticated reset; reconnect if it persists";
        }
        NSEvent* event=[NSApp nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate dateWithTimeIntervalSinceNow:options.ultraLowLatency ? 0.001 : 0.008] inMode:NSDefaultRunLoopMode dequeue:YES]; if(event) [NSApp sendEvent:event];
        auto pixels=decoder.take(); if(pixels) { [view presentPixels:pixels]; ++presented; CVPixelBufferRelease(pixels); }
        [window displayIfNeeded];
        if(std::getenv("SANSER_TELEMETRY") && Clock::now()-lastHealth>=std::chrono::seconds(1)) {
          std::ostringstream line; line<<"SNV2_VIDEO_HEALTH decoded="<<decoder.decoded()<<" converted="<<presented
            <<" decoderOutputErrors="<<decoder.outputErrors()<<" decoderOutputDrops="<<decoder.outputDrops()
            <<" encodedQueueDrops="<<decode.dropped()<<" decodeQueueExpired="<<decode.expired()<<" decodeErrors="<<decode.failures();
          const auto timing=decode.takeTiming();
          if(timing.samples) line<<" decodeWorkAvgMs="<<timing.workUs/1000.0/timing.samples
            <<" decodeQueueAvgMs="<<timing.queueUs/1000.0/timing.samples<<" decodeQueueMaxMs="<<timing.queueMaxUs/1000.0;
          writeTelemetry(line);
          if(options.audio) writeAudioOutputHealth(audio.stats());
          lastHealth=Clock::now();
        }
      }
    }
    [[NSNotificationCenter defaultCenter] removeObserver:observer];
    [[NSNotificationCenter defaultCenter] removeObserver:closeObserver];
    if(userClosed) std::cerr<<"SNV2_VIEWER_CLOSED"<<std::endl;
    view.peer=nullptr; peer.stop(); decode.stop(); [window close];
    if(decode.failures()>=10) throw std::runtime_error("Unable to decode the remote video: "+decode.error());
    return 0;
  }
}
}
