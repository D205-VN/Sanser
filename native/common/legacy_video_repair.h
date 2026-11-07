#pragma once

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <span>
#include <utility>
#include <vector>

namespace sanser::video {

// Optional extension to video-nack. packetIds remains the whole-frame
// fallback for old peers, absent assemblies and excessively fragmented loss.
inline constexpr std::size_t maxNackPackets = 8;
inline constexpr std::size_t maxNackRanges = 12;
struct FragmentRange {
  std::uint64_t packetId;
  std::uint32_t first;
  std::uint32_t last; // Inclusive.
};

inline std::vector<FragmentRange> decodeMissingRanges(
    std::span<const std::uint64_t> flat,
    std::span<const std::uint64_t> packetIds) {
  std::vector<FragmentRange> ranges;
  if (flat.size() % 3 || flat.size() > maxNackRanges * 3) return ranges;
  for (std::size_t i = 0; i < flat.size(); i += 3) {
    if (!flat[i] || flat[i + 1] > flat[i + 2] ||
        flat[i + 2] >= std::numeric_limits<std::uint16_t>::max() ||
        std::find(packetIds.begin(), packetIds.end(), flat[i]) == packetIds.end()) {
      return {}; // Never interpret a partial/invalid hint as all missing data.
    }
    ranges.push_back({flat[i], static_cast<std::uint32_t>(flat[i + 1]),
                      static_cast<std::uint32_t>(flat[i + 2])});
  }
  return ranges;
}

// Add all holes for this packet or none. A truncated hole list would leave a
// viewer stalled while the host incorrectly considered its repair complete.
inline bool appendMissingRanges(std::uint64_t packetId,
                                std::span<const std::uint8_t> received,
                                std::vector<FragmentRange>& output) {
  if (!packetId || received.empty() ||
      received.size() > std::numeric_limits<std::uint16_t>::max()) return false;
  const auto original = output.size();
  for (std::size_t i = 0; i < received.size();) {
    if (received[i]) { ++i; continue; }
    const auto first = i++;
    while (i < received.size() && !received[i]) ++i;
    if (output.size() >= maxNackRanges) {
      output.resize(original);
      return false;
    }
    output.push_back({packetId, static_cast<std::uint32_t>(first),
                      static_cast<std::uint32_t>(i - 1)});
  }
  return output.size() != original;
}

class RepairCursor {
public:
  RepairCursor() = default;
  RepairCursor(std::uint64_t packetId, std::span<const FragmentRange> hints) {
    for (const auto& range : hints) {
      if (range.packetId == packetId) ranges_.push_back(range);
    }
    std::sort(ranges_.begin(), ranges_.end(), [](const auto& a, const auto& b) {
      return a.first < b.first;
    });
    std::vector<FragmentRange> merged;
    for (const auto& range : ranges_) {
      if (!merged.empty() && range.first <= merged.back().last + 1) {
        merged.back().last = std::max(merged.back().last, range.last);
      } else merged.push_back(range);
    }
    ranges_ = std::move(merged);
    if (!ranges_.empty()) next_ = ranges_.front().first;
  }
  std::uint32_t first() const { return next_; }
  bool selective() const { return !ranges_.empty(); }
  std::uint32_t chunkSize(std::uint32_t budget) const {
    if (done_) return 0;
    return selective() ? std::min(budget, ranges_[range_].last - next_ + 1) : budget;
  }
  void advance(std::uint32_t sent, bool frameEnd) {
    if (done_) return;
    if (frameEnd) { done_ = true; return; }
    next_ += std::min(sent, chunkSize(sent));
    if (selective() && next_ > ranges_[range_].last) {
      if (++range_ == ranges_.size()) done_ = true;
      else next_ = ranges_[range_].first;
    }
  }
  bool complete() const { return done_; }
private:
  std::vector<FragmentRange> ranges_;
  std::size_t range_ = 0;
  std::uint32_t next_ = 0;
  bool done_ = false;
};

} // namespace sanser::video
