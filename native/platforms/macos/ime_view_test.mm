#import "ime_view.h"
#include <iostream>
#include <stdexcept>
#include <vector>

using namespace sanser::desktop;
@interface ImeTestView : SanserSnv2RemoteView {
@public
  std::vector<Input> events;
  std::vector<std::string> commits;
  unsigned interpretations;
}
@property(nonatomic) BOOL supportsText;
@property(nonatomic) BOOL acceptText;
@property(nonatomic) std::uint32_t epoch;
@end
@implementation ImeTestView
- (BOOL)unicodeAvailable { return self.inputEnabled && !self.released && self.supportsText; }
- (std::uint32_t)textGeneration { return self.epoch; }
- (NSSize)imageSize { return NSMakeSize(640,360); }
- (void)sendInput:(Input)input { events.push_back(std::move(input)); }
- (BOOL)commitText:(NSString*)value {
  if(!self.acceptText) return NO;
  NSData* bytes=[value dataUsingEncoding:NSUTF8StringEncoding allowLossyConversion:NO];
  commits.emplace_back(static_cast<const char*>(bytes.bytes),bytes.length); return YES;
}
- (void)interpretKeyEvents:(NSArray<NSEvent*>*)values { interpretations+=static_cast<unsigned>(values.count); }
@end

namespace {
void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
NSEvent* key(unsigned code,NSEventModifierFlags flags=0,bool up=false,bool repeat=false) {
  return [NSEvent keyEventWithType:up ? NSEventTypeKeyUp:NSEventTypeKeyDown location:NSZeroPoint modifierFlags:flags timestamp:0 windowNumber:0 context:nil characters:@"a" charactersIgnoringModifiers:@"a" isARepeat:repeat keyCode:code];
}
ImeTestView* view() {
  auto* value=[[ImeTestView alloc] initWithFrame:NSMakeRect(0,0,640,360)];
  value.supportsText=YES; value.acceptText=YES; value.inputEnabled=YES; value.epoch=1; return value;
}
const NSRange unspecified=NSMakeRange(NSNotFound,0);
void marked(ImeTestView* value,NSString* text) {
  [value setMarkedText:text selectedRange:NSMakeRange(text.length,0) replacementRange:unspecified];
}
void testModeAndPhysicalKeys() {
  auto* value=view();
  [value keyDown:key(0)]; [value keyUp:key(0,0,true)];
  require(!value.textMode && value->events.size()==2 && value->events[0].kind==Input::Key && value->events[0].code==4 && value->events[0].down && !value->events[1].down,"default HID game key path changed");
  value->events.clear(); value.supportsText=NO; [value toggleTextMode];
  require(!value.textMode && value->events.empty(),"old host entered text mode or received input");
  value.supportsText=YES;
  const auto toggle=NSEventModifierFlagControl|NSEventModifierFlagOption;
  [value keyDown:key(34,toggle)]; [value keyDown:key(34,toggle,false,true)]; [value keyUp:key(34,toggle,true)];
  require(value.textMode && value->events.size()==1 && value->events[0].kind==Input::Reset,"toggle repeated or leaked a physical I key");
  value->events.clear();
  [value keyDown:key(0)]; [value keyUp:key(0,0,true)];
  require(value->interpretations==1 && value->events.empty(),"text key forwarded HID before IME decided");
  [value insertText:@"A" replacementRange:unspecified];
  [value keyUp:key(0,0,true)];
  require(value->commits==std::vector<std::string>{"A"} && value->events.empty(),"text commit duplicated physical key-up");
  // Ctrl/Command shortcuts still use HID, including held Shift for selection.
  [value keyDown:key(8,NSEventModifierFlagCommand|NSEventModifierFlagShift)];
  [value keyUp:key(8,0,true)]; [value flagsChanged:key(55)];
  require(value->events.size()==6 && value->events[0].code==225 && value->events[0].down && value->events[1].code==227 && value->events[1].down && value->events[2].code==6 && value->events[2].down && !value->events[3].down && !value->events[4].down && !value->events[5].down,"text mode shortcut modifiers did not release");
  value->events.clear();
  [value keyDown:key(123,NSEventModifierFlagShift)]; [value keyUp:key(123,0,true)]; [value flagsChanged:key(56)];
  require(value->events.size()==4 && value->events[0].code==225 && value->events[1].code==80 && value->events[1].down && !value->events.back().down,"Shift+arrow selection broken in text mode");
  value->events.clear();
  [value keyDown:key(118,NSEventModifierFlagOption)]; [value keyUp:key(118,0,true)]; [value flagsChanged:key(58)];
  [value keyDown:key(76)]; [value keyUp:key(76,0,true)];
  require(value->events.size()==6 && value->events[0].code==226 && value->events[1].code==61 && value->events[1].down && !value->events[3].down && value->events[4].code==88 && value->events[4].down && !value->events[5].down,"Alt+F4 or numeric Enter lost in text mode");
  value->events.clear();
  const auto mouse=[](NSEventType type) { return [NSEvent mouseEventWithType:type location:NSMakePoint(100,100) modifierFlags:NSEventModifierFlagShift timestamp:0 windowNumber:0 context:nil eventNumber:1 clickCount:1 pressure:1]; };
  [value mouseDown:mouse(NSEventTypeLeftMouseDown)]; [value mouseUp:mouse(NSEventTypeLeftMouseUp)]; [value flagsChanged:key(56)];
  require(value->events.size()==4 && value->events[0].code==225 && value->events[0].down && value->events[1].kind==Input::Button && value->events[1].down && !value->events[2].down && !value->events[3].down,"Shift+click lost modifier in text mode");
  [value toggleTextMode]; require(!value.textMode && !value.hasMarkedText,"toggle failed to return to physical mode");
}
void testComposition() {
  auto* value=view(); [value toggleTextMode]; value->events.clear();
  marked(value,@"tiee"); marked(value,@"tiế");
  require(value.hasMarkedText && value.markedRange.length==3 && value.selectedRange.location==3 && value->commits.empty() && value->events.empty(),"intermediate IME candidates escaped the controller");
  [value setMarkedText:[[NSAttributedString alloc] initWithString:@"tiếng"] selectedRange:NSMakeRange(0,5) replacementRange:NSMakeRange(0,3)];
  require(value.selectedRange.length==5,"IME candidate selection lost");
  [value keyDown:key(123)]; [value keyUp:key(123,0,true)];
  require(value->interpretations==1 && value->events.empty(),"candidate navigation reached host instead of IME");
  [value insertText:@"tiếng e\u0301 😀" replacementRange:NSMakeRange(0,5)]; [value unmarkText];
  require(value->commits==std::vector<std::string>{"tiếng e\xcc\x81 😀"} && !value.hasMarkedText && value->events.empty(),"committed composition lost Unicode or was sent twice");
  marked(value,@"日本語"); [value unmarkText]; [value unmarkText];
  require(value->commits.size()==2 && value->commits.back()=="日本語","unmark did not commit exactly once");
  marked(value,@"đừng gửi"); [value doCommandBySelector:@selector(cancelOperation:)]; [value unmarkText];
  require(!value.hasMarkedText && value->commits.size()==2 && value->events.empty(),"cancel unexpectedly committed or sent remote Escape");
  marked(value,@"😀"); NSRange actual=unspecified;
  const auto result=[value attributedSubstringForProposedRange:NSMakeRange(1,1) actualRange:&actual];
  require([result.string isEqualToString:@"😀"] && actual.location==0 && actual.length==2,"IME context query split surrogate pair");
  require([value attributedSubstringForProposedRange:NSMakeRange(NSUIntegerMax-1,8) actualRange:&actual]==nil && actual.location==NSNotFound,"invalid context range overflowed");
  [value cancelComposition];
  // Replacing committed host text is unsupported: never guess a backspace
  // count, especially for graphemes or a host selection we cannot observe.
  [value insertText:@"replacement" replacementRange:NSMakeRange(0,1)];
  require(value->commits.size()==2 && value->events.empty(),"remote reconversion generated an unverified edit");
}
void testPermissionsAndFailures() {
  auto* value=view(); [value toggleTextMode]; value->events.clear();
  marked(value,@"old peer draft"); value.epoch=2; [value unmarkText];
  require(value->commits.empty() && !value.hasMarkedText,"draft survived peer generation change");
  marked(value,@"permission draft"); value.inputEnabled=NO; [value unmarkText];
  require(value->commits.empty() && !value.hasMarkedText,"input permission loss committed draft");
  value.inputEnabled=YES; marked(value,@"focus draft"); value.released=YES; [value unmarkText];
  require(value->commits.empty() && !value.hasMarkedText,"focus loss committed draft");
  value.released=NO; marked(value,@"rejected"); value.acceptText=NO;
  [value insertText:@"rejected" replacementRange:unspecified]; [value unmarkText];
  value.acceptText=YES; [value insertText:@"automatic retry" replacementRange:unspecified];
  require(value.hasMarkedText && value->commits.empty(),"rejected text automatically replayed");
  [value doCommandBySelector:@selector(cancelOperation:)];
  [value insertText:@"recovered" replacementRange:unspecified];
  require(value->commits==std::vector<std::string>{"recovered"},"explicit discard failed to unblock new text");
  [value insertText:@"" replacementRange:unspecified];
  require(!value.hasMarkedText && value->commits.size()==1,"empty composition generated input");
  [value insertText:[@"😀" stringByPaddingToLength:258 withString:@"😀" startingAtIndex:0] replacementRange:unspecified];
  require(value->commits.size()==1,"oversized composition truncated or partially typed");
  [value cancelComposition];
  const unichar invalid[]={0xd800}; NSString* invalidString=[NSString stringWithCharacters:invalid length:1];
  [value insertText:invalidString replacementRange:unspecified];
  require(value->commits.size()==1,"unpaired surrogate committed");
  [value cancelComposition]; [value setMarkedText:@"safe" selectedRange:NSMakeRange(NSUIntegerMax-1,8) replacementRange:unspecified];
  [value unmarkText];
  require(value->commits.size()==1 && value.selectedRange.length==0,"invalid marked selection committed or overflowed");
  value.inputEnabled=NO; [value toggleTextMode]; [value insertText:@"read-only" replacementRange:unspecified];
  require(value->commits.size()==1 && value->events.empty(),"disabled/view-only input received text");
}
void testNativeControlsAndFocus() {
  auto* value=view(); value.inputEnabled=NO;
  NSWindow* window=[[NSWindow alloc] initWithContentRect:NSMakeRect(0,0,640,360) styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
  window.releasedWhenClosed=NO; window.contentView=value;
  [value installTextControls];
  require(window.titlebarAccessoryViewControllers.count==0,"view-only installed an input control");
  value.inputEnabled=YES; value.supportsText=NO; [value installTextControls];
  require(window.titlebarAccessoryViewControllers.count==1,"text action missing from native title bar");
  NSButton* button=(NSButton*)window.titlebarAccessoryViewControllers.firstObject.view.subviews.firstObject;
  require(!button.enabled && button.refusesFirstResponder,"unsupported action enabled or steals IME focus");
  value.supportsText=YES; [value refreshTextControls]; [button performClick:nil];
  require(value.textMode && [button.title isEqualToString:@"Physical keys"],"native action did not toggle text mode");
  value->events.clear(); marked(value,@"focus draft"); [value resignFirstResponder]; [value unmarkText];
  require(value.released && !value.hasMarkedText && value->commits.empty() && value->events.size()==1 && value->events[0].kind==Input::Reset,"resigning first responder committed text or left keys held");
  value.inputEnabled=NO; [window close];
}
}
int main() {
  @autoreleasepool {
    @try {
      try {
        [NSApplication sharedApplication];
        auto* resetView=[[SanserSnv2RemoteView alloc] initWithFrame:NSMakeRect(0,0,640,360)];
        __block unsigned resetHooks=0;resetView.willResetInput=^{++resetHooks;};
        [resetView sendInput:Input{Input::Key,0,0,4,0,true}];require(resetHooks==0,"ordinary key stopped analog");
        [resetView sendInput:Input{}];require(resetHooks==1,"input Reset did not stop analog while peer unavailable");
        resetView.willResetInput=nil;[resetView sendInput:Input{}];require(resetHooks==1,"detached reset callback retained analog model");
        testModeAndPhysicalKeys(); testComposition(); testPermissionsAndFailures(); testNativeControlsAndFocus();
        std::cout<<"AppKit IME: physical default, candidate/selection, Unicode commit, shortcuts, cancellation, epoch and failure fences verified\n"; return 0;
      } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
    } @catch(NSException* error) { std::cerr<<"AppKit IME exception: "<<error.name.UTF8String<<'\n'; return 1; }
  }
}
