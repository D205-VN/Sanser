#pragma once
#include "control_cutoff_policy.h"
#include "input.h"
#include <memory>

namespace sanser::desktop {
// Control+Command+K at the host Mac turns remote input off or back on for
// this session (see control_cutoff_policy.h) and shows a short notice. Main thread.
class MacControlCutoff {
public:
  explicit MacControlCutoff(bool input);
  ~MacControlCutoff();
  MacControlCutoff(const MacControlCutoff&)=delete;
  MacControlCutoff& operator=(const MacControlCutoff&)=delete;
  // Once per host loop iteration: delivers the hotkey. True when this call
  // turned control off (the caller then releases everything still held).
  bool update();
  // Any thread: false when a remote event must be dropped.
  static bool allow(const Input& event);
private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};
// Carbon hot key IDs ('SNPV' 1 is the privacy screen).
inline constexpr unsigned controlCutoffHotKeySignature='SNCK';
}
