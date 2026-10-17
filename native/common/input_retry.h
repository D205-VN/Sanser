#pragma once
#include <algorithm>
#include <chrono>
#include <cmath>

namespace sanser {
// Delivery RTT includes application queues. Never label this estimate wire RTT.
// One bounded retry rule for Legacy and SNV2; only reliable input uses backoff.
inline std::chrono::milliseconds inputRetryDelay(double rttMs, double variationMs=0,
                                                unsigned attempts=1,
                                                std::chrono::milliseconds fallback=std::chrono::milliseconds(40)) {
  const double base=std::isfinite(rttMs) && rttMs>0 && rttMs<=5000
    ? std::clamp(std::max(rttMs*1.5+2, rttMs+4*std::max(0.0,variationMs)+2),8.0,500.0)
    : std::clamp(static_cast<double>(fallback.count()),8.0,500.0);
  const auto multiplier=1U<<std::min(4U,attempts>0 ? attempts-1 : 0);
  return std::chrono::milliseconds(static_cast<unsigned>(std::ceil(std::min(1000.0,base*multiplier))));
}
}
