#import <Cocoa/Cocoa.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <VideoToolbox/VideoToolbox.h>
#import <CoreImage/CoreImage.h>
#import <ApplicationServices/ApplicationServices.h>
#include "transport.h"
#include "broadcast.h"
#include "engine.h"
#include "capture_startup.h"
#import "metal_presenter.h"
#include "keyboard.h"
#include <algorithm>
#include <iostream>
#include <memory>
#include <set>
#include <random>

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
class Encoder {
public:
  Encoder(const Options& options,Peer& peer) : Encoder(options,[&peer] { return peer.ready(); },[&peer](Frame frame) { peer.video(frame); }) {}
  Encoder(const Options& options,HostOutput& output) : Encoder(options,[&output] { return output.ready(); },[&output](Frame frame) { output.video(std::move(frame)); }) {}
  Encoder(const Options& options,std::function<bool()> ready,std::function<void(Frame)> publish)
      : options_(options),ready_(std::move(ready)),publish_(std::move(publish)) {
    requireStatus(VTCompressionSessionCreate(nullptr,options.width,options.height,kCMVideoCodecType_H264,nullptr,nullptr,nullptr,output,this,&session_),"Create H.264 encoder");
    VTSessionSetProperty(session_,kVTCompressionPropertyKey_RealTime,kCFBooleanTrue);
    VTSessionSetProperty(session_,kVTCompressionPropertyKey_AllowFrameReordering,kCFBooleanFalse);
    VTSessionSetProperty(session_,kVTCompressionPropertyKey_ProfileLevel,kVTProfileLevel_H264_Baseline_AutoLevel);
    NSDictionary* values=@{(__bridge NSString*)kVTCompressionPropertyKey_AverageBitRate:@(options.bitrate),(__bridge NSString*)kVTCompressionPropertyKey_ExpectedFrameRate:@(options.fps),(__bridge NSString*)kVTCompressionPropertyKey_MaxKeyFrameInterval:@(options.fps)};
    requireStatus(VTSessionSetProperties(session_,(__bridge CFDictionaryRef)values),"Configure H.264 encoder");
    requireStatus(VTCompressionSessionPrepareToEncodeFrames(session_),"Prepare encoder");
  }
  ~Encoder() { if(session_) { VTCompressionSessionCompleteFrames(session_,kCMTimeInvalid); VTCompressionSessionInvalidate(session_); CFRelease(session_); } if(lastPixels_) CVPixelBufferRelease(lastPixels_); }
  void encode(CVPixelBufferRef pixels,CMTime time) {
    std::lock_guard lock(encodeMutex_);
    if(lastPixels_) CVPixelBufferRelease(lastPixels_);
    lastPixels_=CVPixelBufferRetain(pixels);
    if(CMTimeCompare(time,lastTime_)<=0) time=CMTimeAdd(lastTime_,CMTimeMake(1,options_.fps));
    lastTime_=time;
    if(ready_()) submit(pixels,time);
  }
  void refresh() {
    std::lock_guard lock(encodeMutex_);
    if(!lastPixels_ || !ready_()) return;
    lastTime_=CMTimeAdd(lastTime_,CMTimeMake(1,options_.fps));
    submit(lastPixels_,lastTime_);
  }
  void submit(CVPixelBufferRef pixels,CMTime time) {
    const bool force=force_.exchange(false);
    const auto properties=force ? (__bridge CFDictionaryRef)@{(__bridge NSString*)kVTEncodeFrameOptionKey_ForceKeyFrame:@YES} : nullptr;
    if(VTCompressionSessionEncodeFrame(session_,pixels,time,CMTimeMake(1,options_.fps),properties,nullptr,nullptr)!=noErr) failed_=true;
  }
  void keyframe() { force_=true; }
  bool setBitrate(std::uint32_t bitrate) {
    std::lock_guard lock(encodeMutex_);
    return VTSessionSetProperty(session_,kVTCompressionPropertyKey_AverageBitRate,(__bridge CFNumberRef)@(bitrate))==noErr;
  }
  bool failed() const { return failed_; }
private:
  static void output(void* context,void*,OSStatus status,VTEncodeInfoFlags,CMSampleBufferRef sample) {
    auto& self=*static_cast<Encoder*>(context);
    if(status!=noErr || !sample || !CMSampleBufferDataIsReady(sample)) { self.failed_=true; return; }
    try {
      Frame frame; frame.width=self.options_.width; frame.height=self.options_.height;
      const auto attachments=CMSampleBufferGetSampleAttachmentsArray(sample,false);
      frame.keyframe=!attachments || CFArrayGetCount(attachments)==0 || !CFDictionaryContainsKey(static_cast<CFDictionaryRef>(CFArrayGetValueAtIndex(attachments,0)),kCMSampleAttachmentKey_NotSync);
      const auto format=CMSampleBufferGetFormatDescription(sample);
      if(frame.keyframe) {
        for(std::size_t index=0;index<2;++index) { const std::uint8_t* data=nullptr; std::size_t size=0,count=0; int header=0;
          if(CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format,index,&data,&size,&count,&header)!=noErr) { self.failed_=true; return; } appendNal(frame.data,data,size);
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
  std::atomic<bool> force_{true},failed_{false};
};
class Decoder {
public:
  ~Decoder() { clear(); std::lock_guard lock(mutex_); if(latest_) CVPixelBufferRelease(latest_); }
  void decode(const Frame& frame) {
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
  }
  CVPixelBufferRef take() { std::lock_guard lock(mutex_); auto value=latest_; latest_=nullptr; return value; }
private:
  void clear() { if(session_) { VTDecompressionSessionWaitForAsynchronousFrames(session_); VTDecompressionSessionInvalidate(session_); CFRelease(session_); session_=nullptr; } if(format_) { CFRelease(format_); format_=nullptr; } }
  static void output(void* context,void*,OSStatus status,VTDecodeInfoFlags,CVImageBufferRef image,CMTime,CMTime) { if(status!=noErr || !image) return; auto& self=*static_cast<Decoder*>(context); std::lock_guard lock(self.mutex_); if(self.latest_) CVPixelBufferRelease(self.latest_); self.latest_=CVPixelBufferRetain(image); if(!self.reported_) { self.reported_=true; std::cout<<"SNV2_FIRST_FRAME decoded=1"<<std::endl; } }
  Bytes sps_,pps_; VTDecompressionSessionRef session_=nullptr; CMVideoFormatDescriptionRef format_=nullptr; CVPixelBufferRef latest_=nullptr; std::mutex mutex_; bool reported_=false;
};
class Injector {
public:
  explicit Injector(CGDirectDisplayID display) : bounds_(CGDisplayBounds(display)) {}
  ~Injector() { reset(); }
  void apply(const Input& input) {
    if(input.kind==Input::Reset) { reset(); return; }
    const CGPoint point=CGPointMake(bounds_.origin.x+input.x/65535.0*bounds_.size.width,bounds_.origin.y+input.y/65535.0*bounds_.size.height);
    CGEventRef event=nullptr;
    if(input.kind==Input::Key) {
      const auto key=macKey(input.code); if(key<0) return;
      if(input.down) pressed_.insert(input.code); else pressed_.erase(input.code);
      event=CGEventCreateKeyboardEvent(nullptr,static_cast<CGKeyCode>(key),input.down);
      CGEventFlags flags=0;
      if(pressed_.contains(224)||pressed_.contains(228)) flags|=kCGEventFlagMaskControl;
      if(pressed_.contains(225)||pressed_.contains(229)) flags|=kCGEventFlagMaskShift;
      if(pressed_.contains(226)||pressed_.contains(230)) flags|=kCGEventFlagMaskAlternate;
      if(pressed_.contains(227)||pressed_.contains(231)) flags|=kCGEventFlagMaskCommand;
      if(event) CGEventSetFlags(event,flags);
    } else if(input.kind==Input::Scroll) event=CGEventCreateScrollWheelEvent(nullptr,kCGScrollEventUnitLine,1,static_cast<int>(input.delta));
    else {
      CGEventType type=kCGEventMouseMoved; CGMouseButton button=kCGMouseButtonLeft;
      if(input.kind==Input::Button) { button=static_cast<CGMouseButton>(input.code); if(input.down) buttons_.insert(input.code); else buttons_.erase(input.code); type=input.code==0 ? (input.down?kCGEventLeftMouseDown:kCGEventLeftMouseUp) : input.code==1 ? (input.down?kCGEventRightMouseDown:kCGEventRightMouseUp) : (input.down?kCGEventOtherMouseDown:kCGEventOtherMouseUp); }
      else if(!buttons_.empty()) { button=static_cast<CGMouseButton>(*buttons_.begin()); type=button==kCGMouseButtonLeft?kCGEventLeftMouseDragged:button==kCGMouseButtonRight?kCGEventRightMouseDragged:kCGEventOtherMouseDragged; }
      lastPoint_=point; event=CGEventCreateMouseEvent(nullptr,type,point,button);
    }
    if(event) { CGEventPost(kCGHIDEventTap,event); CFRelease(event); }
  }
private:
  void reset() {
    for(auto code:pressed_) { const auto key=macKey(code); if(key>=0) { auto event=CGEventCreateKeyboardEvent(nullptr,static_cast<CGKeyCode>(key),false); if(event) { CGEventPost(kCGHIDEventTap,event); CFRelease(event); } } } pressed_.clear();
    for(auto button:buttons_) { auto event=CGEventCreateMouseEvent(nullptr,button==0?kCGEventLeftMouseUp:button==1?kCGEventRightMouseUp:kCGEventOtherMouseUp,lastPoint_,static_cast<CGMouseButton>(button)); if(event) { CGEventPost(kCGHIDEventTap,event); CFRelease(event); } } buttons_.clear();
  }
  CGRect bounds_; CGPoint lastPoint_{}; std::set<std::uint16_t> pressed_,buttons_;
};
}

API_AVAILABLE(macos(12.3))
@interface SanserCapture : NSObject <SCStreamOutput,SCStreamDelegate>
@property(nonatomic,assign) Encoder* encoder;
@property(atomic,assign) BOOL failed;
@end
@implementation SanserCapture
- (void)stream:(SCStream*)stream didOutputSampleBuffer:(CMSampleBufferRef)sample ofType:(SCStreamOutputType)type {
  (void)stream; if(type!=SCStreamOutputTypeScreen || !CMSampleBufferIsValid(sample) || !self.encoder) return;
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

@interface SanserRemoteView : SanserMetalView
@property(nonatomic,assign) Peer* peer;
@property(nonatomic,assign) BOOL released;
- (void)pointer:(NSEvent*)event kind:(Input::Kind)kind down:(BOOL)down;
@end
@implementation SanserRemoteView
- (BOOL)acceptsFirstResponder { return YES; }
- (void)pointer:(NSEvent*)event kind:(Input::Kind)kind down:(BOOL)down {
  if(!self.peer || self.released || self.imageSize.width<=0 || self.imageSize.height<=0) return;
  auto point=[self convertPoint:event.locationInWindow fromView:nil]; const auto bounds=self.bounds;
  const auto size=self.imageSize; const auto scale=std::min(bounds.size.width/size.width,bounds.size.height/size.height);
  const auto width=size.width*scale,height=size.height*scale; point.x-=(bounds.size.width-width)/2; point.y-=(bounds.size.height-height)/2;
  Input input; input.kind=kind; input.x=static_cast<std::uint16_t>(std::clamp(point.x/width,0.0,1.0)*65535); input.y=static_cast<std::uint16_t>((1-std::clamp(point.y/height,0.0,1.0))*65535); input.down=down; input.code=static_cast<std::uint16_t>(std::min<NSInteger>(event.buttonNumber,2));
  input.delta=static_cast<std::int16_t>(std::clamp(event.scrollingDeltaY,-120.0,120.0)); self.peer->input(input);
}
- (void)mouseMoved:(NSEvent*)event { [self pointer:event kind:Input::Move down:NO]; }
- (void)mouseDragged:(NSEvent*)event { [self pointer:event kind:Input::Move down:NO]; }
- (void)rightMouseDragged:(NSEvent*)event { [self pointer:event kind:Input::Move down:NO]; }
- (void)mouseDown:(NSEvent*)event { if(self.released) { self.released=NO; self.window.title=@"Sanser · Remote desktop · Control+Option+Escape releases input"; return; } [self pointer:event kind:Input::Button down:YES]; }
- (void)mouseUp:(NSEvent*)event { [self pointer:event kind:Input::Button down:NO]; }
- (void)rightMouseDown:(NSEvent*)event { [self pointer:event kind:Input::Button down:YES]; }
- (void)rightMouseUp:(NSEvent*)event { [self pointer:event kind:Input::Button down:NO]; }
- (void)otherMouseDown:(NSEvent*)event { [self pointer:event kind:Input::Button down:YES]; }
- (void)otherMouseUp:(NSEvent*)event { [self pointer:event kind:Input::Button down:NO]; }
- (void)scrollWheel:(NSEvent*)event { [self pointer:event kind:Input::Scroll down:NO]; }
- (void)keyDown:(NSEvent*)event {
  if(event.keyCode==53 && (event.modifierFlags & NSEventModifierFlagControl) && (event.modifierFlags & NSEventModifierFlagOption)) { self.released=YES; if(self.peer) self.peer->input(Input{}); self.window.title=@"Sanser · Input released · Click the stream to resume"; return; }
  if(self.released || !self.peer) return; const auto hid=hidFromMac(event.keyCode); if(hid) self.peer->input(Input{Input::Key,0,0,hid,0,true});
}
- (void)keyUp:(NSEvent*)event { if(!self.released && self.peer) { const auto hid=hidFromMac(event.keyCode); if(hid) self.peer->input(Input{Input::Key,0,0,hid,0,false}); } }
- (void)flagsChanged:(NSEvent*)event {
  if(self.released || !self.peer) return; const auto hid=hidFromMac(event.keyCode); NSEventModifierFlags flag=0;
  if(hid==224||hid==228) flag=NSEventModifierFlagControl; if(hid==225||hid==229) flag=NSEventModifierFlagShift; if(hid==226||hid==230) flag=NSEventModifierFlagOption; if(hid==227||hid==231) flag=NSEventModifierFlagCommand;
  if(flag) self.peer->input(Input{Input::Key,0,0,hid,0,(event.modifierFlags&flag)!=0});
}
@end

namespace sanser::desktop {
bool macHostAvailable() { if (@available(macOS 12.3, *)) return true; return false; }
struct API_AVAILABLE(macos(12.3)) CaptureStop {
  SCStream* __strong stream;
  SanserCapture* __strong capture;
  dispatch_queue_t queue;
  ~CaptureStop() {
    dispatch_sync(queue, ^{ capture.encoder=nullptr; });
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
    for(unsigned y=0;y<360;++y) for(unsigned x=0;x<640;++x) { auto* pixel=bytes+y*stride+x*4; pixel[0]=30; pixel[1]=100; pixel[2]=200; pixel[3]=255; }
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
  }
}

// One VideoToolbox session, two initial viewers and a third joining an idle
// desktop. Each decoder must receive the same freshly encoded keyframe.
void testMacBroadcastCodec() {
  @autoreleasepool {
    Options options; options.host=true; options.input=false; options.width=640; options.height=360; options.bitrate=100000000;
    Broadcaster group(options);
    std::array<Decoder,3> decoders;
    std::vector<std::unique_ptr<Peer>> clients;
    std::random_device random;
    auto join=[&](unsigned index) {
      for(unsigned attempt=0;attempt<20;++attempt) {
        const auto port=static_cast<std::uint16_t>(30000+random()%20000);
        const auto token=std::string(48,static_cast<char>('a'+index));
        Options client=options; client.host=false; client.port=port+1; client.peer="127.0.0.1:"+std::to_string(port); client.token=token;
        std::unique_ptr<Peer> receiver;
        try { receiver=std::make_unique<Peer>(client); group.add({std::to_string(index),"127.0.0.1:"+std::to_string(port+1),port,token,options.bitrate}); }
        catch(...) { continue; }
        receiver->onFrame=[&,index](Frame frame) { decoders[index].decode(frame); };
        receiver->start(); clients.push_back(std::move(receiver)); return;
      }
      throw std::runtime_error("Broadcast codec test could not bind sockets");
    };
    std::atomic<unsigned> encoded{0};
    Encoder encoder(options,[&] { return group.ready(); },[&](Frame frame) { ++encoded; group.publish(std::move(frame)); });
    struct Stop {
      Broadcaster& group; std::vector<std::unique_ptr<Peer>>& clients;
      ~Stop() { for(auto& client:clients) client->stop(); group.stop(); }
    } stop{group,clients};
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
    group.remove("1"); clients[1]->stop();
    encoder.keyframe(); encoder.refresh(); verify(0); verify(2);
    if(encoded!=3) throw std::runtime_error("Removing a viewer restarted or duplicated encoding");
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
        if(options.input && !AXIsProcessTrusted()) { NSDictionary* request=@{(__bridge NSString*)kAXTrustedCheckOptionPrompt:@YES}; AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)request); throw std::runtime_error("On the host Mac, allow Accessibility for Sanser, then quit and reopen Sanser before reconnecting; or turn remote input off"); }
        SCDisplay* display=content.displays.firstObject; for(SCDisplay* candidate in content.displays) if(candidate.displayID==CGMainDisplayID()) display=candidate;
        const double scale=std::min(static_cast<double>(options.width)/display.width,static_cast<double>(options.height)/display.height);
        options.width=std::max(64U,static_cast<std::uint32_t>(display.width*scale)&~1U);
        options.height=std::max(64U,static_cast<std::uint32_t>(display.height*scale)&~1U);
        HostOutput peer(options); Encoder encoder(options,peer); Injector injector(display.displayID);
        peer.onInput=[&](Input event) { injector.apply(event); };
        SanserCapture* capture=[SanserCapture new]; capture.encoder=&encoder;
        SCContentFilter* filter=[[SCContentFilter alloc] initWithDisplay:display excludingWindows:@[]];
        SCStreamConfiguration* config=[SCStreamConfiguration new]; config.width=options.width; config.height=options.height; config.minimumFrameInterval=CMTimeMake(1,options.fps); config.queueDepth=options.ultraLowLatency ? 2 : 3; config.showsCursor=options.sharing ? NO : YES; config.pixelFormat=kCVPixelFormatType_32BGRA;
        SCStream* stream=[[SCStream alloc] initWithFilter:filter configuration:config delegate:capture];
        dispatch_queue_t queue=dispatch_queue_create("sanser.capture",DISPATCH_QUEUE_SERIAL);
        CaptureStop captureStop{stream,capture,queue};
        NSError* failure=nil; if(![stream addStreamOutput:capture type:SCStreamOutputTypeScreen sampleHandlerQueue:queue error:&failure]) throw std::runtime_error(failure ? captureFailure(failure,"Create screen capture output") : "Unable to create screen capture output");
        // Stream startup gets its own deadline, independent of time spent on
        // display enumeration or the user's permission dialog.
        awaitCaptureOperation([&](CaptureCompletion complete) {
          [stream startCaptureWithCompletionHandler:complete];
        },"Start screen capture");
        SharingNotice notice(peer,options.sharing);
        HostOutputStop shutdown{peer}; peer.start();
        auto bitrate=options.bitrate;
        while(peer.running() && !capture.failed && !encoder.failed()) {
          peer.pumpControl();
          notice.pump(); if(!peer.running()) break;
          const auto target=peer.bitrate();
          if(target!=bitrate && encoder.setBitrate(target)) bitrate=target;
          if(peer.takeKeyframeRequest()) { encoder.keyframe(); encoder.refresh(); }
          [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
        }
        peer.stop(); return capture.failed || encoder.failed() ? 1 : 0;
      } else throw std::runtime_error("Hosting requires macOS 12.3 or newer");
    }
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    Peer peer(options); Decoder decoder; std::atomic<unsigned> decodeFailures{0}; std::string decodeError;
    peer.onFrame=[&](Frame frame) { try { decoder.decode(frame); decodeFailures=0; } catch(const std::exception& error) { decodeError=error.what(); ++decodeFailures; peer.requestKeyframe(); } };
    NSWindow* window=[[NSWindow alloc] initWithContentRect:NSMakeRect(0,0,1280,720) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskResizable|NSWindowStyleMaskMiniaturizable backing:NSBackingStoreBuffered defer:NO];
    window.releasedWhenClosed=NO; window.title=options.input ? @"Sanser · Remote desktop · Control+Option+Escape releases input" : @"Sanser · View only";
    SanserRemoteView* view=[[SanserRemoteView alloc] initWithFrame:window.contentView.bounds]; view.peer=&peer; view.autoresizingMask=NSViewWidthSizable|NSViewHeightSizable;
    if (options.ultraLowLatency) { ((CAMetalLayer*)view.layer).displaySyncEnabled=NO; }
    window.contentView=view; window.acceptsMouseMovedEvents=YES; [window center]; [window makeKeyAndOrderFront:nil]; [window makeFirstResponder:view]; [NSApp activateIgnoringOtherApps:YES];
    Peer* peerPointer=&peer;
    id observer=[[NSNotificationCenter defaultCenter] addObserverForName:NSWindowDidResignKeyNotification object:window queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification*) { peerPointer->input(Input{}); view.released=YES; }];
    PeerStop shutdown{peer};
    peer.start();
    while(window.visible && peer.running() && decodeFailures<10) {
      @autoreleasepool {
        NSEvent* event=[NSApp nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate dateWithTimeIntervalSinceNow:options.ultraLowLatency ? 0.001 : 0.008] inMode:NSDefaultRunLoopMode dequeue:YES]; if(event) [NSApp sendEvent:event];
        auto pixels=decoder.take(); if(pixels) { [view presentPixels:pixels]; CVPixelBufferRelease(pixels); }
        [window displayIfNeeded];
      }
    }
    [[NSNotificationCenter defaultCenter] removeObserver:observer]; view.peer=nullptr; peer.stop(); [window close];
    if(decodeFailures>=10) throw std::runtime_error("Unable to decode the remote video: "+decodeError);
    return 0;
  }
}
}
