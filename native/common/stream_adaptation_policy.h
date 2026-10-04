#pragma once

#include <cstdint>

namespace sanser {

// Windows are the host's one-second statistics intervals, not individual
// feedback messages. Repair/render/network reports can describe the same loss.
class StreamAdaptationPolicy {
public:
  double networkBitrateFactor(std::uint64_t window, double lossPercent,
                             bool failedRepairs, bool sendOverBudget = false) {
    // Jitter, wall-clock frame age, and a high but stable RTT are not proof of
    // congestion. Keep quality unless loss/repair failure/send work persists.
    if (window == 0 || !(lossPercent >= 1.0 || failedRepairs || sendOverBudget)) return 1.0;
    if (window < lastPressureWindow_) return 1.0;
    if (window != lastPressureWindow_) {
      pressureWindows_ = lastPressureWindow_ != 0 && window - lastPressureWindow_ == 1
        ? pressureWindows_ + 1 : 1;
      if (pressureWindows_ > 2) pressureWindows_ = 2;
      lastPressureWindow_ = window;
    }
    if (pressureWindows_ < 2 || bitrateReducedWindow_ == window) return 1.0;
    bitrateReducedWindow_ = window;
    return lossPercent >= 4.0 ? 0.84 : 0.88;
  }

  double processingFpsFactor(std::uint64_t window, bool severe) {
    if (window == 0 || window <= fpsReducedWindow_) return 1.0;
    fpsReducedWindow_ = window;
    return severe ? 0.85 : 0.95;
  }

  bool reducedIn(std::uint64_t window) const {
    return window != 0 && (bitrateReducedWindow_ == window || fpsReducedWindow_ == window);
  }

  bool bitrateReducedIn(std::uint64_t window) const {
    return window != 0 && bitrateReducedWindow_ == window;
  }

private:
  std::uint64_t lastPressureWindow_ = 0;
  unsigned pressureWindows_ = 0;
  std::uint64_t bitrateReducedWindow_ = 0;
  std::uint64_t fpsReducedWindow_ = 0;
};

} // namespace sanser
