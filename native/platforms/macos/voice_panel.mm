#include "voice_panel.h"
#import <Cocoa/Cocoa.h>
#include <cstdlib>
#include <stdexcept>
using sanser::desktop::VoiceRuntime;

static NSColor* panelInk() { return [NSColor colorWithSRGBRed:0.88 green:0.91 blue:0.97 alpha:1]; }
static NSColor* panelMuted() { return [NSColor colorWithSRGBRed:0.55 green:0.61 blue:0.71 alpha:1]; }
static NSColor* panelAccent() { return [NSColor colorWithSRGBRed:0.40 green:0.86 blue:0.72 alpha:1]; }
static NSColor* panelRed() { return [NSColor colorWithSRGBRed:1 green:0.49 blue:0.54 alpha:1]; }

// Clicking floating controls must not take keyboard focus from the desktop.
@interface SanserCallPanel : NSPanel
@end
@implementation SanserCallPanel
- (BOOL)canBecomeKeyWindow { return NO; }
- (BOOL)canBecomeMainWindow { return NO; }
@end

@interface SanserCallSurface : NSView
@end
@implementation SanserCallSurface
- (void)drawRect:(NSRect)dirty {
  (void)dirty;
  NSBezierPath* shape=[NSBezierPath bezierPathWithRoundedRect:NSInsetRect(self.bounds,0.5,0.5) xRadius:15 yRadius:15];
  [[NSColor colorWithSRGBRed:0.065 green:0.082 blue:0.12 alpha:0.98] setFill]; [shape fill];
  [[NSColor colorWithWhite:1 alpha:0.12] setStroke]; [shape stroke];
  if(self.bounds.size.height>56) {
    [[NSColor colorWithWhite:1 alpha:0.08] setStroke];
    NSBezierPath* line=[NSBezierPath bezierPath];
    [line moveToPoint:NSMakePoint(14,self.bounds.size.height-56)];
    [line lineToPoint:NSMakePoint(self.bounds.size.width-14,self.bounds.size.height-56)]; [line stroke];
  }
}
@end

@interface SanserCallButton : NSButton
@property(nonatomic,strong) NSColor* fill;
@property(nonatomic,assign) BOOL hovering;
@end
@implementation SanserCallButton
- (void)updateTrackingAreas {
  [super updateTrackingAreas];
  for(NSTrackingArea* area in [self.trackingAreas copy]) [self removeTrackingArea:area];
  [self addTrackingArea:[[NSTrackingArea alloc] initWithRect:self.bounds options:NSTrackingMouseEnteredAndExited|NSTrackingActiveAlways|NSTrackingInVisibleRect owner:self userInfo:nil]];
}
- (void)mouseEntered:(NSEvent*)event { (void)event; self.hovering=YES; self.needsDisplay=YES; }
- (void)mouseExited:(NSEvent*)event { (void)event; self.hovering=NO; self.needsDisplay=YES; }
- (void)drawRect:(NSRect)dirty {
  [(self.hovering && self.enabled ? [self.fill blendedColorWithFraction:0.12 ofColor:NSColor.whiteColor] : self.fill) setFill];
  [[NSBezierPath bezierPathWithRoundedRect:self.bounds xRadius:10 yRadius:10] fill];
  [super drawRect:dirty];
}
@end

@interface SanserCallLabel : NSTextField
@property(nonatomic,weak) NSButton* control;
@end
@implementation SanserCallLabel
- (void)mouseDown:(NSEvent*)event { (void)event; [self.control performClick:nil]; }
@end

@interface SanserVoicePanelActions : NSObject {
@public
  std::function<VoiceRuntime::Status()> read;
  std::function<void(sanser::voice::State)> write;
  std::function<void()> stop;
  VoiceRuntime::Status previous;
  bool hasSnapshot,lastExpanded;
}
@property(nonatomic,weak) NSPanel* panel;
@property(nonatomic,strong) NSButton* toggle;
@property(nonatomic,strong) NSButton* shortcut;
@property(nonatomic,strong) NSButton* chevron;
@property(nonatomic,strong) NSButton* stopButton;
@property(nonatomic,strong) NSButton* join;
@property(nonatomic,strong) NSButton* microphone;
@property(nonatomic,strong) NSButton* deafen;
@property(nonatomic,strong) NSButton* leave;
@property(nonatomic,strong) NSTextField* titleLabel;
@property(nonatomic,strong) NSTextField* subtitle;
@property(nonatomic,strong) NSTextField* status;
@property(nonatomic,assign) BOOL expanded;
@property(nonatomic,assign) BOOL voiceEnabled;
- (void)activateVoice:(NSButton*)sender;
- (void)refreshVoice:(id)sender;
@end

static void callIcon(NSButton* button,NSString* symbol,NSString* label,NSColor* tint,BOOL active=NO) {
  button.image=[NSImage imageWithSystemSymbolName:symbol accessibilityDescription:label];
  button.symbolConfiguration=[NSImageSymbolConfiguration configurationWithPointSize:15 weight:NSFontWeightMedium];
  button.contentTintColor=tint; button.toolTip=label;
  [button setAccessibilityLabel:label];
  ((SanserCallButton*)button).fill=active ? [tint colorWithAlphaComponent:0.16] : [NSColor colorWithWhite:1 alpha:0.045];
  button.needsDisplay=YES;
}
@implementation SanserVoicePanelActions
- (void)activateVoice:(NSButton*)sender {
  if(sender.tag==5) { if(stop) { write({false,false,false}); stop(); } return; }
  if(sender.tag==0) {
    if(!self.voiceEnabled) return;
    self.expanded=!self.expanded;
    NSRect frame=self.panel.frame; const auto height=self.expanded ? 148.0 : 56.0;
    frame.origin.y=NSMaxY(frame)-height; frame.size.height=height;
    [self.panel setFrame:frame display:YES]; [self refreshVoice:nil]; return;
  }
  if(!self.voiceEnabled || !read || !write) return;
  auto next=read().state;
  if(sender.tag==1 && !next.joined) next={true,false,false};
  if(sender.tag==2 && next.joined) { next.microphone=!next.microphone; next.deafened=false; }
  if(sender.tag==3 && next.joined) { next.deafened=!next.deafened; next.microphone=false; }
  if(sender.tag==4) next={false,false,false};
  write(next); [self refreshVoice:nil];
}
- (void)refreshVoice:(id)sender {
  (void)sender; if(!read) return;
  const auto value=read(); const auto& state=value.state;
  if(hasSnapshot && lastExpanded==static_cast<bool>(self.expanded) && previous.state.joined==state.joined && previous.state.microphone==state.microphone && previous.state.deafened==state.deafened && previous.capturing==value.capturing && previous.code==value.code) return;
  previous=value; hasSnapshot=true; lastExpanded=self.expanded;
  const bool sharing=static_cast<bool>(stop);
  const auto width=self.panel.frame.size.width,height=self.panel.frame.size.height;
  self.titleLabel.stringValue=sharing ? @"Sharing screen" : @"Group voice";
  self.subtitle.stringValue=!self.voiceEnabled ? @"Screen sharing active" : value.capturing ? @"Mic on" : state.microphone ? @"Opening mic…" : state.deafened ? @"Voice deafened" : state.joined ? @"Listening · mic off" : @"Voice not joined";
  self.subtitle.textColor=value.capturing ? panelAccent() : panelMuted();
  self.titleLabel.frame=NSMakeRect(44,height-27,122,17); self.subtitle.frame=NSMakeRect(44,height-43,122,14);
  self.toggle.frame=NSMakeRect(10,height-45,28,34);
  callIcon(self.toggle,sharing ? @"display" : @"waveform",sharing ? @"Screen sharing active · open voice controls" : @"Open voice controls",sharing || state.joined ? panelAccent() : panelInk(),YES);
  // The title itself also expands the panel without making a tiny icon the only target.
  self.shortcut.frame=NSMakeRect(width-(sharing ? 126 : 86),height-44,34,32);
  self.chevron.frame=NSMakeRect(width-(sharing ? 86 : 46),height-44,34,32);
  self.stopButton.frame=NSMakeRect(width-46,height-44,34,32);
  self.stopButton.hidden=!sharing;
  self.shortcut.hidden=self.chevron.hidden=!self.voiceEnabled;
  self.toggle.enabled=self.voiceEnabled;
  self.shortcut.enabled=state.joined;
  const auto micLabel=state.microphone ? @"Mute microphone" : @"Unmute microphone";
  callIcon(self.shortcut,state.microphone ? @"mic.fill" : @"mic.slash",micLabel,value.capturing ? panelAccent() : state.microphone ? panelRed() : panelMuted(),state.microphone);
  callIcon(self.chevron,self.expanded ? @"chevron.up" : @"chevron.down",self.expanded ? @"Collapse voice controls" : @"Expand voice controls",panelMuted());
  callIcon(self.stopButton,@"stop.fill",@"Stop screen sharing",panelRed(),YES);
  NSArray<NSButton*>* buttons=@[self.join,self.microphone,self.deafen,self.leave];
  const auto spacing=(width-32)/4;
  for(NSUInteger i=0;i<buttons.count;++i) { buttons[i].frame=NSMakeRect(16+i*spacing+(spacing-42)/2,43,42,38); buttons[i].hidden=!self.expanded; }
  self.status.hidden=!self.expanded; self.status.frame=NSMakeRect(12,10,width-24,26);
  self.join.enabled=!state.joined; self.microphone.enabled=self.deafen.enabled=self.leave.enabled=state.joined;
  callIcon(self.join,@"phone.arrow.up.right",@"Join voice · listen only",panelAccent());
  callIcon(self.microphone,state.microphone ? @"mic.fill" : @"mic.slash",micLabel,value.capturing ? panelAccent() : panelInk(),state.microphone);
  callIcon(self.deafen,state.deafened ? @"speaker.slash" : @"headphones",state.deafened ? @"Hear voice" : @"Deafen voice and mute microphone",state.deafened ? panelRed() : panelInk(),state.deafened);
  callIcon(self.leave,@"phone.down",@"Leave voice",panelRed());
  self.status.stringValue=value.code=="permission-denied" ? @"Allow Microphone in System Settings." : value.code=="no-device" ? @"No microphone available." : value.code=="unavailable" ? @"Microphone unavailable. Unmute to retry." : value.capturing ? @"Microphone on · you can talk" : state.microphone ? @"Opening microphone…" : state.deafened ? @"Voice deafened · microphone off" : state.joined ? @"Listening · unmute to talk" : @"Join to listen · microphone stays off";
  [self.panel.contentView setNeedsDisplay:YES];
}
@end

namespace sanser::desktop {
struct VoicePanel::Impl {
  NSPanel* __strong panel=nil;
  SanserVoicePanelActions* __strong action=nil;
  NSTimer* __strong timer=nil;
  Impl(bool enabled,VoiceRuntime& voice,std::function<void()> stop):Impl([&voice] { return voice.snapshot(); },[&voice](sanser::voice::State state) { voice.configure(state); },enabled,std::move(stop)) {}
  Impl(std::function<VoiceRuntime::Status()> read,std::function<void(sanser::voice::State)> write,bool enabled=true,std::function<void()> stop={}) {
    if(stop) [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    const auto area=NSScreen.mainScreen.visibleFrame; const auto width=stop ? 300.0 : 264.0;
    panel=[[SanserCallPanel alloc] initWithContentRect:NSMakeRect(NSMaxX(area)-width-20,NSMaxY(area)-76,width,56) styleMask:NSWindowStyleMaskBorderless|NSWindowStyleMaskNonactivatingPanel backing:NSBackingStoreBuffered defer:NO];
    panel.releasedWhenClosed=NO; panel.level=NSFloatingWindowLevel; panel.movableByWindowBackground=YES; panel.hidesOnDeactivate=NO;
    panel.opaque=NO; panel.backgroundColor=NSColor.clearColor; panel.hasShadow=YES;
    panel.appearance=[NSAppearance appearanceNamed:NSAppearanceNameDarkAqua];
    panel.collectionBehavior=NSWindowCollectionBehaviorCanJoinAllSpaces|NSWindowCollectionBehaviorFullScreenAuxiliary;
    panel.contentView=[[SanserCallSurface alloc] initWithFrame:NSMakeRect(0,0,width,56)];
    action=[SanserVoicePanelActions new]; action->read=std::move(read); action->write=std::move(write); action->stop=std::move(stop); action.panel=panel; action.voiceEnabled=enabled;
    const auto button=[&](NSInteger tag) {
      SanserCallButton* result=[[SanserCallButton alloc] initWithFrame:NSZeroRect];
      result.target=action; result.action=@selector(activateVoice:); result.bordered=NO; result.imagePosition=NSImageOnly;
      result.tag=tag; [panel.contentView addSubview:result]; return result;
    };
    action.toggle=button(0); action.shortcut=button(2); action.chevron=button(0); action.stopButton=button(5);
    action.join=button(1); action.microphone=button(2); action.deafen=button(3); action.leave=button(4);
    const auto label=[&](CGFloat size,NSColor* color) {
      SanserCallLabel* result=[SanserCallLabel labelWithString:@""]; result.font=[NSFont systemFontOfSize:size weight:NSFontWeightMedium]; result.textColor=color;
      result.control=action.toggle;
      [panel.contentView addSubview:result]; return result;
    };
    action.titleLabel=label(12,panelInk()); action.subtitle=label(10,panelMuted());
    action.status=label(10,panelMuted()); action.status.alignment=NSTextAlignmentCenter; action.status.maximumNumberOfLines=2;
    ((SanserCallLabel*)action.status).control=nil;
    [action refreshVoice:nil];
    timer=[NSTimer timerWithTimeInterval:0.1 target:action selector:@selector(refreshVoice:) userInfo:nil repeats:YES];
    [[NSRunLoop mainRunLoop] addTimer:timer forMode:NSRunLoopCommonModes]; [panel orderFrontRegardless];
  }
  ~Impl() { [timer invalidate]; action->read={}; action->write={}; action->stop={}; [panel close]; }
};
VoicePanel::VoicePanel(bool enabled,VoiceRuntime& voice,std::function<void()> stopSharing) {
  if(enabled || stopSharing) impl_=std::make_unique<Impl>(enabled,voice,std::move(stopSharing));
}
VoicePanel::~VoicePanel()=default;
void VoicePanel::pump() {
  if(!impl_) return;
  for(unsigned count=0;count<64;++count) {
    NSEvent* event=[NSApp nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:YES];
    if(!event) break; [NSApp sendEvent:event];
  }
}
#ifdef SANSER_CODEC_TEST
void testVoicePanelControls() {
  @autoreleasepool {
    [NSApplication sharedApplication];
    const auto preview=[](VoicePanel::Impl& panel,const char* name) {
      const auto dir=std::getenv("SANSER_PANEL_PREVIEW_DIR"); if(!dir) return;
      [panel.panel.contentView display];
      auto* bitmap=[panel.panel.contentView bitmapImageRepForCachingDisplayInRect:panel.panel.contentView.bounds];
      [panel.panel.contentView cacheDisplayInRect:panel.panel.contentView.bounds toBitmapImageRep:bitmap];
      NSString* path=[[NSString stringWithUTF8String:dir] stringByAppendingPathComponent:[NSString stringWithUTF8String:name]];
      [[bitmap representationUsingType:NSBitmapImageFileTypePNG properties:@{}] writeToFile:path atomically:YES];
    };
    VoiceRuntime::Status status; unsigned changes=0,stops=0;
    VoicePanel::Impl panel([&] { return status; },[&](voice::State next) { status.state=next; ++changes; });
    if(changes || panel.action.expanded || !panel.action.stopButton.hidden || panel.panel.canBecomeKeyWindow) throw std::runtime_error("Viewer must start voice-only, without joining, capture or focus stealing");
    preview(panel,"viewer-compact.png");
    [panel.action.toggle performClick:nil];
    if(!panel.action.expanded || changes) throw std::runtime_error("Expanding voice controls opened capture");
    [panel.action.join performClick:nil]; [panel.action refreshVoice:nil];
    if(!status.state.joined || status.state.microphone) throw std::runtime_error("Floating Join must listen only");
    [panel.action.microphone performClick:nil]; [panel.action refreshVoice:nil];
    if(!status.state.microphone) throw std::runtime_error("Floating Unmute did not request capture");
    const auto beforeCollapse=changes;
    [panel.action.toggle performClick:nil]; [panel.action.toggle performClick:nil];
    if(changes!=beforeCollapse || !status.state.microphone) throw std::runtime_error("Collapsing floating controls must not leave the call");
    [panel.action.shortcut performClick:nil]; [panel.action refreshVoice:nil];
    if(status.state.microphone) throw std::runtime_error("Compact Mute did not revoke microphone consent");
    [panel.action.microphone performClick:nil]; [panel.action refreshVoice:nil];
    [panel.action.deafen performClick:nil]; [panel.action refreshVoice:nil];
    if(!status.state.deafened || status.state.microphone) throw std::runtime_error("Floating Deafen must mute capture");
    [panel.action.leave performClick:nil];
    if(status.state.joined || status.state.microphone) throw std::runtime_error("Floating Leave did not revoke consent");
    VoicePanel::Impl host([&] { return status; },[&](voice::State next) { status.state=next; ++changes; },true,[&] { ++stops; });
    if(host.action.stopButton.hidden || host.action.titleLabel.stringValue.length==0) throw std::runtime_error("Sharing host needs a combined screen/voice panel");
    preview(host,"host-compact.png");
    [host.action.chevron performClick:nil]; preview(host,"host-expanded.png");
    [host.action.join performClick:nil]; [host.action.microphone performClick:nil]; status.capturing=true; [host.action refreshVoice:nil];
    preview(host,"host-talking.png");
    [host.action.stopButton performClick:nil];
    if(stops!=1 || status.state.joined || status.state.microphone) throw std::runtime_error("Stop sharing must also leave voice");
    VoicePanel::Impl screenOnly([&] { return status; },[&](voice::State next) { status.state=next; },false,[&] { ++stops; });
    if(screenOnly.action.stopButton.hidden || !screenOnly.action.shortcut.hidden || !screenOnly.action.chevron.hidden) throw std::runtime_error("Host without voice must retain sharing indicator and stop");
    [screenOnly.action.stopButton performClick:nil]; if(stops!=2) throw std::runtime_error("Screen-only Stop did not stop sharing");
  }
}
#endif
}
