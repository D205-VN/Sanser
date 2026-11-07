#include "legacy_host_shortcut.h"
#include <iostream>
#include <stdexcept>
void check(bool value,const char* message){if(!value)throw std::runtime_error(message);}
int main(){try{
  unsigned resets=0;bool allowed=true,neutral=true;
  std::vector<std::pair<std::uint16_t,bool>> events;
  auto apply=[&](std::uint16_t key,bool down){events.emplace_back(key,down);return true;};
  auto run=[&](std::string_view id){return sanser::applyLegacyHostShortcut(id,[&]{++resets;},[&]{return neutral;},[&]{return allowed;},apply);};
  for(const auto& action:sanser::desktop::sessionShortcuts(sanser::desktop::ShortcutHost::Windows)){
    events.clear();check(run(action.id),"fixed Windows action failed");
    check(sanser::legacyHostShortcutId(action.keys)==action.id,"HID/profile lookup mismatch");
    check(events.size()==action.keys.size()*2,"unbalanced native shortcut");
    for(std::size_t i=0;i<action.keys.size();++i){check(events[i]==std::pair{action.keys[i],true},"press order");check(events[events.size()-1-i]==std::pair{action.keys[i],false},"release order");}
  }
  auto before=resets;events.clear();check(!run("arbitrary command") && resets==before && events.empty(),"unlisted action reached OS");
  allowed=false;check(!run("copy") && resets==before,"permission denied action reset input");
  allowed=true;neutral=false;check(!run("copy") && events.empty(),"failed neutral reset still pressed keys");neutral=true;
  unsigned calls=0;events.clear();
  check(!sanser::applyLegacyHostShortcut("previous",[]{},[]{return true;},[]{return true;},[&](std::uint16_t key,bool down){events.emplace_back(key,down);return ++calls!=2;}),"partial OS failure returned success");
  check(events==std::vector<std::pair<std::uint16_t,bool>>{{226,true},{225,true},{225,false},{226,false}},"failure replayed a press or skipped reverse cleanup");
  events.clear();calls=0;allowed=true;
  check(!sanser::applyLegacyHostShortcut("copy",[]{},[]{return true;},[&]{return allowed;},[&](std::uint16_t key,bool down){events.emplace_back(key,down);if(++calls==1)allowed=false;return true;}),"permission-loss race accepted action");
  check(events==std::vector<std::pair<std::uint16_t,bool>>{{224,true},{224,false}},"permission loss prevented release or sent later key");
  check(sanser::desktop::windowsKey(sanser::legacyHostShortcut("desktop")->keys.front())==0x15b,"Windows key mapped through Mac Command/Ctrl alias");
  std::cout<<"Legacy host shortcuts: fixed IDs, Windows HID, policy/reset gates and reverse cleanup without replay passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
