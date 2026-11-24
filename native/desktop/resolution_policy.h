#pragma once
#include <algorithm>
#include <chrono>
#include <cstdint>
#include <deque>
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
    // A size forced down by dropped captures comes back more cautiously: the
    // same picture size starved the sender moments ago.
    std::chrono::milliseconds restoreAfterStarved{20000};
    std::chrono::milliseconds minimumDwell{8000};
  };
  // 0 = requested size, 1 = 2/3 per side (1080p -> 720p), 2 = 1/2 (-> 540p).
  static constexpr unsigned maxLevel=2;
  ResolutionPolicy() = default;
  explicit ResolutionPolicy(Config config):config_(config) {}

  // `starved`: most screen captures were refused because the paced sender
  // still held earlier pictures, i.e. the frame rate collapses although the
  // rate controller has not lowered the bitrate yet. Returns true when any
  // reduced size should be used; level() says which.
  bool update(std::uint32_t targetBps,Clock::time_point now,bool starved=false) {
    const bool down=level_<maxLevel && (starved || (level_==0 && targetBps<config_.reduceBelowBps));
    const bool up=level_>0 && !starved && targetBps>=config_.restoreAtBps;
    const int direction=down ? -1 : up ? 1 : 0;
    if(direction==0) { pendingSince_.reset(); pending_=0; return reduced(); }
    if(direction!=pending_) { pendingSince_=now; pending_=direction; }
    const auto hold=direction<0 ? config_.reduceAfter : starvedReduction_ ? config_.restoreAfterStarved : config_.restoreAfter;
    const bool dwelled=!switchedAt_ || now-*switchedAt_>=config_.minimumDwell;
    if(now-*pendingSince_>=hold && dwelled) {
      if(direction<0) { ++level_; starvedReduction_=starved; } else { --level_; if(level_==0) starvedReduction_=false; }
      switchedAt_=now; pendingSince_.reset(); pending_=0;
    }
    return reduced();
  }
  bool reduced() const { return level_>0; }
  unsigned level() const { return level_; }

  static std::uint32_t reducedDimension(std::uint32_t full) { return dimension(full,1); }
  static std::uint32_t dimension(std::uint32_t full,unsigned level) {
    if(level==0) return full;
    const auto scaled=level==1 ? full*2U/3U : full/2U;
    return std::max<std::uint32_t>(64U,scaled&~1U);
  }

private:
  Config config_{};
  unsigned level_=0;
  int pending_=0;
  bool starvedReduction_=false;
  std::optional<Clock::time_point> pendingSince_,switchedAt_;
};

/// Share of offered screen captures the sender refused over the last second.
/// Counts only real screen changes, so an idle desktop is never "starved".
class CaptureAdmission {
public:
  using Clock=std::chrono::steady_clock;
  // Fed periodically with running totals of offered and admitted captures.
  bool starved(std::uint64_t offered,std::uint64_t admitted,Clock::time_point now) {
    samples_.push_back({now,offered,admitted});
    while(samples_.size()>1 && now-samples_.front().at>std::chrono::milliseconds(1000)) samples_.pop_front();
    const auto& first=samples_.front();
    if(now-first.at<std::chrono::milliseconds(800)) return false;
    const auto offers=offered-first.offered,admits=admitted-first.admitted;
    // At least 20 changes per second (motion), and fewer than 60% sent.
    return offers>=20 && static_cast<double>(admits)<offers*0.6;
  }
  void reset() { samples_.clear(); }
private:
  struct Sample { Clock::time_point at; std::uint64_t offered,admitted; };
  std::deque<Sample> samples_;
};

} // namespace sanser::desktop
