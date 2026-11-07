#pragma once
#include "keyboard.h"
#include <cstdint>
#include <functional>
#include <optional>
#include <span>
#include <stdexcept>
#include <string_view>
#include <vector>
#include <utility>

namespace sanser::desktop {
enum class ShortcutHost { Windows, Mac };
struct SessionShortcut {
  std::string_view id,label,caption;
  std::vector<std::uint16_t> keys;
};
inline std::vector<SessionShortcut> sessionShortcuts(ShortcutHost host) {
  const bool mac=host==ShortcutHost::Mac;
  const std::uint16_t modifier=mac ? 227:224;
  std::vector<SessionShortcut> result{
    {"switch","Switch app",mac ? "Cmd+Tab":"Alt+Tab",{static_cast<std::uint16_t>(mac ? 227:226),43}},
    {"previous","Previous app",mac ? "Cmd+Shift+Tab":"Alt+Shift+Tab",{static_cast<std::uint16_t>(mac ? 227:226),225,43}}
  };
  if(mac) result.insert(result.end(),{
    {"minimize","Minimize window","Cmd+M",{227,16}},
    {"minimize-app","Minimize app windows","Option+Cmd+M",{226,227,16}},
    {"fullscreen","Toggle fullscreen","Ctrl+Cmd+F",{224,227,9}},
    {"hide","Hide app","Cmd+H",{227,11}},
    {"search","Spotlight","Cmd+Space",{227,44}}
  });
  else result.insert(result.end(),{
    {"desktop","Show desktop","Win+D",{227,7}},
    {"overview","Task View","Win+Tab",{227,43}},
    {"snap-left","Snap left","Win+Left",{227,80}},
    {"snap-right","Snap right","Win+Right",{227,79}},
    {"minimize","Minimize all windows","Win+M",{227,16}},
    {"explorer","File Explorer","Win+E",{227,8}}
  });
  result.push_back({"close","Close active window",mac ? "Cmd+W":"Alt+F4",mac ? std::vector<std::uint16_t>{227,26}:std::vector<std::uint16_t>{226,61}});
  result.insert(result.end(),{
    {"copy","Copy",mac ? "Cmd+C":"Ctrl+C",{modifier,6}},
    {"paste","Paste",mac ? "Cmd+V":"Ctrl+V",{modifier,25}},
    {"cut","Cut",mac ? "Cmd+X":"Ctrl+X",{modifier,27}},
    {"undo","Undo",mac ? "Cmd+Z":"Ctrl+Z",{modifier,29}},
    {"redo","Redo",mac ? "Cmd+Shift+Z":"Ctrl+Y",mac ? std::vector<std::uint16_t>{227,225,29}:std::vector<std::uint16_t>{224,28}},
    {"select-all","Select all",mac ? "Cmd+A":"Ctrl+A",{modifier,4}},
    {"find","Find",mac ? "Cmd+F":"Ctrl+F",{modifier,9}},
    {"save","Save",mac ? "Cmd+S":"Ctrl+S",{modifier,22}}
  });
  return result;
}
inline bool validShortcutKeys(std::span<const std::uint16_t> keys) {
  if(keys.empty() || keys.size()>4)return false;
  for(std::size_t i=0;i<keys.size();++i) {
    if(!windowsKey(keys[i]))return false;
    for(std::size_t n=0;n<i;++n)if(keys[n]==keys[i])return false;
  }
  return true;
}

// UI-thread model. Native wrappers own focus and layout; Peer owns transport.
// No window identity or arbitrary commands are accepted from the UI.
class ShortcutSession {
public:
  struct Callbacks {
    std::function<bool()> available,focused;
    std::function<std::uint32_t()> generation;
    std::function<bool()> begin;
    std::function<bool(std::span<const std::uint16_t>,std::uint32_t)> submit;
    std::function<bool(ShortcutHost)> supportsHost={};
    std::function<void()> closed={};
  };
  explicit ShortcutSession(Callbacks callbacks):callbacks_(std::move(callbacks)) {
    if(!callbacks_.available || !callbacks_.focused || !callbacks_.generation || !callbacks_.begin || !callbacks_.submit)
      throw std::invalid_argument("Incomplete shortcut callbacks");
  }
  bool available() const {return callbacks_.available();}
  bool supports(ShortcutHost host) const {return (host==ShortcutHost::Windows || host==ShortcutHost::Mac) && (!callbacks_.supportsHost || callbacks_.supportsHost(host));}
  bool open() {
    if(open_)return current();
    if(!available() || !callbacks_.focused())return false;
    const auto generation=callbacks_.generation();if(!generation)return false;
    if(!callbacks_.begin())return false;
    if(!available() || callbacks_.generation()!=generation)return false;
    epoch_=generation;host_.reset();open_=true;return true;
  }
  void close() {const bool wasOpen=open_;open_=false;epoch_=0;host_.reset();if(wasOpen && callbacks_.closed)callbacks_.closed();}
  bool isOpen() const {return open_;}
  bool current() const {return open_ && epoch_ && available() && callbacks_.focused() && callbacks_.generation()==epoch_;}
  bool select(ShortcutHost host) {if(!supports(host) || !current())return false;host_=host;return true;}
  std::optional<ShortcutHost> host() const {return host_;}
  bool send(std::string_view id) {
    if(!current() || !host_ || sending_)return false;
    for(const auto& action:sessionShortcuts(*host_))if(action.id==id) {
      sending_=true;
      struct Clear {bool& value;~Clear(){value=false;}} clear{sending_};
      // Only the chosen fixed profile reaches the native input engine.
      return callbacks_.submit(action.keys,epoch_);
    }
    return false;
  }
private:
  Callbacks callbacks_;
  bool open_=false,sending_=false;
  std::uint32_t epoch_=0;
  std::optional<ShortcutHost> host_;
};
}
