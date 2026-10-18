#include "input_injector.h"
#include <IOKit/hidsystem/IOLLEvent.h>
#include <iostream>
#include <stdexcept>
using namespace sanser::desktop;
void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
int main() {
  @autoreleasepool {
    std::vector<CGEventRef> events;
    struct Cleanup { std::vector<CGEventRef>& events; ~Cleanup() { for(auto event:events) CFRelease(event); } } cleanup{events};
    try {
      MacInputInjector input(CGRectMake(100,50,1280,832),[&](CGEventRef event) { events.push_back(CGEventCreateCopy(event)); });
      input.apply({Input::Key,0,0,225,0,true});
      input.apply({Input::Key,0,0,4,0,true});
      require(CGEventGetType(events[0])==kCGEventFlagsChanged,"Shift is not a modifier event");
      require(CGEventGetFlags(events[0])&NX_DEVICELSHIFTKEYMASK,"Left Shift lost its sided modifier state");
      require(CGEventGetIntegerValueField(events[1],kCGKeyboardEventKeycode)==0,"The A key (macOS keycode zero) was lost");
      require((CGEventGetFlags(events[1])&kCGEventFlagMaskShift)!=0,"Shift did not reach the typed key");
      input.apply({Input::Key,0,0,4,0,false});
      input.apply({Input::Key,0,0,225,0,false});
      require(CGEventGetType(events.back())==kCGEventFlagsChanged,"Shift release is not a modifier event");
      require(!(CGEventGetFlags(events.back())&kCGEventFlagMaskShift),"Shift remained pressed after key-up");
      {
        MacInputInjector modifiers(CGRectMake(0,0,1280,832),[&](CGEventRef event) { events.push_back(CGEventCreateCopy(event)); });
        modifiers.apply({Input::Key,0,0,225,0,true});
        modifiers.apply({Input::Key,0,0,229,0,true});
        require((CGEventGetFlags(events.back())&(NX_DEVICELSHIFTKEYMASK|NX_DEVICERSHIFTKEYMASK))==(NX_DEVICELSHIFTKEYMASK|NX_DEVICERSHIFTKEYMASK),"Holding both Shift keys must preserve both sides");
        modifiers.apply({Input::Key,0,0,225,0,false});
        auto flags=CGEventGetFlags(events.back());
        require((flags&kCGEventFlagMaskShift) && (flags&NX_DEVICERSHIFTKEYMASK) && !(flags&NX_DEVICELSHIFTKEYMASK),"Releasing left Shift must not release held right Shift");
        modifiers.apply({Input::Key,0,0,30,0,true}); // Shift+1.
        require(CGEventGetFlags(events.back())&kCGEventFlagMaskShift,"Right Shift did not reach a symbol key");
        modifiers.apply({Input::Key,0,0,30,0,false});
        modifiers.apply({Input::Button,0,0,0,0,true});
        require(CGEventGetFlags(events.back())&kCGEventFlagMaskShift,"Shift+click lost its modifier");
        modifiers.apply({Input::Button,0,0,0,0,false});
        modifiers.apply(Input{});
        require(CGEventGetType(events.back())==kCGEventFlagsChanged && CGEventGetFlags(events.back())==0,"Focus reset must release held right Shift and its flags");
      }
      input.apply({Input::Button,0,0,0,0,true}); auto down=events.back();
      input.apply({Input::Button,0,0,0,0,false}); auto up=events.back();
      require(CGEventGetType(down)==kCGEventLeftMouseDown && CGEventGetType(up)==kCGEventLeftMouseUp,"Click pair is incomplete");
      require(CGEventGetIntegerValueField(down,kCGMouseEventClickState)==1 && CGEventGetIntegerValueField(up,kCGMouseEventClickState)==1,"AppKit click count must be one on down and up");
      require(CGEventGetIntegerValueField(down,kCGMouseEventNumber)==CGEventGetIntegerValueField(up,kCGMouseEventNumber),"Mouse down/up event numbers differ");
      require(CGEventGetLocation(down).x==100 && CGEventGetLocation(down).y==50,"Display origin was lost");
      input.apply({Input::Button,0,0,0,0,true});
      require(CGEventGetIntegerValueField(events.back(),kCGMouseEventClickState)==2,"Double-click was not preserved");
      input.apply({Input::Move,65535,65535});
      require(CGEventGetType(events.back())==kCGEventLeftMouseDragged && CGEventGetLocation(events.back()).x==1379 && CGEventGetLocation(events.back()).y==881,"Drag or display edge mapping differs");
      input.apply({Input::Key,0,0,227,0,true});
      input.apply({Input::Key,0,0,26,0,true}); // Windows key + W -> native Mac Command+W.
      require(CGEventGetIntegerValueField(events.back(),kCGKeyboardEventKeycode)==13 && (CGEventGetFlags(events.back())&kCGEventFlagMaskCommand),"Command+W is not forwarded");
      input.apply(Input{});
      require(CGEventGetType(events.back())==kCGEventLeftMouseUp && !(CGEventGetFlags(events.back())&kCGEventFlagMaskCommand),"Focus reset did not release buttons/modifiers");
      std::cout<<"Mac input: keycode zero, modifiers, click count, double click, drag, Command+W and reset passed without posting OS input\n";
      return 0;
    } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
  }
}
