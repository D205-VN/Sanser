#include "control_cutoff.h"
#import <Cocoa/Cocoa.h>
#import <Carbon/Carbon.h>
#include <atomic>
#include <chrono>
#include <iostream>

namespace sanser::desktop {
namespace {
std::atomic<bool> pressed{false};
OSStatus handler(EventHandlerCallRef,EventRef event,void*) {
  EventHotKeyID id{};
  if(GetEventParameter(event,kEventParamDirectObject,typeEventHotKeyID,nullptr,sizeof(id),nullptr,&id)!=noErr ||
     id.signature!=controlCutoffHotKeySignature) return eventNotHandledErr;
  pressed=true;
  return noErr;
}
}
// A short on-screen notice at the host, so the person who pressed the hotkey
// sees whether control is now off or on (no notification permission needed).
static NSPanel* makeNotice() {
  NSPanel* panel=[[NSPanel alloc] initWithContentRect:NSMakeRect(0,0,460,56) styleMask:NSWindowStyleMaskBorderless|NSWindowStyleMaskNonactivatingPanel backing:NSBackingStoreBuffered defer:YES];
  panel.opaque=NO; panel.backgroundColor=NSColor.clearColor; panel.hasShadow=YES;
  panel.level=NSStatusWindowLevel; panel.ignoresMouseEvents=YES; panel.releasedWhenClosed=NO;
  panel.collectionBehavior=NSWindowCollectionBehaviorCanJoinAllSpaces|NSWindowCollectionBehaviorFullScreenAuxiliary|NSWindowCollectionBehaviorStationary;
  NSView* box=[[NSView alloc] initWithFrame:panel.contentView.bounds];
  box.wantsLayer=YES; box.layer.cornerRadius=14; box.layer.backgroundColor=[NSColor colorWithWhite:0.08 alpha:0.9].CGColor;
  NSTextField* label=[NSTextField labelWithString:@""];
  label.font=[NSFont systemFontOfSize:15 weight:NSFontWeightSemibold]; label.textColor=NSColor.whiteColor;
  label.alignment=NSTextAlignmentCenter; label.frame=NSMakeRect(12,17,436,22); label.tag=1;
  [box addSubview:label]; panel.contentView=box;
  return panel;
}
struct MacControlCutoff::Impl {
  EventHotKeyRef hotkey=nullptr; EventHandlerRef handler=nullptr;
  NSPanel* notice=nil; std::chrono::steady_clock::time_point hideAt{};
  void show(bool off) {
    if(!notice) notice=makeNotice();
    NSTextField* label=[notice.contentView viewWithTag:1];
    label.stringValue=[NSString stringWithUTF8String:cutoff::notice(off)];
    NSScreen* screen=NSScreen.mainScreen ?: NSScreen.screens.firstObject;
    if(screen) { const auto area=screen.visibleFrame; [notice setFrameOrigin:NSMakePoint(NSMidX(area)-230,NSMaxY(area)-96)]; }
    [notice orderFrontRegardless];
    hideAt=std::chrono::steady_clock::now()+std::chrono::milliseconds(2500);
  }
};

MacControlCutoff::MacControlCutoff(bool input):impl_(std::make_unique<Impl>()) {
  cutoff::reset(); pressed=false;
  if(!input) return; // View-only: there is no remote control to end.
  EventTypeSpec type{kEventClassKeyboard,kEventHotKeyPressed};
  InstallApplicationEventHandler(&handler,1,&type,nullptr,&impl_->handler);
  const EventHotKeyID id{controlCutoffHotKeySignature,1};
  if(RegisterEventHotKey(kVK_ANSI_K,controlKey|cmdKey,id,GetApplicationEventTarget(),0,&impl_->hotkey)!=noErr) {
    impl_->hotkey=nullptr;
    std::cerr<<"SNCONTROL hotkey=unavailable platform=macos"<<std::endl;
  }
}
MacControlCutoff::~MacControlCutoff() {
  if(impl_->notice) [impl_->notice orderOut:nil];
  if(impl_->hotkey) UnregisterEventHotKey(impl_->hotkey);
  if(impl_->handler) RemoveEventHandler(impl_->handler);
}
bool MacControlCutoff::update() {
  if(!impl_->hotkey) return false;
  // Carbon hot keys arrive through the application event queue.
  while(NSEvent* event=[NSApp nextEventMatchingMask:NSEventMaskAny untilDate:nil inMode:NSDefaultRunLoopMode dequeue:YES]) [NSApp sendEvent:event];
  bool turnedOff=false;
  if(pressed.exchange(false)) { const bool off=cutoff::toggle(); cutoff::announce(off); impl_->show(off); turnedOff=off; }
  if(impl_->notice && impl_->notice.visible && std::chrono::steady_clock::now()>=impl_->hideAt) [impl_->notice orderOut:nil];
  return turnedOff;
}
bool MacControlCutoff::allow(const Input& event) {
  const bool release=(event.kind==Input::Key || event.kind==Input::Button) ? !event.down : event.kind==Input::Reset;
  return cutoff::allows(release);
}
}
