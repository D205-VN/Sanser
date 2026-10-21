#include "voice_panel.h"
#import <Cocoa/Cocoa.h>
#include <stdexcept>
using sanser::desktop::VoiceRuntime;
@interface SanserVoicePanelActions : NSObject {
@public
  std::function<VoiceRuntime::Status()> read;
  std::function<void(sanser::voice::State)> write;
}
@property(nonatomic,weak) NSPanel* panel;
@property(nonatomic,strong) NSButton* toggle;
@property(nonatomic,strong) NSButton* join;
@property(nonatomic,strong) NSButton* microphone;
@property(nonatomic,strong) NSButton* deafen;
@property(nonatomic,strong) NSButton* leave;
@property(nonatomic,strong) NSTextField* status;
@property(nonatomic,assign) BOOL expanded;
- (void)activateVoice:(NSButton*)sender;
- (void)refreshVoice:(id)sender;
@end
@implementation SanserVoicePanelActions
- (void)activateVoice:(NSButton*)sender {
  if(!read || !write) return;
  if(sender.tag==0) {
    self.expanded=!self.expanded;
    NSRect frame=self.panel.frame; const auto height=self.expanded ? 168.0 : 44.0;
    frame.origin.y=NSMaxY(frame)-height; frame.size.height=height;
    [self.panel setFrame:frame display:YES];
    [self refreshVoice:nil]; return;
  }
  auto next=read().state;
  if(sender.tag==1) next={true,false,false};
  if(sender.tag==2 && next.joined) { next.microphone=!next.microphone; next.deafened=false; }
  if(sender.tag==3 && next.joined) { next.deafened=!next.deafened; next.microphone=false; }
  if(sender.tag==4) next={false,false,false};
  write(next);
}
- (void)refreshVoice:(id)sender {
  (void)sender; if(!read || !write) return;
  const auto value=read(); const auto& state=value.state;
  self.toggle.title=value.capturing ? @"Sanser · Voice · Mic on" : state.joined ? @"Sanser · Voice · Mic off" : @"Sanser · Voice";
  const auto height=self.panel.frame.size.height;
  self.toggle.frame=NSMakeRect(8,height-38,304,30);
  self.join.frame=NSMakeRect(8,88,145,30); self.microphone.frame=NSMakeRect(163,88,149,30);
  self.deafen.frame=NSMakeRect(8,50,145,30); self.leave.frame=NSMakeRect(163,50,149,30);
  self.status.frame=NSMakeRect(12,8,296,36);
  self.join.hidden=self.microphone.hidden=self.deafen.hidden=self.leave.hidden=self.status.hidden=!self.expanded;
  self.join.enabled=!state.joined; self.microphone.enabled=self.deafen.enabled=self.leave.enabled=state.joined;
  self.microphone.title=state.microphone ? @"Mute mic" : @"Unmute mic";
  self.deafen.title=state.deafened ? @"Hear voice" : @"Deafen voice";
  self.status.stringValue=value.code=="permission-denied" ? @"Allow Microphone in System Settings." : value.code=="no-device" ? @"No microphone is available." : value.code=="unavailable" ? @"Microphone unavailable. Unmute to retry." : value.capturing ? @"Microphone on" : state.microphone ? @"Opening microphone…" : state.deafened ? @"Voice deafened · microphone off" : state.joined ? @"Listening · microphone off" : @"Join to listen. Unmute to talk.";
}
@end
namespace sanser::desktop {
struct VoicePanel::Impl {
  NSPanel* __strong panel=nil;
  SanserVoicePanelActions* __strong action=nil;
  NSTimer* __strong timer=nil;
  explicit Impl(VoiceRuntime& voice):Impl([&voice] { return voice.snapshot(); },[&voice](sanser::voice::State state) { voice.configure(state); }) {}
  Impl(std::function<VoiceRuntime::Status()> read,std::function<void(sanser::voice::State)> write) {
    const auto area=NSScreen.mainScreen.visibleFrame;
    panel=[[NSPanel alloc] initWithContentRect:NSMakeRect(NSMaxX(area)-340,NSMaxY(area)-136,320,44) styleMask:NSWindowStyleMaskBorderless|NSWindowStyleMaskNonactivatingPanel backing:NSBackingStoreBuffered defer:NO];
    panel.releasedWhenClosed=NO; panel.level=NSFloatingWindowLevel; panel.movableByWindowBackground=YES; panel.hidesOnDeactivate=NO;
    panel.appearance=[NSAppearance appearanceNamed:NSAppearanceNameDarkAqua];
    panel.collectionBehavior=NSWindowCollectionBehaviorCanJoinAllSpaces|NSWindowCollectionBehaviorFullScreenAuxiliary;
    panel.contentView.wantsLayer=YES; panel.contentView.layer.cornerRadius=12;
    action=[SanserVoicePanelActions new]; action->read=std::move(read); action->write=std::move(write); action.panel=panel;
    const auto button=[&](NSString* title,NSInteger tag) {
      NSButton* result=[NSButton buttonWithTitle:title target:action action:@selector(activateVoice:)];
      result.tag=tag; [panel.contentView addSubview:result]; return result;
    };
    action.toggle=button(@"Sanser · Voice",0); action.join=button(@"Join voice",1);
    action.microphone=button(@"Unmute mic",2); action.deafen=button(@"Deafen voice",3); action.leave=button(@"Leave voice",4);
    action.status=[NSTextField wrappingLabelWithString:@""]; [panel.contentView addSubview:action.status];
    [action refreshVoice:nil];
    timer=[NSTimer timerWithTimeInterval:0.1 target:action selector:@selector(refreshVoice:) userInfo:nil repeats:YES];
    [[NSRunLoop mainRunLoop] addTimer:timer forMode:NSRunLoopCommonModes];
    [panel orderFrontRegardless];
  }
  ~Impl() { [timer invalidate]; action->read={}; action->write={}; [panel close]; }
};
VoicePanel::VoicePanel(bool enabled,VoiceRuntime& voice) { if(enabled) impl_=std::make_unique<Impl>(voice); }
VoicePanel::~VoicePanel()=default;
#ifdef SANSER_CODEC_TEST
void testVoicePanelControls() {
  @autoreleasepool {
    [NSApplication sharedApplication];
    VoiceRuntime::Status status; unsigned changes=0;
    VoicePanel::Impl panel([&] { return status; },[&](voice::State next) { status.state=next; ++changes; });
    if(changes || panel.action.expanded) throw std::runtime_error("Floating voice must start without joining or capturing");
    [panel.action.toggle performClick:nil];
    if(!panel.action.expanded || changes) throw std::runtime_error("Expanding voice controls opened capture");
    [panel.action.join performClick:nil]; [panel.action refreshVoice:nil];
    if(!status.state.joined || status.state.microphone) throw std::runtime_error("Floating Join must listen only");
    [panel.action.microphone performClick:nil]; [panel.action refreshVoice:nil];
    if(!status.state.microphone) throw std::runtime_error("Floating Unmute did not request capture");
    const auto beforeCollapse=changes;
    [panel.action.toggle performClick:nil]; [panel.action.toggle performClick:nil];
    if(changes!=beforeCollapse || !status.state.microphone) throw std::runtime_error("Collapsing floating controls must not leave the call");
    [panel.action.microphone performClick:nil]; [panel.action refreshVoice:nil];
    if(status.state.microphone) throw std::runtime_error("Floating Mute did not revoke microphone consent");
    [panel.action.microphone performClick:nil]; [panel.action refreshVoice:nil];
    [panel.action.deafen performClick:nil]; [panel.action refreshVoice:nil];
    if(!status.state.deafened || status.state.microphone) throw std::runtime_error("Floating Deafen must mute capture");
    [panel.action.leave performClick:nil];
    if(status.state.joined || status.state.microphone) throw std::runtime_error("Floating Leave did not revoke consent");
  }
}
#endif
}
