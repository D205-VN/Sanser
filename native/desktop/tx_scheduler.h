#pragma once
#include <array>
#include <chrono>
#include <cstdint>
#include <deque>
#include <optional>
#include <vector>

namespace sanser::desktop {
// Called under Peer::sendMutex_. Each pop is one datagram, never a whole frame.
// Reserve independent lanes so video capacity cannot evict input/control/audio.
class TxScheduler {
public:
  enum Lane : std::size_t { Control, InputLane, Audio, Video, Count };
  struct Packet {
    std::vector<std::uint8_t> bytes;
    std::chrono::steady_clock::time_point queued;
    std::uint64_t frame = 0;
  };
  bool push(Lane lane, Packet packet) {
    const std::size_t limit = lane == Video ? 5 * 1024 * 1024 : 256 * 1024;
    if (bytes_[lane] + packet.bytes.size() > limit) return false;
    bytes_[lane] += packet.bytes.size();
    queues_[lane].push_back(std::move(packet));
    return true;
  }
  void latestInput(Packet packet) {
    // Mouse moves occupy their own slot; reliable keys/buttons stay ordered.
    latestMove_ = std::move(packet);
  }
  std::optional<Packet> pop(bool videoDue) {
    for (std::size_t lane = Control; lane < Video; ++lane) {
      if (!queues_[lane].empty()) return take(lane);
      if (lane == InputLane && latestMove_) {
        auto packet = std::move(latestMove_); latestMove_.reset(); return packet;
      }
    }
    if (videoDue && !queues_[Video].empty()) return take(Video);
    return std::nullopt;
  }
  bool hasVideo() const { return !queues_[Video].empty(); }
  std::size_t nextVideoBytes() const { return hasVideo() ? queues_[Video].front().bytes.size() : 0; }
  bool hasPriority() const {
    return latestMove_ || !queues_[Control].empty() || !queues_[InputLane].empty() || !queues_[Audio].empty();
  }
  void clearVideo() { queues_[Video].clear(); bytes_[Video] = 0; }
private:
  Packet take(std::size_t lane) {
    auto value = std::move(queues_[lane].front()); queues_[lane].pop_front();
    bytes_[lane] -= value.bytes.size(); return value;
  }
  std::array<std::deque<Packet>, Count> queues_;
  std::array<std::size_t, Count> bytes_{};
  std::optional<Packet> latestMove_;
};
}
