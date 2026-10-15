#pragma once
#include "transport.h"
#include <deque>
#include <stdexcept>
#include <utility>

namespace sanser::desktop {
// The socket reader never runs a hardware/software decoder or pixel conversion.
// Encoded P-frames are dependent: overflow discards the chain and requests an
// IDR rather than pretending an arbitrary newest P-frame can be decoded alone.
class DecodeWorker {
public:
  DecodeWorker(std::function<void(Frame)> decode,std::function<void()> reset,std::function<void()> keyframe,bool coldStart=false)
      :decode_(std::move(decode)),reset_(std::move(reset)),keyframe_(std::move(keyframe)),coldStart_(coldStart),warming_(coldStart),worker_([this] { loop(); }) {}
  ~DecodeWorker() { stop(); }
  void submit(Frame frame) {
    bool request=false;
    {
      std::lock_guard lock(mutex_);
      if(stopping_) return;
      // MF initialization can outlast three frame intervals. Preserve a bounded
      // reference chain during this one-time cold start, then switch to the
      // small steady-state queue only after the decoder catches up completely.
      if(queue_.size()>=(warming_ ? 30U : 3U) || bytes_+frame.data.size()>8*1024*1024) {
        queue_.clear(); bytes_=0; waiting_=true; resetPending_=true; warming_=coldStart_; ++dropped_;
      }
      if(waiting_ && !frame.keyframe) request=true;
      else {
        if(frame.keyframe) waiting_=false;
        bytes_+=frame.data.size(); queue_.push_back(std::move(frame));
      }
    }
    wake_.notify_one();
    if(request) keyframe_();
  }
  void stop() {
    { std::lock_guard lock(mutex_); stopping_=true; queue_.clear(); bytes_=0; }
    wake_.notify_all();
    if(worker_.joinable()) worker_.join();
  }
  unsigned failures() const { return failures_.load(); }
  std::uint64_t dropped() const { return dropped_.load(); }
  std::string error() const { std::lock_guard lock(mutex_); return error_; }
private:
  void loop() {
    while(true) {
      Frame frame; bool reset=false;
      {
        std::unique_lock lock(mutex_);
        wake_.wait(lock,[&] { return stopping_ || resetPending_ || !queue_.empty(); });
        if(stopping_) break;
        reset=std::exchange(resetPending_,false);
        if(!queue_.empty()) { frame=std::move(queue_.front()); queue_.pop_front(); bytes_-=frame.data.size(); }
      }
      try {
        if(reset) reset_();
        if(!frame.data.empty()) {
          decode_(std::move(frame)); failures_=0;
          std::lock_guard lock(mutex_); if(queue_.empty()) warming_=false;
        }
      } catch(const std::exception& error) {
        ++failures_;
        {
          std::lock_guard lock(mutex_);
          error_=error.what(); queue_.clear(); bytes_=0; waiting_=true; warming_=coldStart_;
        }
        reset_(); keyframe_();
      }
    }
    // Decoder COM/VideoToolbox objects are released on their owning thread.
    reset_();
  }
  std::function<void(Frame)> decode_;
  std::function<void()> reset_,keyframe_;
  mutable std::mutex mutex_;
  std::condition_variable wake_;
  std::deque<Frame> queue_;
  std::size_t bytes_=0;
  bool coldStart_=false;
  bool stopping_=false, waiting_=true, resetPending_=false, warming_=false;
  std::atomic<unsigned> failures_{0};
  std::atomic<std::uint64_t> dropped_{0};
  std::string error_;
  std::thread worker_;
};
}
