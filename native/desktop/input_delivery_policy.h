#pragma once
#include <algorithm>
#include <chrono>
#include <cstdint>

namespace sanser::desktop {
// ACK latency includes retries and application scheduling; it is a conservative
// delivery bound, not a measurement labelled as network/wire RTT.
class InputDeliveryPolicy {
public:
  void acknowledged(std::chrono::milliseconds elapsed) {
    const auto sample=static_cast<std::uint32_t>(std::clamp<std::int64_t>(elapsed.count(),1,5000));
    latencyMs_=latencyMs_==0 ? sample : sample>latencyMs_ ? sample : (latencyMs_*7+sample)/8;
  }
  std::chrono::milliseconds retry(bool ultra,unsigned attempts) const {
    const auto base=latencyMs_ ? std::clamp(latencyMs_*3/2+4,12U,500U) : ultra ? 40U : 80U;
    return std::chrono::milliseconds(std::min(1000U,base*(1U<<std::min(4U,attempts>0 ? attempts-1 : 0))));
  }
  std::chrono::milliseconds deadline() const {
    return std::chrono::milliseconds(std::clamp(latencyMs_*8+250,2000U,5000U));
  }
  std::chrono::milliseconds repairRetry() const {
    // There may be no keyboard/button ACK before the first video picture.
    // The first NACK is independent of this retry interval and remains fast.
    return std::chrono::milliseconds(latencyMs_ ? std::clamp(latencyMs_*3/2+4,20U,200U) : 100U);
  }
private:
  std::uint32_t latencyMs_=0;
};
}
