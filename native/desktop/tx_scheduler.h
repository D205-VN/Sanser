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
  enum Lane : std::size_t { Control, InputLane, Audio, Video, Repair, Count };
  struct Packet {
    std::vector<std::uint8_t> bytes;
    std::chrono::steady_clock::time_point queued;
    std::uint64_t frame = 0;
    bool repair = false;
    bool audio = false;
    std::chrono::steady_clock::time_point deadline{};
    std::uint64_t inputSequence = 0;
    bool input = false;
  };
  bool push(Lane lane, Packet packet) {
    packet.repair=lane==Repair;
    packet.audio=lane==Audio;
    // Audio is disposable. Drop the oldest block, never terminate a keyboard
    // session or accumulate seconds of sound when the network is congested.
    if(lane==Audio && queues_[lane].size()>=8) (void)take(lane);
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
    if (videoDue && !queues_[Repair].empty()) return take(Repair);
    return std::nullopt;
  }
  bool hasVideo() const { return !queues_[Video].empty() || !queues_[Repair].empty(); }
  std::size_t nextVideoBytes() const { return !queues_[Video].empty() ? queues_[Video].front().bytes.size() : !queues_[Repair].empty() ? queues_[Repair].front().bytes.size() : 0; }
  std::size_t queuedVideoBytes() const { return bytes_[Video]; }
  bool hasPriority() const {
    return latestMove_ || !queues_[Control].empty() || !queues_[InputLane].empty() || !queues_[Audio].empty();
  }
  void clearRealtimeVideo() { queues_[Video].clear(); bytes_[Video]=0; }
  void clearVideo() { clearRealtimeVideo(); queues_[Repair].clear(); bytes_[Repair]=0; }
  void clearInput() { queues_[InputLane].clear(); bytes_[InputLane]=0; latestMove_.reset(); }
  void acknowledgeInput(std::uint64_t contiguous,std::uint64_t mask) {
    auto& lane=queues_[InputLane];
    for(auto it=lane.begin();it!=lane.end();) {
      const auto sequence=it->inputSequence;
      const bool received=sequence && (sequence<=contiguous ||
        (sequence>contiguous && sequence-contiguous<=64 && (mask & (std::uint64_t{1}<<(sequence-contiguous-1)))));
      if(received) { bytes_[InputLane]-=it->bytes.size(); it=lane.erase(it); } else ++it;
    }
  }
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
