#include "windows_mac_clipboard.h"
#include "wheel_input.h"
#include <iostream>
#include <stdexcept>
using namespace sanser::desktop;
void check(bool ok,const char* message) {if(!ok)throw std::runtime_error(message);}
int main() {try {
  {
    WindowsMacClipboard defaults;
    defaults.apply({Input::Key,0,0,224,0,true},true);
    auto raw=defaults.apply({Input::Key,0,0,26,0,true},true);
    check(raw.size()==1 && raw[0].code==26,"Default mode converted a game's Control+W into Close Window");
    defaults.apply(Input{},true);
    defaults.apply({Input::Key,0,0,224,0,true},true);
    auto copy=defaults.apply({Input::Key,0,0,6,0,true},true);
    check(copy.size()==3 && copy[1].code==227,"Default mode did not translate Copy");
  }
  WheelInput wheel;
  for(unsigned i=0;i<3;++i)check(wheel.steps(0,30)==0,"High-resolution wheel emitted too soon");
  check(wheel.steps(0,30)==1 && wheel.steps(1,-60)==0 && wheel.steps(0,60)==0 && wheel.steps(1,-60)==-1 && wheel.steps(0,60)==1,"Wheel axes lost signed fractions or shared remainders");
  wheel.steps(0,90);wheel.reset();check(wheel.steps(0,30)==0,"Focus reset retained wheel remainder");
  for(auto action:{4,6,9,22,25,26,27,29}) for(auto ctrl:{224,228}) {
    WindowsMacClipboard mapper; std::vector<Input> trace;
    const auto edge=[&](unsigned code,bool down) {
      auto events=mapper.apply({Input::Key,0,0,static_cast<std::uint16_t>(code),0,down},true,true);
      trace.insert(trace.end(),events.begin(),events.end());
    };
    edge(ctrl,true); edge(action,true); edge(action,true); edge(action,false); edge(ctrl,false);
    const std::vector<std::pair<unsigned,bool>> expected{{ctrl,true},{ctrl,false},{227,true},
      {action,true},{action,true},{action,false},{227,false},{ctrl,true},{ctrl,false}};
    check(trace.size()==expected.size(),"Copy/paste repeated modifier edges");
    for(std::size_t i=0;i<trace.size();++i)
      check(trace[i].kind==Input::Key && trace[i].code==expected[i].first && trace[i].down==expected[i].second,"Copy/paste press/release order differs");
  }
  WindowsMacClipboard mapper;
  const auto key=[&](unsigned code,bool down,bool mac=true) {return mapper.apply({Input::Key,0,0,static_cast<std::uint16_t>(code),0,down},mac,true);};
  key(224,true);key(6,true);
  check(key(224,false).empty(),"Early Control release leaked into Command chord");
  const auto released=key(6,false);
  check(released.size()==2 && released[1].code==227 && !released[1].down,"Early Control release left Command stuck");
  for(auto modifier:{225,229,226,230,227,231}) {
    mapper.apply(Input{},true,true);key(224,true);key(modifier,true);
    auto events=key(6,true);check(events.size()==1 && events[0].code==6,"Shift/Alt/Command chord was remapped");
  }
  mapper.apply(Input{},true,true);key(224,true);
  auto events=key(5,true);check(events.size()==1 && events[0].code==5,"Unrelated Control chord changed");
  mapper.apply(Input{},true,true);key(224,true);key(225,true);events=key(29,true);
  check(events.size()==3 && events[1].code==227 && events[1].down && events[2].code==29,"Ctrl+Shift+Z did not become Mac redo");
  mapper.apply(Input{},true,true);key(224,true);key(25,true);
  events=mapper.apply({Input::Button,0,0,0,0,true},true,true);
  check(events.size()==3 && events[0].code==227 && !events[0].down && events[1].code==224 && events[1].down && events[2].kind==Input::Button,"Ctrl+click inherited synthetic Command");
  events=mapper.apply(Input{},true,true);check(events.size()==1 && events[0].kind==Input::Reset,"Focus reset must release all synthetic modifiers");
  check(key(25,true).size()==1,"Reset retained stale Control");
  mapper.apply(Input{},false,true);key(224,true,false);events=key(6,true,false);
  check(events.size()==1 && events[0].code==6,"Windows/unknown host was remapped");
  events=key(25,true,true);check(events.size()==2 && events[0].kind==Input::Reset && events[1].code==25,"Host identity change retained held input");
  mapper.apply(Input{},true,true);key(224,true);key(228,true);key(25,true);
  check(key(224,false).empty(),"One-sided Control release disturbed mapped chord");
  events=key(25,false);
  check(events.size()==3 && events[1].code==227 && !events[1].down && events[2].code==228 && events[2].down,"Both Control sides did not restore independently");
  std::cout<<"Windows-to-Mac copy/paste: modifier sides, repeats, reset, native chords and host identity passed\n";
  return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
