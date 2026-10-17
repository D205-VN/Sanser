#pragma once
#include "input.h"
#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <deque>
#include <functional>
#include <mutex>
#include <thread>
#include <utility>

namespace sanser::desktop {
// A slow OS injection call must not stop authenticated UDP receive/ACK/control.
// Exactly one worker owns injection, including reset on generation change/stop.
class InputWorker {
public:
  using Clock=std::chrono::steady_clock;
  struct Stats { std::uint64_t samples=0, queueUs=0, workUs=0, queueMaxUs=0, workMaxUs=0, coalescedMoves=0; };
  InputWorker(std::function<void(Input)> apply,std::function<void()> failed):apply_(std::move(apply)),failed_(std::move(failed)),worker_([this] { loop(); }) {}
  ~InputWorker() { stop(); }
  InputWorker(const InputWorker&)=delete;
  InputWorker& operator=(const InputWorker&)=delete;
  bool submit(Input event) {
    std::lock_guard lock(mutex_);
    if(stopping_) return false;
    const Item item{event,Clock::now()};
    if(event.kind==Input::Reset) pending_.clear();
    // Coalesce only adjacent moves. Never move a drag across a button/key/reset.
    if(event.kind==Input::Move && !pending_.empty() && pending_.back().event.kind==Input::Move) {
      pending_.back()=item; ++stats_.coalescedMoves;
    } else {
      if(pending_.size()>=256) {
        const auto move=std::find_if(pending_.begin(),pending_.end(),[](const Item& value) { return value.event.kind==Input::Move; });
        if(move!=pending_.end()) pending_.erase(move);
        else return event.kind==Input::Move; // Disposable position, never silently lose key-up.
      }
      pending_.push_back(item);
    }
    wake_.notify_one(); return true;
  }
  Stats takeStats() { std::lock_guard lock(mutex_); auto result=stats_; stats_={}; return result; }
  void stop() {
    { std::lock_guard lock(mutex_); stopping_=true; pending_.clear(); }
    wake_.notify_all();
    if(worker_.joinable()) worker_.join();
  }
private:
  struct Item { Input event; Clock::time_point queued; };
  void loop() {
    try {
      for(;;) {
        Item item;
        { std::unique_lock lock(mutex_); wake_.wait(lock,[&] { return stopping_ || !pending_.empty(); });
          if(stopping_) break; item=pending_.front(); pending_.pop_front(); }
        const auto began=Clock::now(); apply_(item.event);
        const auto finished=Clock::now();
        const auto queue=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(began-item.queued).count());
        const auto work=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(finished-began).count());
        { std::lock_guard lock(mutex_); ++stats_.samples; stats_.queueUs+=queue; stats_.workUs+=work;
          stats_.queueMaxUs=std::max(stats_.queueMaxUs,queue); stats_.workMaxUs=std::max(stats_.workMaxUs,work); }
      }
    } catch(...) {
      { std::lock_guard lock(mutex_); stopping_=true; pending_.clear(); }
      if(failed_) failed_();
    }
    // Do not replay queued presses during shutdown. Release the held state on
    // the same thread, even when the previous injection threw an exception.
    try { apply_(Input{}); } catch(...) { if(failed_) failed_(); }
  }
  std::function<void(Input)> apply_;
  std::function<void()> failed_;
  std::mutex mutex_; std::condition_variable wake_; std::deque<Item> pending_;
  Stats stats_{}; bool stopping_=false;
  std::thread worker_;
};
}
