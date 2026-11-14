#pragma once
#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <deque>
#include <optional>

namespace sanser::desktop {
// Receiver feedback and the sender's pacer do not share window boundaries.
// Compare original payloads actually sent, over several feedback windows;
// encoded bytes still waiting in the local pacer are not network loss.
class VideoDeliveryWindow {
public:
  struct Sample {
    std::uint64_t sent=0,delivered=0,packets=0,missing=0,elapsedMs=0;
    bool ready() const { return elapsedMs>=1000; }
    bool busy() const { return sent>=32000; }
    bool shortfall() const { return ready() && busy() && static_cast<double>(delivered)<sent*0.85; }
    bool loss() const { return packets>=16 && static_cast<double>(missing)/packets>0.03; }
  };
  Sample observe(Sample sample) {
    samples_.push_back(sample);
    while(samples_.size()>4) samples_.pop_front();
    Sample total;
    for(const auto& value:samples_) {
      total.sent+=value.sent;total.delivered+=value.delivered;
      total.packets+=value.packets;total.missing+=value.missing;total.elapsedMs+=value.elapsedMs;
    }
    return total;
  }
  void reset() { samples_.clear(); }
private:
  std::deque<Sample> samples_;
};

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
    while(!elevated_.empty() && now-elevated_.front().at>std::chrono::seconds(1)) elevated_.pop_front();
    const auto delay=rtt-baseline;
    if(delay<=std::max(12.0,baseline*0.5)) return {};
    // A large spike is a queue even if it drains between bursts: two within a
    // second are enough. A moderate rise on its own is ordinary Wi-Fi/power-
    // save jitter (probes arrive every 250 ms): it must persist across most of
    // the second before it reduces the rate or holds recovery.
    const bool severe=delay>=std::max(40.0,baseline*2.0);
    elevated_.push_back({now,severe});
    if(elevated_.size()>4) elevated_.pop_front();
    const auto severeCount=std::count_if(elevated_.begin(),elevated_.end(),[](const Sample& sample){return sample.severe;});
    // Three of the last four 250 ms probes, not every other one.
    const auto recent=std::count_if(elevated_.begin(),elevated_.end(),[&](const Sample& sample){return now-sample.at<=std::chrono::milliseconds(800);});
    const bool sustained=severeCount>=2 || recent>=3;
    if(severe || sustained) quietUntil_=now+std::chrono::seconds(3);
    if(!sustained || (reduced_ && now-lastReduction_<std::chrono::milliseconds(500))) return {};
    const auto floor=std::min(ceiling,1000000U);
    const auto target=std::clamp(static_cast<std::uint32_t>(current*(delay>=80 ? 0.60:0.85)),floor,ceiling);
    reduced_=true;lastReduction_=now;elevated_.clear();
    return target<current ? std::optional<std::uint32_t>{target}:std::nullopt;
  }
  bool recovering(Time::time_point now) const { return now<quietUntil_; }
private:
  struct Sample { Time::time_point at; bool severe; };
  std::deque<Sample> elevated_;
  bool reduced_=false;
  Time::time_point quietUntil_{},lastReduction_{};
};
}
