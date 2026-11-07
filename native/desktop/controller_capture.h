#pragma once
#include "gamepad.h"
#include <cmath>
#include <condition_variable>
#include <functional>
#include <mutex>
#include <thread>

namespace sanser::desktop {
struct ControllerSample {
  int connected=0,sampled=0;std::uint32_t buttons=0;
  double lx=0,ly=0,rx=0,ry=0,lt=0,rt=0;
};
template<class Sample> inline std::optional<GamepadState> physicalGamepad(const Sample& raw) {
  if((raw.connected!=0 && raw.connected!=1) || (raw.sampled!=0 && raw.sampled!=1) || raw.buttons>0x3fff)return {};
  for(auto v:{raw.lx,raw.ly,raw.rx,raw.ry})if(!std::isfinite(v) || v<-1 || v>1)return {};
  for(auto v:{raw.lt,raw.rt})if(!std::isfinite(v) || v<0 || v>1)return {};
  GamepadState state;if(!raw.connected)return state;state.connected=true;
  constexpr std::uint16_t mask[]={0x1000,0x2000,0x4000,0x8000,0x100,0x200,0x10,0x20,0x40,0x80,1,2,4,8};
  for(unsigned i=0;i<14;++i)if(raw.buttons & (1U<<i))state.buttons|=mask[i];
  const auto stick=[](double v){return static_cast<std::int16_t>(std::lround(v*(v<0?32768:32767)));};
  state.lx=stick(raw.lx);state.ly=stick(raw.ly);state.rx=stick(raw.rx);state.ry=stick(raw.ry);
  state.lt=static_cast<std::uint8_t>(std::lround(raw.lt*255));state.rt=static_cast<std::uint8_t>(std::lround(raw.rt*255));return state;
}
// One latest snapshot, no UI timer for transmission or historical axis queue.
// Callbacks are short native transport operations; all state/send/stop share a
// lock so a disable finishes after an in-progress send and before later samples.
class ControllerCapture {
public:
  using State=GamepadState;using Clock=std::chrono::steady_clock;
  ControllerCapture(std::function<std::uint32_t()> current,std::function<bool(State,std::uint32_t)> send)
    :current_(std::move(current)),send_(std::move(send)),worker_([this]{loop();}){}
  ~ControllerCapture(){ {std::lock_guard lock(mutex_);clear();stopping_=true;}wake_.notify_one();if(worker_.joinable())worker_.join(); }
  bool capture(std::uint64_t id,bool enabled) {
    std::lock_guard lock(mutex_);
    if(!enabled){if(id==capture_)clear();return true;}
    if(!id || stopping_)return false;
    const auto epoch=current_();if(!epoch)return false;
    if(capture_==id)return epoch==epoch_;
    if(id<=highest_)return false;
    clear();highest_=capture_=id;epoch_=epoch;wake_.notify_one();return true;
  }
  void release(){std::lock_guard lock(mutex_);clear();}
  template<class Sample> bool sample(std::uint64_t id,const Sample& raw) {
    std::lock_guard lock(mutex_);
    if(!id || id!=capture_)return false;
    const auto state=physicalGamepad(raw);
    if(!state || current_()!=epoch_ || (raw.buttons&0xc0)==0xc0){clear();return false;}
    connected_=raw.connected!=0;
    if(!connected_){neutral();return true;}
    if(!raw.sampled)return true; // Android attach is not a hardware neutral sample.
    if(!armed_){
      const auto centered=[](std::int16_t n){return n>=-4096 && n<=4096;};
      if(state->buttons || state->lt>8 || state->rt>8 || !centered(state->lx) || !centered(state->ly) || !centered(state->rx) || !centered(state->ry))return true;
      armed_=true;latest_=State{};latest_.connected=true; // Canonical neutral, no startup drift.
    }else latest_=*state;
    dirty_=true;wake_.notify_one();return true;
  }
  unsigned status(std::uint64_t id){std::lock_guard lock(mutex_);const auto epoch=current_();
    if(capture_ && epoch!=epoch_)clear();
    return (epoch?1U:0U)|((id && capture_==id)?(2U|(armed_?4U:0U)|(connected_?8U:0U)):0U);
  }
private:
  void neutral(){if(armed_)try{(void)send_({},epoch_);}catch(...){}armed_=false;dirty_=false;latest_={};}
  void clear(){neutral();capture_=0;epoch_=0;connected_=false;wake_.notify_one();}
  void loop(){std::unique_lock lock(mutex_);
    while(!stopping_){
      if(!capture_){wake_.wait(lock,[&]{return stopping_ || capture_!=0;});continue;}
      if(current_()!=epoch_){clear();continue;}
      const auto now=Clock::now();
      if(armed_ && now>=sent_+std::chrono::milliseconds(dirty_?8:40)){
        bool accepted=false;try{accepted=send_(latest_,epoch_);}catch(...){}
        if(!accepted){clear();continue;}dirty_=false;sent_=now;
      }
      wake_.wait_until(lock,armed_ ? sent_+std::chrono::milliseconds(dirty_?8:40):now+std::chrono::milliseconds(40));
    }
  }
  std::function<std::uint32_t()> current_;std::function<bool(State,std::uint32_t)> send_;
  std::mutex mutex_;std::condition_variable wake_;std::uint64_t highest_=0,capture_=0;std::uint32_t epoch_=0;
  State latest_{};bool stopping_=false,armed_=false,connected_=false,dirty_=false;Clock::time_point sent_{};
  std::thread worker_;
};
}
