#pragma once
#include <algorithm>
#include <cstdint>

namespace sanser {
// Deadlines are local monotonic durations, never cross-machine wall clocks.
struct LatencyPolicy {
  bool ultra = false;
  std::uint64_t baseDelay(std::uint64_t duration) const {
    return ultra ? 0 : std::clamp<std::uint64_t>(duration / 2, 6000, 18000);
  }
  std::uint64_t adaptiveLimit(std::uint64_t duration) const {
    return ultra ? 4000 : std::clamp<std::uint64_t>(duration * 2, 12000, 24000);
  }
  std::uint64_t presentAt(std::uint64_t arrival, std::uint64_t timeline,
                          std::uint64_t adaptive) const {
    // A stale or future sender timeline must not grow the gaming render queue.
    return ultra ? arrival + std::min<std::uint64_t>(adaptive, 4000) : timeline;
  }
  std::uint64_t jitterHoldMs() const { return ultra ? 12 : 70; }
  std::uint64_t repairDeadlineMs() const { return ultra ? 12 : 120; }
};
}
