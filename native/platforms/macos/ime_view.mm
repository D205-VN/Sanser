#import "ime_view.h"
#include "keyboard.h"
#include "text_input.h"
#include <algorithm>

using namespace sanser::desktop;
namespace {
bool inside(NSRange range,NSUInteger size) {
  return range.location!=NSNotFound && range.location<=size && range.length<=size-range.location;
}
NSString* plainText(id value) {
  if([value isKindOfClass:NSAttributedString.class]) return [value string];
  return [value isKindOfClass:NSString.class] ? value:nil;
}
std::optional<std::string> strictText(NSString* value) {
  if(!value || !value.length || value.length>sanser::maxTextInputUnits) return std::nullopt;
  NSData* bytes=[value dataUsingEncoding:NSUTF8StringEncoding allowLossyConversion:NO];
  if(!bytes || bytes.length>sanser::maxTextInputBytes) return std::nullopt;
  std::string text(static_cast<const char*>(bytes.bytes),bytes.length);
  return sanser::textInputUtf16(text) ? std::optional<std::string>{std::move(text)}:std::nullopt;
}
bool navigation(std::uint16_t hid) {
  return (hid>=40 && hid<=43) || (hid>=58 && hid<=82) || hid==88;
}
}

@implementation SanserSnv2RemoteView
@synthesize inputEnabled=_inputEnabled, released=_released;
- (BOOL)acceptsFirstResponder { return self.inputEnabled; }
- (BOOL)textMode { return _textMode; }
- (BOOL)unicodeAvailable { return self.inputEnabled && !self.released && self.window.isKeyWindow && NSApp.active && self.peer && self.peer->inputAvailable() && self.peer->textAvailable(); }
- (std::uint32_t)textGeneration { return self.peer ? self.peer->remoteGeneration():0; }
- (BOOL)validateTextSession {
  if(![self unicodeAvailable] || (_compositionGeneration && _compositionGeneration!=[self textGeneration])) {
    if(_marked || _textBlocked || !_textKeys.empty() || !_textModifiers.empty()) [self suspendTextInput];
    return NO;
  }
  return YES;
}
- (BOOL)commitText:(NSString*)value {
  const auto text=strictText(value);
  return text && [self unicodeAvailable] && self.peer->text(*text,_compositionGeneration ? _compositionGeneration:[self textGeneration]);
}
- (NSTextInputContext*)inputContext { return _textMode && self.inputEnabled && !self.released ? [super inputContext]:nil; }
- (void)setInputEnabled:(BOOL)value { _inputEnabled=value; if(!value) [self suspendTextInput]; }
- (void)setReleased:(BOOL)value { _released=value; if(value) [self suspendTextInput]; }
- (void)sendInput:(Input)input { if(input.kind==Input::Reset && self.willResetInput)self.willResetInput();if(self.inputEnabled && self.peer) self.peer->input(input); }
- (void)showTextHint {
  if(!_textHint && _textMode) {
    _textHint=[NSTextField labelWithString:@""];
    _textHint.font=[NSFont systemFontOfSize:13]; _textHint.textColor=NSColor.whiteColor;
    _textHint.drawsBackground=YES; _textHint.backgroundColor=[NSColor colorWithWhite:0.06 alpha:0.92];
    _textHint.lineBreakMode=NSLineBreakByTruncatingTail; _textHint.maximumNumberOfLines=2;
    _textHint.accessibilityLabel=@"Remote text input";
    [self addSubview:_textHint];
  }
  _textHint.hidden=!_textMode;
  if(!_textMode) return;
  _textHint.frame=NSMakeRect(16,self.isFlipped ? 16 : std::max(0.0,self.bounds.size.height-70),std::max(0.0,std::min(620.0,self.bounds.size.width-32)),54);
  NSString* heading=_textBlocked ? @"Text not sent · Esc discards this draft; Ctrl+Option+I returns to physical keys" : @"Text input · Ctrl+Option+I returns to physical keys";
  NSMutableAttributedString* label=[[NSMutableAttributedString alloc] initWithString:[heading stringByAppendingString:@"\n"]];
  if(_marked.length) {
    NSMutableAttributedString* preview=[_marked mutableCopy];
    [preview addAttribute:NSUnderlineStyleAttributeName value:@(NSUnderlineStyleSingle) range:NSMakeRange(0,preview.length)];
    if(inside(_selection,preview.length) && _selection.length)
      [preview addAttribute:NSBackgroundColorAttributeName value:NSColor.selectedTextBackgroundColor range:_selection];
    [label appendAttributedString:preview];
  } else [label appendAttributedString:[[NSAttributedString alloc] initWithString:@"Use your Mac input method; committed text is typed on the host."]];
  _textHint.attributedStringValue=label;
}
- (void)setFrameSize:(NSSize)size { [super setFrameSize:size]; [self showTextHint]; [self.inputContext invalidateCharacterCoordinates]; }
- (BOOL)resignFirstResponder {
  if(self.inputEnabled && !self.released) { [self sendInput:Input{}]; self.released=YES; }
  [self suspendTextInput]; return [super resignFirstResponder];
}
- (void)cancelComposition {
  // Clear first: discardMarkedText may synchronously call unmarkText. A cancel
  // must never turn into a commit when focus, permission or the session changes.
  _marked=nil; _selection=NSMakeRange(0,0); _textBlocked=NO; _compositionGeneration=0;
  if(_textMode && !_discarding) {
    _discarding=YES;
    @try { [[super inputContext] discardMarkedText]; }
    @finally { _discarding=NO; }
  }
  [self showTextHint];
}
- (void)suspendTextInput { [self cancelComposition]; _textKeys.clear(); _textModifiers.clear(); }
- (void)installTextControls {
  if(_textButton || !self.inputEnabled || !self.window) return;
  auto* accessory=[[NSTitlebarAccessoryViewController alloc] init];
  accessory.layoutAttribute=NSLayoutAttributeRight;
  accessory.view=[[NSView alloc] initWithFrame:NSMakeRect(0,0,116,28)];
  _textButton=[NSButton buttonWithTitle:@"Text input" target:self action:@selector(toggleTextModeFromButton:)];
  _textButton.frame=NSMakeRect(4,2,108,24); _textButton.bezelStyle=NSBezelStyleRounded;
  _textButton.refusesFirstResponder=YES;
  _textButton.toolTip=@"Use the Mac input method for text; switch back for games. Control+Option+I";
  [accessory.view addSubview:_textButton]; [self.window addTitlebarAccessoryViewController:accessory];
  [self refreshTextControls];
}
- (void)refreshTextControls {
  if(!_textButton) return;
  const BOOL available=[self unicodeAvailable];
  if(_textButton.enabled!=available) _textButton.enabled=available;
  NSString* title=_textMode ? @"Physical keys":@"Text input";
  if(![_textButton.title isEqualToString:title]) _textButton.title=title;
}
- (void)toggleTextModeFromButton:(id)sender { (void)sender; [self toggleTextMode]; }
- (void)toggleTextMode {
  if(!self.inputEnabled || self.released) return;
  if(!_textMode && ![self unicodeAvailable]) {
    self.window.title=@"Sanser · Text input unavailable · Waiting for a Unicode-capable host"; return;
  }
  [self sendInput:Input{}]; [self suspendTextInput];
  if(_textMode) [[super inputContext] deactivate];
  _textMode=!_textMode;
  if(_textMode) [self.inputContext activate];
  self.window.title=_textMode ? @"Sanser · Text input · Ctrl+Option+I: physical keys · Ctrl+Option+Escape: release" : @"Sanser · Remote desktop · Ctrl+Option+I: text input · Ctrl+Option+Escape: release";
  [self showTextHint]; [self refreshTextControls];
}
- (void)syncTextModifiers:(NSEventModifierFlags)flags {
  for(const auto& [code,mask]:std::initializer_list<std::pair<std::uint16_t,NSEventModifierFlags>>{
      {224,NSEventModifierFlagControl},{225,NSEventModifierFlagShift},{226,NSEventModifierFlagOption},{227,NSEventModifierFlagCommand}}) {
    const bool down=(flags&mask)!=0;
    if(down!=_textModifiers.contains(code)) {
      [self sendInput:Input{Input::Key,0,0,code,0,down}];
      if(down) _textModifiers.insert(code); else _textModifiers.erase(code);
    }
  }
}
- (void)tapTextKey:(std::uint16_t)code {
  if(![self unicodeAvailable] || _textBlocked) return;
  [self syncTextModifiers:0];
  [self sendInput:Input{Input::Key,0,0,code,0,true}]; [self sendInput:Input{Input::Key,0,0,code,0,false}];
}
- (void)pointer:(NSEvent*)event kind:(Input::Kind)kind down:(BOOL)down {
  if(!self.inputEnabled || self.released || self.imageSize.width<=0 || self.imageSize.height<=0 || self.bounds.size.width<=0 || self.bounds.size.height<=0) return;
  auto point=[self convertPoint:event.locationInWindow fromView:nil]; const auto bounds=self.bounds;
  const auto size=self.imageSize; const auto scale=std::min(bounds.size.width/size.width,bounds.size.height/size.height);
  const auto width=size.width*scale,height=size.height*scale; point.x-=(bounds.size.width-width)/2; point.y-=(bounds.size.height-height)/2;
  const double y=std::clamp(point.y/height,0.0,1.0);
  Input input; input.kind=kind; input.x=static_cast<std::uint16_t>(std::clamp(point.x/width,0.0,1.0)*65535); input.y=static_cast<std::uint16_t>((self.isFlipped ? y : 1-y)*65535); input.down=down;
  if(kind==Input::Scroll) {
    if(event.type!=NSEventTypeScrollWheel) return;
    input.delta=static_cast<std::int16_t>(std::clamp(event.scrollingDeltaY,-120.0,120.0));
  } else if(kind==Input::Button) {
    switch(event.type) {
      case NSEventTypeLeftMouseDown: case NSEventTypeLeftMouseUp: input.code=0; break;
      case NSEventTypeRightMouseDown: case NSEventTypeRightMouseUp: input.code=1; break;
      case NSEventTypeOtherMouseDown: case NSEventTypeOtherMouseUp:
        if(event.buttonNumber!=2) return;
        input.code=2; break;
      default: return;
    }
    if(_textMode) [self syncTextModifiers:event.modifierFlags];
  }
  [self sendInput:input];
}
- (void)mouseMoved:(NSEvent*)event { [self pointer:event kind:Input::Move down:NO]; }
- (void)mouseDragged:(NSEvent*)event { [self pointer:event kind:Input::Move down:NO]; }
- (void)rightMouseDragged:(NSEvent*)event { [self pointer:event kind:Input::Move down:NO]; }
- (void)otherMouseDragged:(NSEvent*)event { [self pointer:event kind:Input::Move down:NO]; }
- (void)mouseDown:(NSEvent*)event {
  if(!self.inputEnabled) return;
  [self cancelComposition];
  if(self.released) { self.released=NO; self.window.title=@"Sanser · Remote desktop · Ctrl+Option+I: text input · Ctrl+Option+Escape: release"; }
  _candidateAnchor=[self convertPoint:event.locationInWindow fromView:nil];
  [self.window makeFirstResponder:self]; [self pointer:event kind:Input::Button down:YES];
}
- (void)mouseUp:(NSEvent*)event { [self pointer:event kind:Input::Button down:NO]; }
- (void)rightMouseDown:(NSEvent*)event { [self cancelComposition]; [self pointer:event kind:Input::Button down:YES]; }
- (void)rightMouseUp:(NSEvent*)event { [self pointer:event kind:Input::Button down:NO]; }
- (void)otherMouseDown:(NSEvent*)event { [self cancelComposition]; [self pointer:event kind:Input::Button down:YES]; }
- (void)otherMouseUp:(NSEvent*)event { [self pointer:event kind:Input::Button down:NO]; }
- (void)scrollWheel:(NSEvent*)event { [self pointer:event kind:Input::Scroll down:NO]; }
- (void)keyDown:(NSEvent*)event {
  if(!self.inputEnabled) return;
  const auto flags=event.modifierFlags;
  if(event.keyCode==53 && (flags&NSEventModifierFlagControl) && (flags&NSEventModifierFlagOption)) {
    self.released=YES; [self sendInput:Input{}]; self.window.title=@"Sanser · Input released · Click the stream to resume"; return;
  }
  if(self.released) return;
  if(event.keyCode==34 && (flags&NSEventModifierFlagControl) && (flags&NSEventModifierFlagOption) && !(flags&NSEventModifierFlagCommand)) {
    _toggleDown=YES; if(!event.isARepeat) [self toggleTextMode]; return;
  }
  const auto hid=hidFromMac(event.keyCode);
  if(!_textMode) { if(hid) [self sendInput:Input{Input::Key,0,0,hid,0,true}]; return; }
  if(![self validateTextSession]) return;
  if(_textBlocked) { if(event.keyCode==53) [self cancelComposition]; return; }
  if([self hasMarkedText]) { [self interpretKeyEvents:@[event]]; return; }
  if((flags&(NSEventModifierFlagControl|NSEventModifierFlagCommand)) || navigation(hid)) {
    if(hid) { [self syncTextModifiers:flags]; _textKeys.insert(hid); [self sendInput:Input{Input::Key,0,0,hid,0,true}]; } return;
  }
  [self syncTextModifiers:0]; [self interpretKeyEvents:@[event]];
}
- (void)keyUp:(NSEvent*)event {
  if(event.keyCode==34 && _toggleDown) { _toggleDown=NO; return; }
  if(!self.inputEnabled || self.released) return;
  const auto hid=hidFromMac(event.keyCode); if(!hid) return;
  if(!_textMode || _textKeys.erase(hid)) [self sendInput:Input{Input::Key,0,0,hid,0,false}];
}
- (void)flagsChanged:(NSEvent*)event {
  if(!self.inputEnabled || self.released) return;
  if(_textMode) {
    [self syncTextModifiers:(event.modifierFlags&(NSEventModifierFlagControl|NSEventModifierFlagCommand)) && ![self hasMarkedText] && !_textBlocked ? event.modifierFlags:0]; return;
  }
  const auto hid=hidFromMac(event.keyCode); NSEventModifierFlags flag=0;
  if(hid==224||hid==228) flag=NSEventModifierFlagControl; if(hid==225||hid==229) flag=NSEventModifierFlagShift; if(hid==226||hid==230) flag=NSEventModifierFlagOption; if(hid==227||hid==231) flag=NSEventModifierFlagCommand;
  if(flag) [self sendInput:Input{Input::Key,0,0,hid,0,(event.modifierFlags&flag)!=0}];
}
- (BOOL)performKeyEquivalent:(NSEvent*)event {
  if(_textMode && self.inputEnabled && !self.released && event.type==NSEventTypeKeyDown && (event.modifierFlags&(NSEventModifierFlagControl|NSEventModifierFlagCommand))) { [self keyDown:event]; return YES; }
  return [super performKeyEquivalent:event];
}
- (BOOL)replacementIsLocal:(NSRange)range {
  return (range.location==NSNotFound && range.length==0) || ([self hasMarkedText] && range.location==0 && range.length==_marked.length);
}
- (void)rejectText:(NSString*)value {
  _textBlocked=YES;
  // Keep only a bounded local preview. Never serialize a rejected draft or its
  // content into diagnostics, and never retry an ambiguous committed edit.
  if(strictText(value)) _marked=[[NSMutableAttributedString alloc] initWithString:value];
  [self showTextHint];
}
- (void)insertText:(id)value replacementRange:(NSRange)replacement {
  if(!_textMode || _discarding || ![self validateTextSession] || _textBlocked) return;
  NSString* string=plainText(value);
  if(string && !string.length && [self replacementIsLocal:replacement]) { [self cancelComposition]; return; }
  if(![self replacementIsLocal:replacement] || !strictText(string) || ![self commitText:string]) { [self rejectText:string]; return; }
  // The text injector resets host-owned modifiers. Match that locally before
  // another physical shortcut; an IME-consumed key-up is never sent as HID.
  _textKeys.clear(); _textModifiers.clear(); _marked=nil; _selection=NSMakeRange(0,0); _compositionGeneration=0; [self showTextHint];
}
- (void)setMarkedText:(id)value selectedRange:(NSRange)selection replacementRange:(NSRange)replacement {
  if(!_textMode || _discarding || ![self validateTextSession] || _textBlocked) return;
  NSString* string=plainText(value);
  if(![self replacementIsLocal:replacement] || !string || (string.length && !strictText(string)) || !inside(selection,string.length)) { [self rejectText:string]; return; }
  _compositionGeneration=[self textGeneration];
  _marked=[[NSMutableAttributedString alloc] initWithString:string]; _selection=selection; [self showTextHint];
}
- (void)unmarkText {
  if([self hasMarkedText] && !_textBlocked) [self insertText:_marked.string replacementRange:NSMakeRange(NSNotFound,0)];
}
- (BOOL)hasMarkedText { return _marked.length>0; }
- (NSRange)markedRange { return [self hasMarkedText] ? NSMakeRange(0,_marked.length):NSMakeRange(NSNotFound,0); }
- (NSRange)selectedRange { return inside(_selection,_marked.length) ? _selection:NSMakeRange(0,0); }
- (NSArray<NSAttributedStringKey>*)validAttributesForMarkedText { return @[]; }
- (NSAttributedString*)attributedSubstringForProposedRange:(NSRange)range actualRange:(NSRangePointer)actual {
  if(actual) *actual=NSMakeRange(NSNotFound,0);
  if(![self hasMarkedText] || !inside(range,_marked.length)) return nil;
  if(range.length) range=[_marked.string rangeOfComposedCharacterSequencesForRange:range];
  if(actual) *actual=range; return [_marked attributedSubstringFromRange:range];
}
- (NSRect)firstRectForCharacterRange:(NSRange)range actualRange:(NSRangePointer)actual {
  if(actual) *actual=inside(range,_marked.length) ? range:self.selectedRange;
  NSPoint point=NSMakePoint(std::clamp(_candidateAnchor.x,0.0,std::max(0.0,self.bounds.size.width-1)),std::clamp(_candidateAnchor.y,0.0,std::max(0.0,self.bounds.size.height-20)));
  return [self.window convertRectToScreen:[self convertRect:NSMakeRect(point.x,point.y,1,20) toView:nil]];
}
- (NSUInteger)characterIndexForPoint:(NSPoint)point { (void)point; return NSNotFound; }
- (void)doCommandBySelector:(SEL)selector {
  if(!_textMode || ![self unicodeAvailable]) return;
  if(selector==@selector(cancelOperation:)) { if([self hasMarkedText] || _textBlocked) [self cancelComposition]; else [self tapTextKey:41]; return; }
  if(_textBlocked) return;
  if([self hasMarkedText]) {
    if(selector==@selector(insertNewline:) || selector==@selector(insertTab:)) [self unmarkText];
    // Unsupported document edits must not become remote key presses while a
    // candidate is still being chosen. The local input method owns that edit.
    return;
  }
  std::uint16_t key=0;
  if(selector==@selector(insertNewline:)) key=40; else if(selector==@selector(insertTab:)) key=43;
  else if(selector==@selector(deleteBackward:)) key=42; else if(selector==@selector(deleteForward:)) key=76;
  else if(selector==@selector(moveLeft:)) key=80; else if(selector==@selector(moveRight:)) key=79;
  else if(selector==@selector(moveUp:)) key=82; else if(selector==@selector(moveDown:)) key=81;
  if(key) [self tapTextKey:key];
}
@end
