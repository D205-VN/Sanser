#pragma once
#include <condition_variable>
#include <functional>
#include <mutex>
#include <optional>
#include <thread>
#include <utility>

namespace sanser::desktop {
// Raw captures and decoded pictures are independent. Keep one waiting picture
// while encoding/converting another. Never use this to replace encoded H.264
// P-frames: those retain dependencies on previous encoded pictures.
template<class T> class LatestFrameWorker {
public:
  explicit LatestFrameWorker(std::function<void(T)> work):work_(std::move(work)),worker_([this] { loop(); }) {}
  ~LatestFrameWorker() { stop(); }
  void submit(T frame) { { std::lock_guard lock(mutex_); if(stopping_) return; pending_=std::move(frame); } wake_.notify_all(); }
  void clear() { { std::lock_guard lock(mutex_); pending_.reset(); } wake_.notify_all(); }
  // A barrier for finite producers/tests; never call from the worker callback.
  void drain() { std::unique_lock lock(mutex_); wake_.wait(lock,[&] { return !pending_ && !working_; }); }
  void stop() { { std::lock_guard lock(mutex_); stopping_=true; pending_.reset(); } wake_.notify_all(); if(worker_.joinable()) worker_.join(); }
private:
  void loop() {
    for(;;) {
      std::optional<T> frame;
      { std::unique_lock lock(mutex_); wake_.wait(lock,[&] { return stopping_ || pending_; }); if(stopping_) return; frame=std::move(pending_); pending_.reset(); working_=true; }
      work_(std::move(*frame));
      { std::lock_guard lock(mutex_); working_=false; }
      wake_.notify_all();
    }
  }
  std::function<void(T)> work_;
  std::mutex mutex_; std::condition_variable wake_; std::optional<T> pending_; bool stopping_=false;
  bool working_=false;
  std::thread worker_;
};
}
