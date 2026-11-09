#include "input_injector.h"
#include "windows_mac_clipboard.h"
#include "keyboard.h"
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
      {
        WindowsMacClipboard mapper;
        for(auto code:{4,6,9,22,25,26,27,29}) {
          const auto before=events.size();
          for(const auto& edge:std::vector<Input>{{Input::Key,0,0,228,0,true},{Input::Key,0,0,static_cast<std::uint16_t>(code),0,true},
                {Input::Key,0,0,static_cast<std::uint16_t>(code),0,false},{Input::Key,0,0,228,0,false}})
            for(const auto& translated:mapper.apply(edge,true,true)) input.apply(translated);
          require(events.size()==before+8,"Windows clipboard chord lost an edge before Mac injection");
          auto pressed=events[before+3];
          require(CGEventGetIntegerValueField(pressed,kCGKeyboardEventKeycode)==macKey(code) &&
            (CGEventGetFlags(pressed)&kCGEventFlagMaskCommand) && !(CGEventGetFlags(pressed)&kCGEventFlagMaskControl),
            "Windows Ctrl+C/V must arrive as native Command+C/V without Control");
          require(CGEventGetFlags(events.back())==0,"Windows clipboard chord left a Mac modifier pressed");
        }
        for(auto event:events)CFRelease(event);events.clear();
        for(auto shift:{225,229}) {
          for(const auto& edge:std::vector<Input>{{Input::Key,0,0,224,0,true},{Input::Key,0,0,static_cast<std::uint16_t>(shift),0,true},{Input::Key,0,0,29,0,true}})
            for(const auto& translated:mapper.apply(edge,true,true))input.apply(translated);
          const auto flags=CGEventGetFlags(events.back());
          require((flags&kCGEventFlagMaskCommand) && (flags&kCGEventFlagMaskShift) && !(flags&kCGEventFlagMaskControl),"Windows redo lost Command/Shift or retained Control on Mac");
          for(const auto& event:mapper.apply(Input{},true,true))input.apply(event);
        }
        for(auto event:events)CFRelease(event);events.clear();
      }
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
      input.apply({Input::Key,0,0,227,0,true});
      Input edit; edit.kind=Input::Text; edit.text="ế😀";
      const auto before=events.size(); input.apply(edit);
      require(events.size()==before+5 && CGEventGetType(events[before])==kCGEventFlagsChanged,"Text must release held Command first");
      for(std::size_t i=before+1;i<events.size();++i) require(CGEventGetFlags(events[i])==0,"Unicode must not turn into a held-modifier shortcut");
      UniChar units[4]{}; UniCharCount count=0;
      CGEventKeyboardGetUnicodeString(events[before+1],4,&count,units);
      require(count==1 && units[0]==0x1ebf,"Vietnamese was not carried in the native key event");
      CGEventKeyboardGetUnicodeString(events[before+3],4,&count,units);
      require(count==2 && units[0]==0xd83d && units[1]==0xde00,"Emoji surrogate pair was split between Mac key events");
      edit.text="valid\xed\xa0\x80"; const auto unchanged=events.size(); input.apply(edit);
      require(events.size()==unchanged,"Malformed Unicode injected a partial prefix");
      input.apply({Input::Key,0,0,4,0,true});
      require(!CGEventGetIntegerValueField(events.back(),kCGKeyboardEventAutorepeat),"First key-down must not be a repeat");
      input.apply({Input::Key,0,0,4,0,true});
      require(CGEventGetIntegerValueField(events.back(),kCGKeyboardEventAutorepeat)==1,"Held typing key lost its native autorepeat marker");
      input.apply(Input{});
      input.apply({Input::Key,0,0,57,0,true});
      require(CGEventGetType(events.back())==kCGEventFlagsChanged && (CGEventGetFlags(events.back())&kCGEventFlagMaskAlphaShift),"Caps Lock did not toggle native flags");
      input.apply({Input::Key,0,0,57,0,true});input.apply({Input::Key,0,0,57,0,false});
      input.apply({Input::Key,0,0,4,0,true});
      require(CGEventGetFlags(events.back())&kCGEventFlagMaskAlphaShift,"Caps Lock repeat/up lost the toggle");
      edit.text="a";input.apply(edit);
      require(CGEventGetFlags(events.back())==0,"Literal text inherited Caps Lock");
      input.apply({Input::Key,0,0,57,0,true});input.apply({Input::Key,0,0,57,0,false});
      require(!(CGEventGetFlags(events.back())&kCGEventFlagMaskAlphaShift),"Second Caps Lock press did not turn it off");
      input.apply({Input::Scroll,0,0,0,3,false});
      require(CGEventGetIntegerValueField(events.back(),kCGScrollWheelEventDeltaAxis1)==3,"Vertical scroll changed direction");
      input.apply({Input::Scroll,0,0,1,2,false});
      require(CGEventGetIntegerValueField(events.back(),kCGScrollWheelEventDeltaAxis1)==0 && CGEventGetIntegerValueField(events.back(),kCGScrollWheelEventDeltaAxis2)==-2,"Horizontal Windows scroll must preserve rightward direction on Mac");
      const auto scrollCount=events.size();input.apply({Input::Scroll,0,0,2,1,false});
      require(events.size()==scrollCount,"Unsupported scroll axis reached native injection");
      for(const auto& mapping:keys) {
        input.apply(Input{});
        input.apply({Input::Key,0,0,mapping.hid,0,true});
        require(CGEventGetIntegerValueField(events.back(),kCGKeyboardEventKeycode)==mapping.mac,"Supported HID key mapped to the wrong Mac key");
        input.apply({Input::Key,0,0,mapping.hid,0,false});
        require(CGEventGetIntegerValueField(events.back(),kCGKeyboardEventKeycode)==mapping.mac,"Key-up used a different Mac key");
        if(mapping.hid==57) {input.apply({Input::Key,0,0,57,0,true});input.apply({Input::Key,0,0,57,0,false});}
      }
      input.apply(Input{});
      for(std::uint16_t button:{1,2}) {
        input.apply({Input::Button,20000,30000,button,0,true});
        require(CGEventGetType(events.back())==(button==1 ? kCGEventRightMouseDown:kCGEventOtherMouseDown),"Right/middle mouse down type differs");
        input.apply({Input::Move,40000,50000});
        require(CGEventGetType(events.back())==(button==1 ? kCGEventRightMouseDragged:kCGEventOtherMouseDragged),"Right/middle drag lost the held button");
        input.apply({Input::Button,40000,50000,button,0,false});
        require(CGEventGetType(events.back())==(button==1 ? kCGEventRightMouseUp:kCGEventOtherMouseUp),"Right/middle mouse release type differs");
      }
      std::cout<<"Mac input: modifiers, clicks, Unicode scalar pairs and reset passed without posting OS input\n";
      return 0;
    } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
  }
}
