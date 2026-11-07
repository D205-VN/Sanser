#include "controller_session.h"
#include <iostream>
#include <stdexcept>
#include <vector>
using namespace sanser::desktop;
namespace {
void check(bool value,const char* text){if(!value)throw std::runtime_error(text);}
template<class F>bool waitFor(F f){for(unsigned i=0;i<100;++i){if(f())return true;std::this_thread::sleep_for(std::chrono::milliseconds(5));}return false;}
}
int main(){try{
  std::atomic<bool> available{false};std::atomic<std::uint32_t> epoch{1};
  bool admit=true,changeOnReset=false;unsigned resets=0;std::mutex mutex;std::vector<GamepadState> sent;
  ControllerSession model({[&]{return available.load();},[&]{return epoch.load();},
    [&](auto e){check(e==epoch,"reset wrong generation");++resets;if(changeOnReset)++epoch;return admit;},
    [&](auto state,auto){std::lock_guard lock(mutex);sent.push_back(state);return true;}});
  const auto count=[&]{std::lock_guard lock(mutex);return sent.size();};
  check(!model.toggle() && resets==0,"unfocused capture reset host");
  model.refresh(true);check(!model.toggle() && resets==0,"unsupported host reset");
  available=true;epoch=0;check(!model.toggle() && resets==0,"zero epoch accepted");
  epoch=1;admit=false;check(!model.toggle() && !model.active() && count()==0,"full reliable window enabled controller");
  admit=true;changeOnReset=true;check(!model.toggle() && !model.active(),"generation changed during reset armed a new peer");
  changeOnReset=false;check(model.toggle() && model.active(),"capture did not start");
  ControllerSample sample;sample.connected=sample.sampled=1;sample.buttons=1;
  model.sample(sample);std::this_thread::sleep_for(std::chrono::milliseconds(25));check(count()==0,"held sample armed capture");
  sample.buttons=0;model.sample(sample);check(waitFor([&]{return count()>0;}),"neutral missing");
  sample.lx=.5;sample.rt=1;model.sample(sample);
  check(waitFor([&]{std::lock_guard lock(mutex);return sent.back().lx==16384 && sent.back().rt==255;}),"analog magnitude lost");
  model.refresh(false);const auto afterStop=count();
  std::this_thread::sleep_for(std::chrono::milliseconds(55));check(!model.active() && count()==afterStop,"focus loss kept sending");
  {std::lock_guard lock(mutex);check(sent.back()==GamepadState{},"focus loss did not disconnect");}
  model.refresh(true);check(!model.active(),"focus recovery restarted capture");
  check(model.toggle(),"restart");sample={};sample.connected=sample.sampled=1;model.sample(sample);
  sample.buttons=0xc0;check(!model.sample(sample) && !model.active(),"Start+Select was forwarded");
  check(model.toggle(),"generation setup");sample.buttons=0;model.sample(sample);epoch=2;
  check(waitFor([&]{model.refresh(true);return !model.active();}),"generation change retained capture");
  check(model.toggle(),"permission setup");model.sample(sample);available=false;
  check(waitFor([&]{model.refresh(true);return !model.active();}),"capability loss retained capture");
  available=true;check(model.toggle(),"stalled UI setup");model.sample(sample);
  const auto before=count();check(waitFor([&]{return count()>before;}),"heartbeat missing");
  std::this_thread::sleep_for(std::chrono::milliseconds(260));model.refresh(true);
  check(!model.active(),"stalled UI replayed old sample indefinitely");
  const auto afterStall=count();std::this_thread::sleep_for(std::chrono::milliseconds(55));check(count()==afterStall,"stale capture sent after stop");
  std::cout<<"Controller session: bounded reset, neutral arming, focus, escape, generation/capability and stalled poll release passed\n";
  return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
