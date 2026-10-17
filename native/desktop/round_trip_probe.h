#pragma once
#include <chrono>
#include <cstdint>
#include <map>
#include <optional>

namespace sanser::desktop {
// Application RTT includes both TX queues, authentication and peer processing.
// It is deliberately not called wire RTT or one-way network latency.
class RoundTripProbe {
public:
  using Time=std::chrono::steady_clock;
  std::uint64_t sent(Time::time_point now) {
    while(!pending_.empty() && (pending_.size()>=4 || now-pending_.begin()->second>std::chrono::seconds(3))) pending_.erase(pending_.begin());
    const auto id=++sequence_; pending_.emplace(id,now); return id;
  }
  std::optional<double> received(std::uint64_t id,Time::time_point now) {
    const auto it=pending_.find(id); if(it==pending_.end()) return {};
    const auto elapsed=now-it->second; pending_.erase(it);
    if(elapsed<Time::duration::zero() || elapsed>std::chrono::seconds(3)) return {};
    return std::chrono::duration<double,std::milli>(elapsed).count();
  }
  void reset() { pending_.clear(); } // Never reuse IDs after a peer generation change.
private:
  std::uint64_t sequence_=0;
  std::map<std::uint64_t,Time::time_point> pending_;
};
}
