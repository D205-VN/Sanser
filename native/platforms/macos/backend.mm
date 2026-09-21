#import <Cocoa/Cocoa.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <VideoToolbox/VideoToolbox.h>
#import <CoreImage/CoreImage.h>
#import <ApplicationServices/ApplicationServices.h>
#include "transport.h"
#include "engine.h"
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
  Encoder(const Options& options,Peer& peer) : options_(options),peer_(peer) {
    requireStatus(VTCompressionSessionCreate(nullptr,options.width,options.height,kCMVideoCodecType_H264,nullptr,nullptr,nullptr,output,this,&session_),"Create H.264 encoder");
    VTSessionSetProperty(session_,kVTCompressionPropertyKey_RealTime,kCFBooleanTrue);
    VTSessionSetProperty(session_,kVTCompressionPropertyKey_AllowFrameReordering,kCFBooleanFalse);
    VTSessionSetProperty(session_,kVTCompressionPropertyKey_ProfileLevel,kVTProfileLevel_H264_Baseline_AutoLevel);
    NSDictionary* values=@{(__bridge NSString*)kVTCompressionPropertyKey_AverageBitRate:@(options.bitrate),(__bridge NSString*)kVTCompressionPropertyKey_ExpectedFrameRate:@(options.fps),(__bridge NSString*)kVTCompressionPropertyKey_MaxKeyFrameInterval:@(options.fps)};
    requireStatus(VTSessionSetProperties(session_,(__bridge CFDictionaryRef)values),"Configure H.264 encoder");
    requireStatus(VTCompressionSessionPrepareToEncodeFrames(session_),"Prepare encoder");
  }
  ~Encoder() { if(session_) { VTCompressionSessionCompleteFrames(session_,kCMTimeInvalid); VTCompressionSessionInvalidate(session_); CFRelease(session_); } }
  void encode(CVPixelBufferRef pixels,CMTime time) {
    if(!peer_.ready()) return;
    const bool force=force_.exchange(false);
    const auto properties=force ? (__bridge CFDictionaryRef)@{(__bridge NSString*)kVTEncodeFrameOptionKey_ForceKeyFrame:@YES} : nullptr;
    if(VTCompressionSessionEncodeFrame(session_,pixels,time,CMTimeMake(1,options_.fps),properties,nullptr,nullptr)!=noErr) failed_=true;
  }
  void keyframe() { force_=true; }
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
      self.peer_.video(frame);
    } catch(...) { self.failed_=true; }
  }
  Options options_; Peer& peer_; VTCompressionSessionRef session_=nullptr;
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
  static void output(void* context,void*,OSStatus status,VTDecodeInfoFlags,CVImageBufferRef image,CMTime,CMTime) { if(status!=noErr || !image) return; auto& self=*static_cast<Decoder*>(context); std::lock_guard lock(self.mutex_); if(self.latest_) CVPixelBufferRelease(self.latest_); self.latest_=CVPixelBufferRetain(image); }
  Bytes sps_,pps_; VTDecompressionSessionRef session_=nullptr; CMVideoFormatDescriptionRef format_=nullptr; CVPixelBufferRef latest_=nullptr; std::mutex mutex_;
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
- (void)stream:(SCStream*)stream didStopWithError:(NSError*)error { (void)stream; (void)error; self.failed=YES; }
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

#endif
int runMac(bool host,int argc,char** argv) {
  @autoreleasepool {
    auto options=parseOptions(host,argc,argv); [NSApplication sharedApplication];
    if(host) {
      if(@available(macOS 12.3,*)) {
        if(!CGPreflightScreenCaptureAccess()) { CGRequestScreenCaptureAccess(); throw std::runtime_error("Allow Screen Recording for Sanser in System Settings, then retry"); }
        if(options.input && !AXIsProcessTrusted()) { NSDictionary* request=@{(__bridge NSString*)kAXTrustedCheckOptionPrompt:@YES}; AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)request); throw std::runtime_error("Allow Accessibility for Sanser to enable remote input, or turn remote input off"); }
        __block SCShareableContent* content=nil; __block bool finished=false;
        [SCShareableContent getShareableContentExcludingDesktopWindows:YES onScreenWindowsOnly:YES completionHandler:^(SCShareableContent* value,NSError*) { dispatch_async(dispatch_get_main_queue(), ^{ content=value; finished=true; }); }];
        const auto deadline=Clock::now()+std::chrono::seconds(10);
        while(!finished && Clock::now()<deadline) [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
        if(!content || content.displays.count==0) throw std::runtime_error("No display is available for capture");
        SCDisplay* display=content.displays.firstObject; for(SCDisplay* candidate in content.displays) if(candidate.displayID==CGMainDisplayID()) display=candidate;
        const double scale=std::min(static_cast<double>(options.width)/display.width,static_cast<double>(options.height)/display.height);
        options.width=std::max(64U,static_cast<std::uint32_t>(display.width*scale)&~1U);
        options.height=std::max(64U,static_cast<std::uint32_t>(display.height*scale)&~1U);
        Peer peer(options); Encoder encoder(options,peer); Injector injector(display.displayID);
        peer.onInput=[&](Input event) { injector.apply(event); }; peer.onKeyframe=[&] { encoder.keyframe(); };
        SanserCapture* capture=[SanserCapture new]; capture.encoder=&encoder;
        SCContentFilter* filter=[[SCContentFilter alloc] initWithDisplay:display excludingWindows:@[]];
        SCStreamConfiguration* config=[SCStreamConfiguration new]; config.width=options.width; config.height=options.height; config.minimumFrameInterval=CMTimeMake(1,options.fps); config.queueDepth=3; config.showsCursor=YES; config.pixelFormat=kCVPixelFormatType_32BGRA;
        SCStream* stream=[[SCStream alloc] initWithFilter:filter configuration:config delegate:capture];
        dispatch_queue_t queue=dispatch_queue_create("sanser.capture",DISPATCH_QUEUE_SERIAL);
        CaptureStop captureStop{stream,capture,queue};
        NSError* failure=nil; if(![stream addStreamOutput:capture type:SCStreamOutputTypeScreen sampleHandlerQueue:queue error:&failure]) throw std::runtime_error("Unable to create screen capture output");
        __block bool started=false,failed=false;
        [stream startCaptureWithCompletionHandler:^(NSError* error) { dispatch_async(dispatch_get_main_queue(), ^{ failed=error!=nil; started=true; }); }];
        while(!started && Clock::now()<deadline) [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
        if(!started || failed) throw std::runtime_error("Screen capture could not start; check Screen Recording permission");
        PeerStop shutdown{peer};
        peer.start();
        while(peer.running() && !capture.failed && !encoder.failed()) [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
        peer.stop(); return capture.failed || encoder.failed() ? 1 : 0;
      } else throw std::runtime_error("Hosting requires macOS 12.3 or newer");
    }
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    Peer peer(options); Decoder decoder; std::atomic<unsigned> decodeFailures{0}; std::string decodeError;
    peer.onFrame=[&](Frame frame) { try { decoder.decode(frame); decodeFailures=0; } catch(const std::exception& error) { decodeError=error.what(); ++decodeFailures; peer.requestKeyframe(); } };
    NSWindow* window=[[NSWindow alloc] initWithContentRect:NSMakeRect(0,0,1280,720) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskResizable|NSWindowStyleMaskMiniaturizable backing:NSBackingStoreBuffered defer:NO];
    window.releasedWhenClosed=NO; window.title=@"Sanser · Remote desktop · Control+Option+Escape releases input";
    SanserRemoteView* view=[[SanserRemoteView alloc] initWithFrame:window.contentView.bounds]; view.peer=&peer; view.autoresizingMask=NSViewWidthSizable|NSViewHeightSizable;
    window.contentView=view; window.acceptsMouseMovedEvents=YES; [window center]; [window makeKeyAndOrderFront:nil]; [window makeFirstResponder:view]; [NSApp activateIgnoringOtherApps:YES];
    Peer* peerPointer=&peer;
    id observer=[[NSNotificationCenter defaultCenter] addObserverForName:NSWindowDidResignKeyNotification object:window queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification*) { peerPointer->input(Input{}); view.released=YES; }];
    PeerStop shutdown{peer};
    peer.start();
    while(window.visible && peer.running() && decodeFailures<10) {
      @autoreleasepool {
        NSEvent* event=[NSApp nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate dateWithTimeIntervalSinceNow:0.008] inMode:NSDefaultRunLoopMode dequeue:YES]; if(event) [NSApp sendEvent:event];
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
