#pragma once
#include "control_cutoff_policy.h"
#include "input.h"
#include <memory>

namespace sanser::desktop {
// Control+Command+K at the host Mac ends remote input for the rest of
// the session (see control_cutoff_policy.h). Main thread.
class MacControlCutoff {
public:
  explicit MacControlCutoff(bool input);
  ~MacControlCutoff();
  MacControlCutoff(const MacControlCutoff&)=delete;
  MacControlCutoff& operator=(const MacControlCutoff&)=delete;
  // Once per host loop iteration: delivers the hotkey.
  void update();
  // Any thread: false when a remote event must be dropped.
  static bool allow(const Input& event);
private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};
// Carbon hot key IDs ('SNPV' 1 is the privacy screen).
inline constexpr unsigned controlCutoffHotKeySignature='SNCK';
}
