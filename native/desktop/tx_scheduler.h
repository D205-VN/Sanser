#pragma once
#include <algorithm>
#include <array>
#include <chrono>
#include <cstdint>
#include <deque>
#include <optional>
#include <atomic>
#include <memory>
#include <vector>

namespace sanser::desktop {
// Called under Peer::sendMutex_. Each pop is one datagram, never a whole frame.
// Reserve independent lanes so video capacity cannot evict input/control/audio.
class TxScheduler {
public:
  enum Lane : std::size_t { Control, InputLane, Voice, Audio, Video, Repair, UserData, Count };
  struct Packet {
    std::vector<std::uint8_t> bytes;
    std::chrono::steady_clock::time_point queued;
    std::uint64_t frame = 0;
    bool repair = false;
    bool audio = false;
    std::chrono::steady_clock::time_point deadline{};
    std::uint64_t inputSequence = 0;
    bool input = false;
    std::shared_ptr<std::atomic<std::uint32_t>> originalSent;
    std::uint32_t originalOrdinal=0;
    std::uint64_t userMessage=0;
    std::uint16_t userFragment=0;
    bool userData=false;
    bool gamepad=false;
    std::size_t originalVideoPayloadBytes=0;
  };
  bool push(Lane lane, Packet packet) {
    packet.repair=lane==Repair;
    packet.audio=lane==Audio || lane==Voice;
    packet.userData=lane==UserData;
    if(lane==UserData) {
      // Retrying before a paced datagram was sent must not append duplicates.
      for(const auto& queued:queues_[lane])
        if(queued.userMessage==packet.userMessage && queued.userFragment==packet.userFragment) return true;
    }
    // Audio is disposable. Drop the oldest block, never terminate a keyboard
    // session or accumulate seconds of sound when the network is congested.
    if(lane==Audio && queues_[lane].size()>=8) (void)take(lane);
    if(lane==Voice && queues_[lane].size()>=8) (void)take(lane);
    const std::size_t limit = lane == UserData ? 16*1024 : lane == Video ? 5 * 1024 * 1024 : 256 * 1024;
    if (bytes_[lane] + packet.bytes.size() > limit) return false;
    bytes_[lane] += packet.bytes.size();
    queues_[lane].push_back(std::move(packet));
    return true;
  }
  void latestInput(Packet packet) {
    // Mouse moves occupy their own slot; reliable keys/buttons stay ordered.
    latestMove_ = std::move(packet);
  }
  void latestGamepad(Packet packet) {packet.gamepad=true;latestGamepad_=std::move(packet);}
  // A UI shortcut is one bounded transaction. Publish none of its key edges
  // when the whole down/up sequence (or panel-entry reset) cannot fit. Called under the
  // same send mutex as normal input; never lets mouse/video split its edges.
  bool pushInputBatch(std::vector<Packet> packets) {
    if(packets.empty() || packets.size()>10)return false;
    constexpr std::size_t limit=256*1024;
    auto bytes=bytes_[InputLane];
    for(const auto& packet:packets) {
      if(packet.bytes.size()>limit-bytes)return false;
      bytes+=packet.bytes.size();
    }
    auto next=queues_[InputLane];
    for(auto& packet:packets) {packet.input=true;next.push_back(std::move(packet));}
    queues_[InputLane].swap(next);bytes_[InputLane]=bytes;return true;
  }
  std::optional<Packet> pop(bool videoDue,bool dataDue=false) {
    for (std::size_t lane = Control; lane < Video; ++lane) {
      if (!queues_[lane].empty()) return take(lane);
      if (lane == InputLane && latestMove_ && (!latestGamepad_ || !gamepadNext_)) {
        auto packet = std::move(latestMove_); latestMove_.reset();gamepadNext_=true;return packet;
      }
      if(lane==InputLane && latestGamepad_) {
        auto packet=std::move(latestGamepad_);latestGamepad_.reset();gamepadNext_=false;return packet;
      }
    }
    if(dataDue && !queues_[UserData].empty() && (!videoDue || !hasVideo() || videoSinceData_>=32)) {
      videoSinceData_=0; return take(UserData);
    }
    if (videoDue && hasVideo()) {
      const auto lane=nextVideoLane();
      if(lane==Repair) freshSinceRepair_=0;
      else freshSinceRepair_=std::min(8U,freshSinceRepair_+1);
      videoSinceData_=std::min(32U,videoSinceData_+1);
      return take(lane);
    }
    return std::nullopt;
  }
  bool hasVideo() const { return !queues_[Video].empty() || !queues_[Repair].empty(); }
  std::size_t nextVideoBytes() const { return hasVideo() ? queues_[nextVideoLane()].front().bytes.size() : 0; }
  std::size_t queuedVideoBytes() const { return bytes_[Video]; }
  bool hasUserData() const { return !queues_[UserData].empty(); }
  std::size_t nextUserBytes() const { return hasUserData() ? queues_[UserData].front().bytes.size():0; }
  void clearUserData() { queues_[UserData].clear(); bytes_[UserData]=0; }
  bool hasPriority() const {
    return latestMove_ || latestGamepad_ || !queues_[Control].empty() || !queues_[InputLane].empty() || !queues_[Voice].empty() || !queues_[Audio].empty();
  }
  void clearRealtimeVideo() { queues_[Video].clear(); bytes_[Video]=0; }
  void clearVideo() { clearRealtimeVideo(); queues_[Repair].clear(); bytes_[Repair]=0; freshSinceRepair_=0; }
  void clearInput() { queues_[InputLane].clear(); bytes_[InputLane]=0; latestMove_.reset();latestGamepad_.reset(); }
  void clearVoice() { queues_[Voice].clear(); bytes_[Voice]=0; }
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
  Lane nextVideoLane() const {
    // A complete reference picture can be missing one fragment while its
    // dependent pictures keep the realtime lane continuously busy. Give repair
    // one paced slot after at most eight fresh datagrams, preserving all higher
    // priority lanes and the existing shared wire budget.
    return !queues_[Repair].empty() && (queues_[Video].empty() || freshSinceRepair_>=8) ? Repair : Video;
  }
  Packet take(std::size_t lane) {
    auto value = std::move(queues_[lane].front()); queues_[lane].pop_front();
    bytes_[lane] -= value.bytes.size(); return value;
  }
  std::array<std::deque<Packet>, Count> queues_;
  std::array<std::size_t, Count> bytes_{};
  std::optional<Packet> latestMove_;
  std::optional<Packet> latestGamepad_;
  bool gamepadNext_=false;
  unsigned freshSinceRepair_=0;
  unsigned videoSinceData_=0;
};
}
