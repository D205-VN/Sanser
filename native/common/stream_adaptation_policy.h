#pragma once

#include <cstdint>
#include <algorithm>
#include <cmath>

namespace sanser {

// Windows are the host's one-second statistics intervals, not individual
// feedback messages. Repair/render/network reports can describe the same loss.
class StreamAdaptationPolicy {
public:
  // Delivery is unique authenticated video payload, measured before decode.
  // Offered rate disambiguates an idle desktop from a saturated network path.
  void observeDelivery(std::uint64_t window, double targetBps, double deliveredBps,
                       double offeredBps, double rttMs, double socketWaitMs) {
    if (window == 0 || window <= deliveryWindow_) return;
    const bool consecutive = deliveryWindow_ != 0 && window - deliveryWindow_ == 1;
    deliveryWindow_ = window;
    targetBps_ = std::isfinite(targetBps) ? std::max(0.0, targetBps) : 0;
    deliveryLimited_ = false;
    delayPressure_ = false;
    if (std::isfinite(rttMs) && rttMs >= 0 && rttMs < 5000) {
      // Periodically refresh the baseline so a route change cannot remain
      // classified as a permanently growing queue.
      if (minRttMs_ < 0 || window - baselineWindow_ >= 30) {
        minRttMs_ = rttMs; baselineWindow_ = window;
      }
      minRttMs_ = std::min(minRttMs_, rttMs);
      const double trend = previousRttMs_ < 0 || !consecutive ? 0 : rttMs - previousRttMs_;
      delayPressure_ = rttMs - minRttMs_ > std::max(8.0, minRttMs_ * 0.25) &&
        (trend > 2.0 || socketWaitMs > 2.0);
      previousRttMs_ = rttMs;
    }
    if (!std::isfinite(deliveredBps) || !std::isfinite(offeredBps) ||
        deliveredBps <= 0 || offeredBps <= 0 || targetBps_ <= 0) return;
    // App-limited samples do not set a capacity ceiling or drag it downward.
    if (offeredBps < targetBps_ * 0.65) return;
    const double sample = std::min(deliveredBps, offeredBps * 1.25);
    deliveryBps_ = deliveryBps_ <= 0 || !consecutive ? sample : 0.6 * sample + 0.4 * deliveryBps_;
    deliveryLimited_ = deliveredBps < offeredBps * 0.95;
  }

  double deliveryEstimateBps() const { return deliveryBps_; }

  double networkBitrateFactor(std::uint64_t window, double lossPercent,
                             bool failedRepairs, bool sendOverBudget = false) {
    // Jitter, wall-clock frame age, and a high but stable RTT are not proof of
    // congestion. Keep quality unless loss/repair failure/send work persists.
    const bool currentDelivery = window == deliveryWindow_;
    if (window == 0 || !(lossPercent >= 1.0 || failedRepairs || sendOverBudget ||
                        (currentDelivery && delayPressure_))) return 1.0;
    if (window < lastPressureWindow_) return 1.0;
    if (window != lastPressureWindow_) {
      pressureWindows_ = lastPressureWindow_ != 0 && window - lastPressureWindow_ == 1
        ? pressureWindows_ + 1 : 1;
      if (pressureWindows_ > 2) pressureWindows_ = 2;
      lastPressureWindow_ = window;
    }
    if (pressureWindows_ < 2 || bitrateReducedWindow_ == window) return 1.0;
    bitrateReducedWindow_ = window;
    if (currentDelivery && deliveryBps_ > 0 && targetBps_ > 0 &&
        (deliveryLimited_ || delayPressure_)) {
      // Drain the measured backlog with headroom, bounded to avoid a quality
      // collapse after one noisy throughput report. Never lower FPS here.
      return std::clamp(deliveryBps_ * 0.95 / targetBps_, 0.75, 0.95);
    }
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
  std::uint64_t deliveryWindow_ = 0, baselineWindow_ = 0;
  double targetBps_ = 0, deliveryBps_ = 0;
  double minRttMs_ = -1, previousRttMs_ = -1;
  bool deliveryLimited_ = false, delayPressure_ = false;
  std::uint64_t lastPressureWindow_ = 0;
  unsigned pressureWindows_ = 0;
  std::uint64_t bitrateReducedWindow_ = 0;
  std::uint64_t fpsReducedWindow_ = 0;
};

} // namespace sanser
