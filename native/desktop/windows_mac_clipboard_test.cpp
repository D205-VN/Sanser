#include "windows_mac_clipboard.h"
#include "clipboard_paste_gate.h"
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
  {
    // The editing profile (the Windows viewer's default) covers the everyday
    // Ctrl chords a Windows user expects on a Mac, and never Command+Q.
    for(auto code:{4,5,9,12,17,18,19,21,22,23,24,26,27,29}) {
      WindowsMacClipboard editing;
      editing.apply({Input::Key,0,0,224,0,true},true,true);
      const auto mapped=editing.apply({Input::Key,0,0,static_cast<std::uint16_t>(code),0,true},true,true);
      check(mapped.size()==3 && mapped[0].code==224 && !mapped[0].down && mapped[1].code==227 && mapped[1].down && mapped[2].code==code,
        "Editing profile did not send a Windows Ctrl chord as Command");
    }
    for(auto code:{20,28,7}) { // Q, Y, D stay physical Control.
      WindowsMacClipboard editing;
      editing.apply({Input::Key,0,0,224,0,true},true,true);
      const auto raw=editing.apply({Input::Key,0,0,static_cast<std::uint16_t>(code),0,true},true,true);
      check(raw.size()==1 && raw[0].code==code,"Editing profile remapped a chord it must leave alone");
    }
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
  auto events=key(7,true);check(events.size()==1 && events[0].code==7,"Unrelated Control chord changed");
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
  {
    // Ctrl+V on a Mac host waits for the Windows text to reach the Mac.
    WindowsMacClipboard paste; ClipboardPasteGate gate; const auto t0=ClipboardPasteGate::Clock::now();
    const auto down=[&](std::uint16_t code,bool isDown) { return paste.apply({Input::Key,0,0,code,0,isDown},true,false); };
    check(gate.route(down(224,true)).size()==1,"Control alone must pass through");
    auto chord=down(25,true);
    check(ClipboardPasteGate::startsPaste(chord,paste.commandActive()),"Command+V was not recognised as a paste");
    const auto ticket=gate.hold(std::move(chord),t0);
    check(gate.route(down(25,false)).empty() && gate.route(down(224,false)).empty(),"Input after a pending paste overtook it");
    check(gate.release(t0+std::chrono::milliseconds(100)).empty(),"Paste released before the clipboard write");
    gate.complete(ticket+1);
    check(gate.release(t0+std::chrono::milliseconds(100)).empty(),"A stale completion released the paste");
    gate.complete(ticket);
    const auto released=gate.release(t0+std::chrono::milliseconds(120));
    // Control up, Command down, V down, then the queued V up / Command up.
    check(released.size()>=5 && released[0].code==224 && !released[0].down && released[1].code==227 && released[1].down &&
      released[2].code==25 && released[2].down && released[3].code==25 && !released[3].down,"Held paste lost order");
    check(!gate.waiting() && gate.route(down(4,true)).size()==1,"Gate stayed closed after release");
    gate.hold({{Input::Key,0,0,25,0,true}},t0);
    check(gate.release(t0+ClipboardPasteGate::timeout).size()==1,"A failed clipboard write must not block typing");
    WindowsMacClipboard plain; auto ctrlV=(plain.apply({Input::Key,0,0,224,0,true},false),plain.apply({Input::Key,0,0,25,0,true},false));
    check(!ClipboardPasteGate::startsPaste(ctrlV,plain.commandActive()),"Windows host paste must not wait for a Mac clipboard");
  }
  std::cout<<"Windows-to-Mac copy/paste: modifier sides, repeats, reset, native chords, host identity and clipboard-synced paste passed\n";
  return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
