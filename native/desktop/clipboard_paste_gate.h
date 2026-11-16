#pragma once
#include "input.h"
#include <chrono>
#include <cstdint>
#include <mutex>
#include <vector>

namespace sanser::desktop {
// Windows viewer of a Mac host. Ctrl+V becomes Command+V, which pastes the
// Mac's own clipboard. Hold that chord (and any input after it) until the
// viewer's clipboard text has been written to the Mac, so the paste uses what
// the user just copied on Windows. A bounded timeout keeps typing responsive
// if the write never completes. Thread-safe: completion arrives on peer or
// clipboard worker threads.
class ClipboardPasteGate {
public:
  using Clock=std::chrono::steady_clock;
  static constexpr auto timeout=std::chrono::milliseconds(400);

  static bool startsPaste(const std::vector<Input>& batch,bool commandActive) {
    if(!commandActive) return false;
    for(const auto& event:batch) if(event.kind==Input::Key && event.down && event.code==25) return true;
    return false;
  }
  bool waiting() const { std::lock_guard lock(mutex_); return waiting_; }
  // Pass-through unless a paste is pending; then queue behind it in order.
  std::vector<Input> route(std::vector<Input> batch) {
    std::lock_guard lock(mutex_);
    if(!waiting_) return batch;
    held_.insert(held_.end(),batch.begin(),batch.end());
    return {};
  }
  std::uint64_t hold(std::vector<Input> batch,Clock::time_point now) {
    std::lock_guard lock(mutex_);
    held_.insert(held_.end(),batch.begin(),batch.end());
    waiting_=true; acked_=false; deadline_=now+timeout;
    return ++generation_;
  }
  void complete(std::uint64_t generation) {
    std::lock_guard lock(mutex_);
    if(generation==generation_) acked_=true;
  }
  std::vector<Input> release(Clock::time_point now) {
    std::lock_guard lock(mutex_);
    if(!waiting_ || (!acked_ && now<deadline_)) return {};
    waiting_=false; std::vector<Input> out; out.swap(held_); return out;
  }
private:
  mutable std::mutex mutex_;
  std::vector<Input> held_;
  bool waiting_=false,acked_=false;
  Clock::time_point deadline_{};
  std::uint64_t generation_=0;
};
}
