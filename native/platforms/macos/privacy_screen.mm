#include "privacy_screen.h"
#import <Carbon/Carbon.h>
#include <atomic>
#include <iostream>

namespace sanser::desktop {
namespace {
std::atomic<bool> hotkeyPressed{false};
OSStatus hotkeyHandler(EventHandlerCallRef,EventRef,void*) { hotkeyPressed=true; return noErr; }
}
struct MacPrivacyScreen::Impl {
  NSMutableArray<NSWindow*>* windows=[NSMutableArray array];
  EventHotKeyRef hotkey=nullptr;
  EventHandlerRef handler=nullptr;
  bool visible=false;
};
MacPrivacyScreen::MacPrivacyScreen(bool enabled):impl_(std::make_unique<Impl>()) {
  state_.enabled=enabled;
  if(!enabled) return;
  hotkeyPressed=false;
  for(NSScreen* screen in NSScreen.screens) {
    NSWindow* window=[[NSWindow alloc] initWithContentRect:screen.frame styleMask:NSWindowStyleMaskBorderless backing:NSBackingStoreBuffered defer:NO];
    window.releasedWhenClosed=NO;
    window.backgroundColor=NSColor.blackColor; window.opaque=NO; window.alphaValue=0;
    window.level=NSScreenSaverWindowLevel; window.ignoresMouseEvents=YES; window.hasShadow=NO;
    // Remote clicks must reach the applications below; never activate.
    window.collectionBehavior=NSWindowCollectionBehaviorCanJoinAllSpaces|NSWindowCollectionBehaviorStationary|
      NSWindowCollectionBehaviorIgnoresCycle|NSWindowCollectionBehaviorFullScreenAuxiliary;
    [window setFrame:screen.frame display:NO];
    [window orderFrontRegardless];
    [impl_->windows addObject:window];
  }
  EventTypeSpec type{kEventClassKeyboard,kEventHotKeyPressed};
  InstallApplicationEventHandler(&hotkeyHandler,1,&type,nullptr,&impl_->handler);
  const EventHotKeyID id{'SNPV',1};
  if(RegisterEventHotKey(kVK_ANSI_P,controlKey|optionKey|cmdKey,id,GetApplicationEventTarget(),0,&impl_->hotkey)!=noErr) {
    impl_->hotkey=nullptr;
    std::cerr<<"SNPRIVACY hotkey=unavailable"<<std::endl;
  }
}
MacPrivacyScreen::~MacPrivacyScreen() {
  if(impl_->hotkey) UnregisterEventHotKey(impl_->hotkey);
  if(impl_->handler) RemoveEventHandler(impl_->handler);
  for(NSWindow* window in impl_->windows) [window orderOut:nil];
  [impl_->windows removeAllObjects];
}
NSArray<NSNumber*>* MacPrivacyScreen::windowIds() const {
  NSMutableArray<NSNumber*>* ids=[NSMutableArray array];
  for(NSWindow* window in impl_->windows) [ids addObject:@(window.windowNumber)];
  return ids;
}
void MacPrivacyScreen::setCaptureExcluded(bool excluded) { state_.captureExcluded=state_.enabled && excluded; }
void MacPrivacyScreen::update(bool sessionReady) {
  if(!state_.enabled) return;
  // Carbon hot keys arrive through the application event queue; the host
  // loop otherwise only runs the run loop.
  while(NSEvent* event=[NSApp nextEventMatchingMask:NSEventMaskAny untilDate:nil inMode:NSDefaultRunLoopMode dequeue:YES]) [NSApp sendEvent:event];
  if(hotkeyPressed.exchange(false)) state_.dismissed=true;
  state_.sessionReady=sessionReady;
  const bool show=privacy::showOverlay(state_);
  if(show!=impl_->visible) {
    impl_->visible=show;
    for(NSWindow* window in impl_->windows) { window.alphaValue=show ? 1 : 0; if(show) [window orderFrontRegardless]; }
  }
  if(auto line=privacy::marker(state_); line!=marker_) { marker_=line; std::cerr<<line<<std::endl; }
}
}
