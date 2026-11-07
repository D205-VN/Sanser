#pragma once
#include "controller_capture.h"
#include <atomic>
#include <memory>
#include <stdexcept>

namespace sanser::desktop {
// Main-thread capture ownership; the shared sender only reads atomic/native
// availability. AppKit/Win32 focus objects never enter the sender thread.
class ControllerSession {
public:
  struct Callbacks {
    std::function<bool()> available;
    std::function<std::uint32_t()> generation;
    std::function<bool(std::uint32_t)> reset;
    std::function<bool(GamepadState,std::uint32_t)> submit;
  };
  explicit ControllerSession(Callbacks callbacks):callbacks_(std::move(callbacks)) {
    if(!callbacks_.available || !callbacks_.generation || !callbacks_.reset || !callbacks_.submit)
      throw std::invalid_argument("Incomplete controller callbacks");
    capture_=std::make_unique<ControllerCapture>(
      [this]{const auto epoch=callbacks_.generation();return focused_ && fresh() && callbacks_.available() && epoch==captureEpoch_ ? epoch:0U;},callbacks_.submit);
  }
  ~ControllerSession(){capture_.reset();}
  bool available()const{return callbacks_.available();}
  bool active()const{return active_;}
  void refresh(bool focused) {
    focused_=focused;
    if(active_ && (!focused || !(capture_->status(id_)&2)))stop();
  }
  bool toggle() {
    if(active_){stop();return true;}
    const auto epoch=callbacks_.generation();
    if(!focused_ || !available() || !epoch || id_==UINT64_MAX)return false;
    try{if(!callbacks_.reset(epoch))return false;}catch(...){return false;}
    sampledAt_=ticks();captureEpoch_=epoch;
    active_=capture_->capture(++id_,true);return active_;
  }
  bool sample(ControllerSample state) {
    if(!active_ || !focused_)return false;
    sampledAt_=ticks();
    if(!capture_->sample(id_,state)){stop();return false;}return true;
  }
  void stop(){if(active_)capture_->capture(id_,false);active_=false;}
  unsigned status(){return active_ ? capture_->status(id_):0;}
private:
  static std::int64_t ticks(){return std::chrono::duration_cast<std::chrono::nanoseconds>(ControllerCapture::Clock::now().time_since_epoch()).count();}
  // A blocked UI must not refresh a held state indefinitely from its last poll.
  bool fresh()const{return ticks()-sampledAt_.load()<200000000;}
  std::atomic<std::int64_t> sampledAt_{0};std::atomic<std::uint32_t> captureEpoch_{0};
  Callbacks callbacks_;std::atomic<bool> focused_{false};
  std::unique_ptr<ControllerCapture> capture_;std::uint64_t id_=0;bool active_=false;
};
}
