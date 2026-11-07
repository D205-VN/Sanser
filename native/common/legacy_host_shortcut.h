#pragma once
#include "../desktop/session_shortcuts.h"
#include <algorithm>
#include <optional>

namespace sanser {
// Legacy Mac control keys deliberately map Command to Ctrl. This optional
// fixed Windows action uses real host HID mappings instead, including Win.
inline std::optional<desktop::SessionShortcut> legacyHostShortcut(std::string_view id) {
  for(auto action:desktop::sessionShortcuts(desktop::ShortcutHost::Windows))
    if(action.id==id)return action;
  return std::nullopt;
}
inline std::optional<std::string_view> legacyHostShortcutId(std::span<const std::uint16_t> keys) {
  for(const auto& action:desktop::sessionShortcuts(desktop::ShortcutHost::Windows))
    if(std::equal(keys.begin(),keys.end(),action.keys.begin(),action.keys.end()))return action.id;
  return std::nullopt;
}
// The input thread owns dispatch. Never replay a down after a partial OS
// rejection; attempt reverse releases and leave failed releases tracked by
// the existing host recovery/watchdog. No caller-provided key sequence.
template<class Reset,class Neutral,class Allowed,class Apply>
bool applyLegacyHostShortcut(std::string_view id,Reset reset,Neutral neutral,Allowed allowed,Apply apply) {
  const auto action=legacyHostShortcut(id);
  if(!action || !allowed())return false;
  reset();if(!neutral() || !allowed())return false;
  std::vector<std::uint16_t> attempted;
  bool complete=true;
  for(const auto key:action->keys) {
    if(!allowed()){complete=false;break;}
    attempted.push_back(key);
    if(!apply(key,true)){complete=false;break;}
  }
  for(auto it=attempted.rbegin();it!=attempted.rend();++it)if(!apply(*it,false))complete=false;
  return complete;
}
}
