#pragma once
#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cmath>
#include "input_retry.h"

namespace sanser::desktop {
// ACK latency includes retries and application scheduling; it is a conservative
// delivery bound, not a measurement labelled as network/wire RTT.
class InputDeliveryPolicy {
public:
  void acknowledged(std::chrono::milliseconds elapsed) {
    const auto sample=static_cast<std::uint32_t>(std::clamp<std::int64_t>(elapsed.count(),1,5000));
    if(latencyMs_==0) latencyMs_=sample;
    else {
      variationMs_=(variationMs_*3+std::abs(static_cast<double>(sample)-latencyMs_))/4;
      latencyMs_=(latencyMs_*7+sample)/8;
    }
  }
  std::chrono::milliseconds retry(bool ultra,unsigned attempts) const {
    return sanser::inputRetryDelay(latencyMs_,variationMs_,attempts,std::chrono::milliseconds(ultra ? 40 : 80));
  }
  std::chrono::milliseconds deadline() const {
    return std::chrono::milliseconds(static_cast<unsigned>(std::clamp(latencyMs_*8+250,2000.0,5000.0)));
  }
  std::chrono::milliseconds repairRetry(double controlRttMs=-1) const {
    // There may be no keyboard/button ACK before the first video picture.
    // The first NACK is independent of this retry interval and remains fast.
    if(latencyMs_) return std::clamp(retry(false,1),std::chrono::milliseconds(20),std::chrono::milliseconds(200));
    // View-only and pointer-only sessions may never receive a reliable input
    // ACK. Use authenticated control RTT from the same socket in that case.
    if(std::isfinite(controlRttMs) && controlRttMs>0 && controlRttMs<=3000)
      return std::chrono::milliseconds(static_cast<unsigned>(std::clamp(std::ceil(controlRttMs*1.5+5),20.0,200.0)));
    return std::chrono::milliseconds(100);
  }
private:
  double latencyMs_=0, variationMs_=0;
};
}
