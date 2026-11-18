#pragma once
#import <Cocoa/Cocoa.h>
#include "privacy_screen_policy.h"
#include <memory>

namespace sanser::desktop {
// Opaque black, click-through windows covering every host screen while a
// one-to-one control session is ready. The windows are created on screen with
// zero alpha before ScreenCaptureKit reads shareable content, so capture can
// exclude them by window ID. They belong to the host engine process: when the
// engine exits or crashes, WindowServer removes them.
//
// Local escape: Control+Option+Command+P dismisses the overlay for the rest of
// the session. Blocking local input is not offered on macOS.
class MacPrivacyScreen {
public:
  explicit MacPrivacyScreen(bool enabled);
  ~MacPrivacyScreen();
  MacPrivacyScreen(const MacPrivacyScreen&)=delete;
  MacPrivacyScreen& operator=(const MacPrivacyScreen&)=delete;
  bool enabled() const { return state_.enabled; }
  // CGWindowIDs to exclude from capture; empty when disabled.
  NSArray<NSNumber*>* windowIds() const;
  void setCaptureExcluded(bool excluded);
  // Main thread, once per host loop iteration.
  void update(bool sessionReady);
private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
  privacy::State state_;
  std::string marker_;
};
}
