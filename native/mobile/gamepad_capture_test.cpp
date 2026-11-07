#include "gamepad_capture.h"
#include <atomic>
#include <iostream>
#include <stdexcept>
#include <limits>
using namespace sanser::mobile;
using State=sanser::desktop::GamepadState;
void check(bool value,const char* text){if(!value)throw std::runtime_error(text);}
template<class F>bool waitFor(F f){for(unsigned i=0;i<200;++i){if(f())return true;std::this_thread::sleep_for(std::chrono::milliseconds(5));}return false;}
int main(){try{
  SanserMobileGamepad raw{};raw.connected=raw.sampled=1;
  constexpr std::uint16_t expected[]={0x1000,0x2000,0x4000,0x8000,0x100,0x200,0x10,0x20,0x40,0x80,1,2,4,8};
  for(unsigned i=0;i<14;++i){raw.buttons=1U<<i;check(physicalGamepad(raw)->buttons==expected[i],"physical/Xbox button layout mismatch");}
  raw.buttons=0;raw.lx=-1;raw.ly=1;raw.rx=-.5;raw.ry=.5;raw.lt=.5;raw.rt=1;
  const auto axes=physicalGamepad(raw);check(axes && axes->lx==-32768 && axes->ly==32767 && axes->rx==-16384 && axes->ry==16384 && axes->lt==128 && axes->rt==255,"signed axes/trigger scaling");
  raw.lx=std::numeric_limits<double>::quiet_NaN();check(!physicalGamepad(raw),"NaN accepted");raw={};raw.connected=raw.sampled=1;
  std::atomic<std::uint32_t> epoch{1};std::atomic<bool> accept{true};std::mutex mutex;std::vector<State> sent;
  GamepadCapture capture([&]{return epoch.load();},[&](State s,std::uint32_t generation){check(generation==1 || generation==2,"invalid remote epoch");std::lock_guard lock(mutex);sent.push_back(s);return accept.load();});
  const auto count=[&]{std::lock_guard lock(mutex);return sent.size();};
  check(!capture.capture(0,true) && capture.capture(1,true),"capture ID validation");
  raw.sampled=0;check(capture.sample(1,raw) && capture.status(1)==11 && count()==0,"attach availability armed analog");
  raw.sampled=1;raw.buttons=1;check(capture.sample(1,raw) && count()==0,"held-before-capture input forwarded");
  raw.buttons=0;check(capture.sample(1,raw) && waitFor([&]{return count()>0;}) && capture.status(1)==15,"neutral sample did not arm");
  raw.lx=.5;raw.lt=.25;raw.buttons=1;check(capture.sample(1,raw),"active sample");
  check(waitFor([&]{std::lock_guard lock(mutex);return sent.back().lx==16384 && sent.back().buttons==0x1000;}),"native analog values");
  auto before=count();std::this_thread::sleep_for(std::chrono::milliseconds(165));check(count()>=before+3,"held input lacks native heartbeat");
  check(capture.capture(2,true),"replace capture");before=count();
  check(!capture.sample(1,raw) && capture.capture(1,false) && capture.status(2)==3 && count()==before,"stale source affected replacement");
  raw={};raw.connected=raw.sampled=1;check(capture.sample(2,raw),"replacement neutral");
  check(waitFor([&]{return count()>before;}),"replacement pulse");capture.release();before=count();
  std::this_thread::sleep_for(std::chrono::milliseconds(60));check(count()==before && !capture.capture(2,true) && capture.status(2)==1,"released capture revived/continued");
  check(capture.capture(3,true) && capture.sample(3,raw),"epoch setup");epoch=2;
  check(waitFor([&]{return capture.status(3)==1;}),"remote generation change retained capture");
  check(capture.capture(4,true),"new epoch capture");raw.buttons=0xc0;
  check(!capture.sample(4,raw) && capture.status(4)==1,"Start+Select sent to host");
  check(capture.capture(5,true),"invalid sample setup");raw.buttons=0;raw.rt=2;
  check(!capture.sample(5,raw) && capture.status(5)==1,"bad source kept heartbeat");
  check(capture.capture(6,true),"backend failure setup");raw.rt=0;accept=false;capture.sample(6,raw);
  check(waitFor([&]{return capture.status(6)==1;}),"rejected send silently retried indefinitely");
  epoch=0;check(!capture.capture(7,true) && capture.status(0)==0,"unavailable host enabled analog");
  std::cout<<"Mobile analog: physical mappings, native held heartbeat, capture/epoch isolation, escape, release and failure checks passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
