#pragma once
#include <condition_variable>
#include <functional>
#include <mutex>
#include <optional>
#include <thread>
#include <utility>

namespace sanser::desktop {
// Decoded pictures are independent. Keep one waiting picture and do expensive
// CPU conversion off the reference decoder, dropping old decoded pictures
// without resetting H.264 or asking every viewer for another keyframe.
template<class T> class LatestFrameWorker {
public:
  explicit LatestFrameWorker(std::function<void(T)> work):work_(std::move(work)),worker_([this] { loop(); }) {}
  ~LatestFrameWorker() { stop(); }
  void submit(T frame) { { std::lock_guard lock(mutex_); if(stopping_) return; pending_=std::move(frame); } wake_.notify_one(); }
  void clear() { std::lock_guard lock(mutex_); pending_.reset(); }
  void stop() { { std::lock_guard lock(mutex_); stopping_=true; pending_.reset(); } wake_.notify_one(); if(worker_.joinable()) worker_.join(); }
private:
  void loop() {
    for(;;) {
      std::optional<T> frame;
      { std::unique_lock lock(mutex_); wake_.wait(lock,[&] { return stopping_ || pending_; }); if(stopping_) return; frame=std::move(pending_); pending_.reset(); }
      work_(std::move(*frame));
    }
  }
  std::function<void(T)> work_;
  std::mutex mutex_; std::condition_variable wake_; std::optional<T> pending_; bool stopping_=false;
  std::thread worker_;
};
}
