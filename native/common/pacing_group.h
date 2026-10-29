#pragma once
#include <algorithm>
#include <cstdint>

namespace sanser {
// Batch timer wakeups, not UDP payloads: each datagram remains independently
// authenticated and the caller retains its per-datagram priority/socket lock.
class PacingGroup {
public:
  bool waitRequired(bool ultra,std::int64_t aheadMicros) const {
    if(aheadMicros<=0) return false;
    return !ultra || packets_>=maxPackets || aheadMicros>maxAdvanceMicros;
  }
  void waited() { packets_=0; }
  void datagram() { packets_=std::min(maxPackets,packets_+1); }
  static constexpr unsigned maxPackets=4;
  static constexpr std::int64_t maxAdvanceMicros=500;
private:
  unsigned packets_=0;
};
}
