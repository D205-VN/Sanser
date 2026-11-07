#include "input_injector.h"
#include "keyboard.h"
#include "text_input.h"
#include <IOKit/hidsystem/IOLLEvent.h>
#include <cmath>
#include <stdexcept>
namespace sanser::desktop {
MacInputInjector::MacInputInjector(CGRect bounds,std::function<void(CGEventRef)> post):bounds_(bounds),post_(std::move(post)) {
  source_=CGEventSourceCreate(kCGEventSourceStatePrivate);
  if(!source_) throw std::runtime_error("Unable to create the remote input event source");
  CGEventSourceSetLocalEventsSuppressionInterval(source_,0);
  if(!post_) post_=[](CGEventRef event) { CGEventPost(kCGHIDEventTap,event); };
}
MacInputInjector::~MacInputInjector() { try { reset(); } catch(...) {} CFRelease(source_); }
CGEventFlags MacInputInjector::flags() const {
  CGEventFlags flags=0;
  if(keys_.contains(224)||keys_.contains(228)) flags|=kCGEventFlagMaskControl;
  if(keys_.contains(225)||keys_.contains(229)) flags|=kCGEventFlagMaskShift;
  if(keys_.contains(226)||keys_.contains(230)) flags|=kCGEventFlagMaskAlternate;
  if(keys_.contains(227)||keys_.contains(231)) flags|=kCGEventFlagMaskCommand;
  // Keep the sided state as well as the aggregate flags. A held right Shift
  // must survive releasing left Shift, including on mouse/shortcut events.
  static constexpr CGEventFlags sided[]={NX_DEVICELCTLKEYMASK,NX_DEVICELSHIFTKEYMASK,
    NX_DEVICELALTKEYMASK,NX_DEVICELCMDKEYMASK,NX_DEVICERCTLKEYMASK,
    NX_DEVICERSHIFTKEYMASK,NX_DEVICERALTKEYMASK,NX_DEVICERCMDKEYMASK};
  for(unsigned i=0;i<8;++i) if(keys_.contains(224+i)) flags|=sided[i];
  return flags;
}
void MacInputInjector::emit(CGEventRef event) {
  if(!event) throw std::runtime_error("Unable to create a remote input event");
  CGEventSetFlags(event,flags());
  struct Release { CGEventRef event; ~Release() { CFRelease(event); } } release{event};
  post_(event);
}
void MacInputInjector::apply(const Input& input) {
  if(input.kind==Input::Reset) { reset(); return; }
  if(input.kind==Input::Text) {
    const auto text=sanser::textInputUtf16(input.text); if(!text) return;
    // A committed edit is literal text, never a held Ctrl/Command shortcut.
    reset();
    for(std::size_t i=0;i<text->size();) {
      const auto unit=(*text)[i];
      if(unit=='\n' || unit=='\t') {
        const std::uint16_t code=unit=='\n' ? 40:43;
        apply({Input::Key,0,0,code,0,true}); apply({Input::Key,0,0,code,0,false}); ++i; continue;
      }
      const auto count=unit>=0xd800 && unit<=0xdbff ? 2U:1U;
      const UniChar scalar[]={static_cast<UniChar>(unit),static_cast<UniChar>(count==2 ? (*text)[i+1]:0)};
      for(bool down:{true,false}) {
        auto event=CGEventCreateKeyboardEvent(source_,0,down);
        if(event) CGEventKeyboardSetUnicodeString(event,count,scalar);
        emit(event);
      }
      i+=count;
    }
    return;
  }
  if(input.kind==Input::Key) {
    const auto key=macKey(input.code); if(key<0) return;
    if(input.down) keys_.insert(input.code); else keys_.erase(input.code);
    auto event=CGEventCreateKeyboardEvent(source_,static_cast<CGKeyCode>(key),input.down);
    if(event && input.code>=224) CGEventSetType(event,kCGEventFlagsChanged);
    emit(event); return;
  }
  if(input.kind==Input::Scroll) {
    auto event=CGEventCreateScrollWheelEvent(source_,kCGScrollEventUnitLine,1,static_cast<int>(input.delta));
    if(event) CGEventSetLocation(event,point_);
    emit(event); return;
  }
  point_=CGPointMake(bounds_.origin.x+input.x/65535.0*std::max(0.0,bounds_.size.width-1),
                     bounds_.origin.y+input.y/65535.0*std::max(0.0,bounds_.size.height-1));
  CGMouseButton button=kCGMouseButtonLeft; CGEventType type=kCGEventMouseMoved;
  if(input.kind==Input::Button) {
    if(input.code>2) return;
    button=static_cast<CGMouseButton>(input.code);
    if(input.down) buttons_.insert(input.code); else buttons_.erase(input.code);
    type=button==kCGMouseButtonLeft ? (input.down?kCGEventLeftMouseDown:kCGEventLeftMouseUp) :
         button==kCGMouseButtonRight ? (input.down?kCGEventRightMouseDown:kCGEventRightMouseUp) :
         (input.down?kCGEventOtherMouseDown:kCGEventOtherMouseUp);
    auto& click=clicks_[input.code];
    if(input.down) {
      const auto now=Clock::now();
      const bool adjacent=now-click.at<std::chrono::milliseconds(500) && std::hypot(point_.x-click.point.x,point_.y-click.point.y)<4;
      click.count=adjacent ? click.count%3+1 : 1; click.at=now; click.point=point_; click.number=++number_;
    }
  } else if(!buttons_.empty()) {
    button=static_cast<CGMouseButton>(*buttons_.begin());
    type=button==kCGMouseButtonLeft ? kCGEventLeftMouseDragged : button==kCGMouseButtonRight ? kCGEventRightMouseDragged : kCGEventOtherMouseDragged;
  }
  auto event=CGEventCreateMouseEvent(source_,type,point_,button);
  if(event && input.kind==Input::Button) {
    const auto& click=clicks_[input.code];
    CGEventSetIntegerValueField(event,kCGMouseEventClickState,std::max(1U,click.count));
    CGEventSetIntegerValueField(event,kCGMouseEventNumber,click.number);
    CGEventSetIntegerValueField(event,kCGMouseEventButtonNumber,input.code);
  }
  emit(event);
}
void MacInputInjector::reset() {
  while(!keys_.empty()) { const auto code=*keys_.begin(); apply({Input::Key,0,0,code,0,false}); }
  while(!buttons_.empty()) {
    const auto code=*buttons_.begin(); buttons_.erase(code);
    auto event=CGEventCreateMouseEvent(source_,code==0?kCGEventLeftMouseUp:code==1?kCGEventRightMouseUp:kCGEventOtherMouseUp,point_,static_cast<CGMouseButton>(code));
    if(event) { CGEventSetIntegerValueField(event,kCGMouseEventClickState,1); CGEventSetIntegerValueField(event,kCGMouseEventNumber,clicks_[code].number); }
    emit(event);
  }
}
}
