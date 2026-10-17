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
  using Now=std::function<Clock::time_point()>;
  DecodeWorker(std::function<void(Frame)> decode,std::function<void()> reset,std::function<void()> keyframe,bool coldStart=false,bool ultra=false,Now now=[] { return Clock::now(); })
      :decode_(std::move(decode)),reset_(std::move(reset)),keyframe_(std::move(keyframe)),now_(std::move(now)),coldStart_(coldStart),ultra_(ultra),warming_(coldStart),worker_([this] { loop(); }) {}
  ~DecodeWorker() { stop(); }
  void submit(Frame frame) {
    bool request=false;
    {
      std::lock_guard lock(mutex_);
      if(stopping_) return;
      // An IDR is a new reference chain. It can replace a waiting older chain
      // safely; arbitrary P-frames must never be treated as independent images.
      if(frame.keyframe && !queue_.empty()) {
        // The IDR itself resets references; do not recreate a healthy hardware
        // decoder on every GOP and reintroduce its cold-start cost.
        dropped_+=queue_.size(); queue_.clear(); bytes_=0;
      }
      // MF initialization can outlast three frame intervals. Preserve a bounded
      // reference chain during this one-time cold start, then switch to the
      // small steady-state queue only after the decoder catches up completely.
      if(queue_.size()>=(warming_ ? 30U : 3U) || bytes_+frame.data.size()>8*1024*1024) {
        dropped_+=queue_.size(); queue_.clear(); bytes_=0; waiting_=true; resetPending_=true; warming_=coldStart_;
      }
      if(frame.data.size()>8*1024*1024) { ++dropped_; request=true; }
      else if(waiting_ && !frame.keyframe) { ++dropped_; request=true; }
      else {
        if(frame.keyframe) waiting_=false;
        bytes_+=frame.data.size(); queue_.push_back({std::move(frame),now_()});
      }
    }
    wake_.notify_one();
    if(request) requestRecovery();
  }
  void stop() {
    { std::lock_guard lock(mutex_); stopping_=true; queue_.clear(); bytes_=0; }
    wake_.notify_all();
    if(worker_.joinable()) worker_.join();
  }
  unsigned failures() const { return failures_.load(); }
  std::uint64_t dropped() const { return dropped_.load(); }
  struct Timing { std::uint64_t samples=0,queueUs=0,workUs=0,queueMaxUs=0; };
  Timing takeTiming() { std::lock_guard lock(mutex_); return std::exchange(timing_,{}); }
  std::uint64_t expired() const { return expired_.load(); }
  std::string error() const { std::lock_guard lock(mutex_); return error_; }
private:
  void fatal(const std::string& detail) {
    { std::lock_guard lock(mutex_); error_=detail.substr(0,1024); failures_=10; stopping_=true; queue_.clear(); bytes_=0; }
    wake_.notify_all();
  }
  bool resetCodec() {
    try { reset_(); return true; }
    catch(const std::exception& error) { fatal(std::string("Decoder cleanup failed: ")+error.what()); }
    catch(...) { fatal("Decoder cleanup failed"); }
    return false;
  }
  void requestRecovery() {
    {
      std::lock_guard lock(mutex_);
      if(stopping_) return;
      const auto now=now_();
      if(lastRequest_ && now-*lastRequest_<std::chrono::milliseconds(ultra_ ? 100 : 300)) return;
      lastRequest_=now;
    }
    try { keyframe_(); }
    catch(const std::exception& error) { fatal(std::string("Video recovery request failed: ")+error.what()); }
    catch(...) { fatal("Video recovery request failed"); }
  }
  void loop() {
    while(true) {
      Frame frame; bool reset=false,request=false; Clock::time_point queued;
      {
        std::unique_lock lock(mutex_);
        wake_.wait(lock,[&] { return stopping_ || resetPending_ || !queue_.empty(); });
        if(stopping_) break;
        // Queue length alone cannot detect two stale frames after a long GPU
        // call. Discard the dependent chain once its local age budget expires.
        const auto budget=std::chrono::milliseconds(warming_ ? 500 : ultra_ ? 50 : 100);
        if(!queue_.empty() && now_()-queue_.front().queued>budget) {
          expired_+=queue_.size(); dropped_+=queue_.size(); queue_.clear(); bytes_=0;
          waiting_=true; resetPending_=true; request=true;
        }
        reset=std::exchange(resetPending_,false);
        if(!queue_.empty()) { queued=queue_.front().queued; frame=std::move(queue_.front().frame); queue_.pop_front(); bytes_-=frame.data.size(); }
      }
      if(reset && !resetCodec()) break;
      if(request) requestRecovery();
      try {
        if(!frame.data.empty()) {
          const auto began=now_(); decode_(std::move(frame));
          const auto ended=now_();
          std::lock_guard lock(mutex_);
          if(!stopping_) failures_=0;
          ++timing_.samples;
          timing_.queueUs+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(began-queued).count());
          timing_.workUs+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(ended-began).count());
          timing_.queueMaxUs=std::max(timing_.queueMaxUs,static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(began-queued).count()));
          if(queue_.empty()) warming_=false;
        }
      } catch(const std::exception& error) {
        ++failures_;
        {
          std::lock_guard lock(mutex_);
          error_=std::string(error.what()).substr(0,1024); dropped_+=queue_.size(); queue_.clear(); bytes_=0; waiting_=true; warming_=coldStart_; resetPending_=true;
        }
        requestRecovery();
      } catch(...) {
        fatal("Unknown video decoder failure");
      }
    }
    // Decoder COM/VideoToolbox objects are released on their owning thread.
    resetCodec();
  }
  std::function<void(Frame)> decode_;
  std::function<void()> reset_,keyframe_;
  Now now_;
  mutable std::mutex mutex_;
  std::condition_variable wake_;
  struct Queued { Frame frame; Clock::time_point queued; };
  std::deque<Queued> queue_;
  Timing timing_;
  std::size_t bytes_=0;
  bool coldStart_=false;
  bool ultra_=false;
  bool stopping_=false, waiting_=true, resetPending_=false, warming_=false;
  std::atomic<unsigned> failures_{0};
  std::atomic<std::uint64_t> dropped_{0};
  std::atomic<std::uint64_t> expired_{0};
  std::optional<Clock::time_point> lastRequest_;
  std::string error_;
  std::thread worker_;
};
}
