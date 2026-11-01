#pragma once
#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <deque>
#include <optional>

namespace sanser::desktop {
// Only authenticated, same-clock RTT samples enter this policy. Application
// queue delay is useful congestion evidence even when no UDP packet is lost.
class DelayRatePolicy {
public:
  using Time=std::chrono::steady_clock;
  std::optional<std::uint32_t> observe(double rtt,double baseline,std::uint32_t current,
      std::uint32_t ceiling,Time::time_point now) {
    if(!std::isfinite(rtt) || !std::isfinite(baseline) || baseline<=0 ||
       rtt<baseline || rtt>3000 || ceiling<1000000) return {};
    // Bursty queues drain between peaks. A good sample must not erase recent
    // congestion, but isolated old spikes must not accumulate indefinitely.
    while(!elevated_.empty() && now-elevated_.front()>std::chrono::seconds(1)) elevated_.pop_front();
    const auto delay=rtt-baseline;
    if(delay<=std::max(12.0,baseline*0.5)) return {};
    quietUntil_=now+std::chrono::seconds(3);
    elevated_.push_back(now);
    if(elevated_.size()>4) elevated_.pop_front();
    if(elevated_.size()<2 || (reduced_ && now-lastReduction_<std::chrono::milliseconds(500))) return {};
    const auto floor=std::min(ceiling,1000000U);
    const auto target=std::clamp(static_cast<std::uint32_t>(current*(delay>=80 ? 0.60:0.85)),floor,ceiling);
    reduced_=true;lastReduction_=now;elevated_.clear();
    return target<current ? std::optional<std::uint32_t>{target}:std::nullopt;
  }
  bool recovering(Time::time_point now) const { return now<quietUntil_; }
private:
  std::deque<Time::time_point> elevated_;
  bool reduced_=false;
  Time::time_point quietUntil_{},lastReduction_{};
};
}
