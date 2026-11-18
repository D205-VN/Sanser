#pragma once
#include "privacy_screen_policy.h"
#include <atomic>
#include <memory>
#include <string>
#include <thread>

namespace sanser::desktop {
// Opaque black, click-through, topmost window over the whole virtual screen,
// excluded from capture with WDA_EXCLUDEFROMCAPTURE (Windows 10 2004+). It is
// owned by a private UI thread of the host engine process, so Windows removes
// it when the engine exits or crashes. If capture exclusion is unavailable the
// overlay is never shown.
//
// Local escape: Ctrl+Alt+Shift+P dismisses the overlay for the rest of the
// session. When local input is blocked, Ctrl+Alt+Del first (Windows always
// cancels BlockInput on the secure attention sequence), then the hotkey.
class WindowsPrivacyScreen {
public:
  WindowsPrivacyScreen(bool enabled,bool blockInput);
  ~WindowsPrivacyScreen();
  WindowsPrivacyScreen(const WindowsPrivacyScreen&)=delete;
  WindowsPrivacyScreen& operator=(const WindowsPrivacyScreen&)=delete;
  // Any thread; cheap. Applies on the overlay thread.
  void update(bool sessionReady);
private:
  void run();
  bool enabled_=false,blockInput_=false;
  std::atomic<bool> ready_{false},stop_{false};
  std::atomic<unsigned long> thread_{0};
  std::thread worker_;
};
}
