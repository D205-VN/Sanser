#pragma once
#include "input.h"
#include <set>
#include <utility>
#include <vector>

namespace sanser::desktop {
// Windows viewer only. Translate plain Ctrl+C/V on an authenticated Mac host;
// preserve real Control for terminals, games, Ctrl+click and other shortcuts.
// Each edge still goes through Peer's existing reliable input path.
class WindowsMacClipboard {
public:
  std::vector<Input> apply(Input input,bool macHost) {
    std::vector<Input> result;
    if(macHost_!=macHost) {
      if(!held_.empty()) result.push_back(Input{});
      held_.clear(); mapped_.clear(); command_=false; macHost_=macHost;
    }
    if(input.kind==Input::Reset || input.kind==Input::Text) {
      held_.clear(); mapped_.clear(); command_=false;
      result.push_back(std::move(input)); return result;
    }
    const auto key=[&](std::uint16_t code,bool down) {result.push_back({Input::Key,0,0,code,0,down});};
    const auto restore=[&] {
      if(!command_) return;
      key(227,false); command_=false;
      for(auto code:{224,228}) if(held_.contains(code)) key(code,true);
    };
    if(input.kind!=Input::Key) {
      // Mouse/scroll modifiers must retain their physical Control semantics.
      restore(); result.push_back(std::move(input)); return result;
    }
    if(input.down) held_.insert(input.code); else held_.erase(input.code);
    const bool control=input.code==224 || input.code==228;
    if(command_ && control) return result;
    if(!input.down && mapped_.erase(input.code)) {
      result.push_back(input);
      if(mapped_.empty()) restore();
      return result;
    }
    const bool copyPaste=input.code==6 || input.code==25;
    bool plainControl=held_.contains(224) || held_.contains(228);
    for(auto code:held_) if(code!=224 && code!=228 && code!=6 && code!=25) plainControl=false;
    if(macHost && input.down && copyPaste && plainControl) {
      if(!command_) {
        for(auto code:{224,228}) if(held_.contains(code)) key(code,false);
        key(227,true); command_=true;
      }
      mapped_.insert(input.code); result.push_back(input); return result;
    }
    if(input.down) restore();
    result.push_back(std::move(input)); return result;
  }
private:
  bool macHost_=false,command_=false;
  std::set<std::uint16_t> held_,mapped_;
};
}
