#pragma once
#import "metal_presenter.h"
#include "transport.h"
#include <set>

// SNV2 viewer only. Physical HID mode remains the default for game input.
// Text mode uses the controller's AppKit IME; the host receives committed text,
// never a guessed sequence of backspaces or a remote selection replacement.
@interface SanserSnv2RemoteView : SanserSnv2MetalView <NSTextInputClient> {
  BOOL _textMode, _textBlocked, _toggleDown, _discarding;
  NSMutableAttributedString* _marked;
  NSRange _selection;
  NSTextField* _textHint;
  NSButton* _textButton;
  NSPoint _candidateAnchor;
  std::uint32_t _compositionGeneration;
  std::set<std::uint16_t> _textKeys, _textModifiers;
}
@property(nonatomic,assign) sanser::desktop::Peer* peer;
@property(nonatomic,assign) BOOL inputEnabled;
@property(nonatomic,assign) BOOL released;
@property(nonatomic,readonly) BOOL textMode;
@property(nonatomic,copy) void (^willResetInput)(void);
- (void)sendInput:(sanser::desktop::Input)input;
- (void)pointer:(NSEvent*)event kind:(sanser::desktop::Input::Kind)kind down:(BOOL)down;
- (BOOL)unicodeAvailable;
- (std::uint32_t)textGeneration;
- (BOOL)validateTextSession;
- (BOOL)commitText:(NSString*)value;
- (void)toggleTextMode;
- (void)installTextControls;
- (void)refreshTextControls;
- (void)cancelComposition;
- (void)suspendTextInput;
@end
