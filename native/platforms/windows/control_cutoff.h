#pragma once
#include "control_cutoff_policy.h"
#include <atomic>
#include <thread>

namespace sanser::desktop {
// Ctrl+Alt+K at the host PC turns remote input off or back on for this
// session (see control_cutoff_policy.h) and shows a short notice. The hot key lives on a private
// message thread of the host engine process. The SendInput gate in
// blocked_apps.cpp (windowsInputAllowed) enforces it for both Windows hosts.
class WindowsControlCutoff {
public:
  explicit WindowsControlCutoff(bool input);
  ~WindowsControlCutoff();
  WindowsControlCutoff(const WindowsControlCutoff&)=delete;
  WindowsControlCutoff& operator=(const WindowsControlCutoff&)=delete;
private:
  void run();
  std::atomic<unsigned long> thread_{0};
  std::atomic<bool> stop_{false};
  std::thread worker_;
};
}
