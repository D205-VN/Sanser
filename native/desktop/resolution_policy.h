#pragma once
#include <algorithm>
#include <chrono>
#include <cstdint>
#include <optional>

namespace sanser::desktop {

// Chooses between the requested capture size and a reduced one (2/3 per side,
// e.g. 1920x1080 -> 1280x720) for a bandwidth-limited (mobile) viewer.
//
// At a few Mbps, a full-size picture of fast motion makes every frame so large
// that the paced sender admits only 15-20 captures per second. A smaller
// picture at the same bitrate keeps the frame rate instead. Hysteresis and a
// minimum dwell keep a jittery path from flapping, since every switch costs a
// keyframe.
class ResolutionPolicy {
public:
  using Clock=std::chrono::steady_clock;
  struct Config {
    std::uint32_t reduceBelowBps=4'000'000;
    std::uint32_t restoreAtBps=5'400'000;
    std::chrono::milliseconds reduceAfter{1500};
    std::chrono::milliseconds restoreAfter{6000};
    std::chrono::milliseconds minimumDwell{8000};
  };
  ResolutionPolicy() = default;
  explicit ResolutionPolicy(Config config):config_(config) {}

  // Returns true when the reduced size should be used.
  bool update(std::uint32_t targetBps,Clock::time_point now) {
    const bool wantsChange=reduced_ ? targetBps>=config_.restoreAtBps : targetBps<config_.reduceBelowBps;
    if(!wantsChange) { pendingSince_.reset(); return reduced_; }
    if(!pendingSince_) pendingSince_=now;
    const auto hold=reduced_ ? config_.restoreAfter : config_.reduceAfter;
    const bool dwelled=!switchedAt_ || now-*switchedAt_>=config_.minimumDwell;
    if(now-*pendingSince_>=hold && dwelled) { reduced_=!reduced_; switchedAt_=now; pendingSince_.reset(); }
    return reduced_;
  }
  bool reduced() const { return reduced_; }

  static std::uint32_t reducedDimension(std::uint32_t full) {
    return std::max<std::uint32_t>(64U,(full*2U/3U)&~1U);
  }

private:
  Config config_{};
  bool reduced_=false;
  std::optional<Clock::time_point> pendingSince_,switchedAt_;
};

} // namespace sanser::desktop
