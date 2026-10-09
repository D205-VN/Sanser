#pragma once
#include <cstdint>
#include <map>
#include <mutex>
#include <optional>
#include <string>

namespace sanser {
struct ReceivedControl {
  std::string payload;
  std::uint64_t receivedMicros = 0;
};

// Each subtraction stays on one machine's monotonic clock. Residual RTT includes
// kernel queues and receiver scheduling BEFORE recv(); it is NOT one-way delay.
struct ControlTiming {
  std::uint64_t t0=0, t1=0, t2=0, t3=0, t4=0, t5=0, t6=0;
  bool input = false;
  std::uint64_t hostSendLockMicros = 0, hostSendCallMicros = 0;
  double appMs=0, socketMs=0, residualMs=0, sendQueueMs=0, hostQueueMs=0,
         hostWorkMs=0, receiveQueueMs=0;
  bool calculate() {
    if (!t0 || !t1 || !t2 || !t3 || !t4 || !t5 || !t6 ||
        t1 < t0 || t5 < t1 || t6 < t5 || t3 < t2 || t4 < t3) return false;
    const auto socket = t5-t1, host = t4-t2;
    // Reject stale/corrupt/mismatched records, never clamp an impossible sample.
    if (t6-t0 > 10000000 || host > socket || hostSendLockMicros > t4-t3 ||
        hostSendCallMicros > 10000000) return false;
    appMs = (t6-t0)/1000.0; socketMs = socket/1000.0;
    residualMs = (socket-host)/1000.0;
    sendQueueMs = (t1-t0)/1000.0; hostQueueMs = (t3-t2)/1000.0;
    hostWorkMs = (t4-t3)/1000.0; receiveQueueMs = (t6-t5)/1000.0;
    return true;
  }
};

// A fresh ID per send attempt prevents a retry ACK being paired with a later send.
// Host send timestamps arrive in a separate authenticated receipt, AFTER send().
class ControlTimingTracker {
  std::mutex mutex_;
  std::map<std::uint64_t, ControlTiming> pending_;
  std::uint64_t next_ = 0;
  std::optional<ControlTiming> finish(std::uint64_t id) {
    auto it = pending_.find(id);
    if (it == pending_.end() || !it->second.t4 || !it->second.t6) return {};
    auto value = it->second;
    pending_.erase(it);
    if (!value.calculate()) return {};
    return value;
  }
public:
  std::uint64_t prepare(std::uint64_t queued, bool input) {
    std::lock_guard<std::mutex> lock(mutex_);
    while (pending_.size() >= 128) pending_.erase(pending_.begin());
    const auto id = ++next_;
    pending_[id].t0 = queued; pending_[id].input = input;
    return id;
  }
  template <typename Clock>
  void sending(std::uint64_t id, Clock clock) {
    std::lock_guard<std::mutex> lock(mutex_);
    auto it = pending_.find(id); if (it != pending_.end()) it->second.t1 = clock();
  }
  void sent(std::uint64_t id, std::uint64_t at) {
    std::lock_guard<std::mutex> lock(mutex_);
    auto it = pending_.find(id); if (it != pending_.end()) it->second.t1 = at;
  }
  std::optional<ControlTiming> received(std::uint64_t id, std::uint64_t t5, std::uint64_t t6) {
    std::lock_guard<std::mutex> lock(mutex_);
    auto it = pending_.find(id); if (it == pending_.end()) return {};
    if (it->second.t6) return {}; // Duplicate replies must not replace first arrival.
    it->second.t5 = t5; it->second.t6 = t6;
    return finish(id);
  }
  std::optional<ControlTiming> receipt(std::uint64_t id, std::uint64_t t2,
                                      std::uint64_t t3, std::uint64_t t4,
                                      std::uint64_t sendLock = 0, std::uint64_t sendCall = 0) {
    std::lock_guard<std::mutex> lock(mutex_);
    auto it = pending_.find(id); if (it == pending_.end()) return {};
    if (it->second.t4) return {};
    it->second.t2=t2; it->second.t3=t3; it->second.t4=t4;
    it->second.hostSendLockMicros=sendLock; it->second.hostSendCallMicros=sendCall;
    return finish(id);
  }
};
} // namespace sanser
