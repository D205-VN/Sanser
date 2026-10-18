#pragma once
#include <algorithm>
#include <chrono>
#include <cstdint>

namespace sanser::desktop {
// Per-viewer health, evaluated with monotonic time and interval deltas rather
// than lifetime ratios. Access is serialized by Delivery::mutex.
class SharingDeliveryPolicy {
public:
  using Clock=std::chrono::steady_clock;
  enum State { Active, Paused, Probing };
  struct Sample {
    std::uint64_t submitted=0, replaced=0, referenceDrops=0, queueDrops=0;
    unsigned backoffMs=0;
    std::uint64_t feedbackSerial=0, deliveredBytes=0;
    std::uint64_t completedFrame=0, sentFrame=0;
    bool feedbackFresh=false, feedbackCongested=false;
  };
  void observe(Clock::time_point now,const Sample& sample) {
    if(!initialized_) { initialized_=true; previous_=sample; sampledAt_=now; return; }
    if(state_==Paused) {
      if(now>=retryAt_) {
        state_=Probing; probeAt_=now; cleanFeedback_=0; delivered_=false;
        probeAfterFrame_=sample.sentFrame;
        previous_=sample; sampledAt_=now;
      }
      return;
    }
    if(now-sampledAt_<std::chrono::milliseconds(500)) return;
    // Sampling gaps are not evidence of sustained congestion or recovery.
    if(now-sampledAt_>std::chrono::seconds(2)) { bad_=false; backoff_=false; cleanFeedback_=0; }
    const auto delta=[](auto current,auto old) { return current>=old ? current-old : 0; };
    const auto submitted=delta(sample.submitted,previous_.submitted);
    const auto dropped=delta(sample.replaced,previous_.replaced)+delta(sample.referenceDrops,previous_.referenceDrops);
    const bool localBad=submitted>=10 && (dropped*100>=submitted*35 || sample.queueDrops>previous_.queueDrops);
    const bool newFeedback=sample.feedbackFresh && sample.feedbackSerial>previous_.feedbackSerial;
    const bool recovered=newFeedback && !sample.feedbackCongested && sample.deliveredBytes>0 && sample.completedFrame>previous_.completedFrame;
    const bool pressure=localBad || (sample.feedbackFresh && sample.feedbackCongested);
    previous_=sample; sampledAt_=now;
    if(state_==Probing) {
      if(pressure) { ++failedProbes_; pause(now,1); return; }
      if(newFeedback) { ++cleanFeedback_; delivered_=delivered_ || sample.deliveredBytes>0; }
      if(delivered_ && sample.completedFrame>probeAfterFrame_ && cleanFeedback_>=3 && now-probeAt_>=std::chrono::milliseconds(1500)) {
        state_=Active; failedProbes_=0; bad_=false; backoff_=false; pauseReason_=0; return;
      }
      if(now-probeAt_>=std::chrono::seconds(3)) { ++failedProbes_; pause(now,3); }
      return;
    }
    if(pressure) { if(!bad_) badAt_=now; bad_=true; } else bad_=false;
    // Publishing an IDR alone is not recovery proof, but fresh clean feedback
    // and a completed new picture are. Old retry history must not freeze a
    // viewer which has already recovered. Sustained congestion still isolates it.
    if(sample.backoffMs>=2000 && !recovered) { if(!backoff_) backoffAt_=now; backoff_=true; } else backoff_=false;
    if(bad_ && now-badAt_>=std::chrono::seconds(6)) pause(now,1);
    else if(backoff_ && now-backoffAt_>=std::chrono::seconds(8)) pause(now,2);
  }
  State state() const { return state_; }
  bool sendsVideo() const { return state_!=Paused; }
  bool affectsBitrate() const { return state_==Active; }
  const char* label() const { return state_==Active ? "active" : state_==Paused ? "paused" : "probing"; }
  std::uint64_t pauses() const { return pauses_; }
  // 0=none, 1=sustained pressure, 2=IDR backoff without progress, 3=failed probe.
  unsigned pauseReason() const { return pauseReason_; }
private:
  void pause(Clock::time_point now,unsigned reason) {
    pauseReason_=reason;
    state_=Paused; ++pauses_; bad_=false; backoff_=false;
    retryAt_=now+std::chrono::seconds(std::min(30U,5U*(1U<<std::min(3U,failedProbes_))));
  }
  State state_=Active;
  bool initialized_=false, bad_=false, backoff_=false, delivered_=false;
  unsigned failedProbes_=0, cleanFeedback_=0;
  unsigned pauseReason_=0;
  std::uint64_t pauses_=0;
  std::uint64_t probeAfterFrame_=0;
  Sample previous_{};
  Clock::time_point sampledAt_{}, badAt_{}, backoffAt_{}, retryAt_{}, probeAt_{};
};
}
