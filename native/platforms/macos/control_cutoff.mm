#include "control_cutoff.h"
#import <Cocoa/Cocoa.h>
#import <Carbon/Carbon.h>
#include <atomic>
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
struct MacControlCutoff::Impl { EventHotKeyRef hotkey=nullptr; EventHandlerRef handler=nullptr; };

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
  if(impl_->hotkey) UnregisterEventHotKey(impl_->hotkey);
  if(impl_->handler) RemoveEventHandler(impl_->handler);
}
void MacControlCutoff::update() {
  if(!impl_->hotkey) return;
  // Carbon hot keys arrive through the application event queue.
  while(NSEvent* event=[NSApp nextEventMatchingMask:NSEventMaskAny untilDate:nil inMode:NSDefaultRunLoopMode dequeue:YES]) [NSApp sendEvent:event];
  if(pressed.exchange(false) && cutoff::revoke()) cutoff::announce();
}
bool MacControlCutoff::allow(const Input& event) {
  const bool release=(event.kind==Input::Key || event.kind==Input::Button) ? !event.down : event.kind==Input::Reset;
  return cutoff::allows(release);
}
}
