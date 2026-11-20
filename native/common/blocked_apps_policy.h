#pragma once
// Host "blocked apps": applications the remote person must not see or use.
//
// The list comes only from the host's own launch arguments (--blocked-app,
// one per application), never from the viewer. While the list is not empty:
//  - capture hides the blocked applications' windows (macOS excludes them
//    from ScreenCaptureKit; Windows paints their visible area black);
//  - remote key presses are dropped while a blocked application is in front,
//    and remote clicks/scrolls are dropped over a blocked window.
// Releases (key up, button up) and pointer moves always pass, so nothing can
// stay stuck down. Modifiers pass, and Tab passes while Command/Alt is held,
// so the remote person can switch away with Cmd+Tab / Alt+Tab.
//
// Identifiers: macOS bundle identifier (com.google.Chrome), Windows
// executable file name (chrome.exe). Matching ignores ASCII case.
// This is a deterrent for a person with full control, not a sandbox: they
// can still rename or copy an executable. See docs/blocked-apps.md.
#include <algorithm>
#include <cctype>
#include <cstddef>
#include <cstdint>
#include <stdexcept>
#include <string>
#include <string_view>
#include <vector>

namespace sanser::blocked {
inline constexpr std::size_t maxApps=64;
inline constexpr std::size_t maxIdLength=255;

inline std::string lower(std::string_view value) {
  std::string out(value);
  std::transform(out.begin(),out.end(),out.begin(),[](unsigned char c) { return static_cast<char>(std::tolower(c)); });
  return out;
}

// Validates one --blocked-app value. Throws on anything that is not a plain
// identifier, so a malformed preference cannot reach platform APIs.
inline std::string normalizeId(std::string_view value) {
  while(!value.empty() && value.front()==' ') value.remove_prefix(1);
  while(!value.empty() && value.back()==' ') value.remove_suffix(1);
  if(value.empty() || value.size()>maxIdLength) throw std::runtime_error("Blocked app identifier must be 1-255 characters");
  for(const unsigned char c:value) {
    if(c<0x20 || c==0x7f || c=='/' || c=='\\' || c==':' || c=='*' || c=='?' || c=='"' || c=='<' || c=='>' || c=='|')
      throw std::runtime_error("Blocked app identifier contains an invalid character");
  }
  return lower(value);
}

class List {
public:
  void add(std::string_view id) {
    auto value=normalizeId(id);
    if(std::find(ids_.begin(),ids_.end(),value)!=ids_.end()) return;
    if(ids_.size()>=maxApps) throw std::runtime_error("At most 64 blocked apps are supported");
    ids_.push_back(std::move(value));
  }
  bool empty() const { return ids_.empty(); }
  std::size_t size() const { return ids_.size(); }
  const std::vector<std::string>& ids() const { return ids_; }
  bool matches(std::string_view id) const {
    if(id.empty()) return false;
    const auto value=lower(id);
    return std::find(ids_.begin(),ids_.end(),value)!=ids_.end();
  }
  // Windows: matches the file name of a full image path (C:\...\chrome.exe).
  bool matchesPath(std::string_view path) const {
    const auto slash=path.find_last_of("\\/");
    return matches(slash==std::string_view::npos ? path : path.substr(slash+1));
  }
private:
  std::vector<std::string> ids_;
};

// One remote key event, classified by the platform caller.
struct Key {
  bool down=false;
  bool modifier=false;  // Shift, Control, Alt/Option, Command/Windows
  bool switcher=false;  // Alt/Option or Command/Windows (app-switch chord)
  bool tab=false;
};

// Decides which remote events reach the operating system. Single-threaded:
// call from the input thread only.
class InputGate {
public:
  // `blockedFront`: a blocked application owns the keyboard focus.
  bool allowKey(const Key& key,bool blockedFront) {
    if(key.switcher) { if(key.down) ++switchers_; else if(switchers_>0) --switchers_; }
    if(!key.down || key.modifier) return true;
    if(!blockedFront) return true;
    if(key.tab && switchers_>0) return true;  // Cmd+Tab / Alt+Tab away
    ++dropped_;
    return false;
  }
  // `press`: a button down or a wheel step; `blockedTarget`: the pointer is
  // over a blocked application's window.
  bool allowPointer(bool press,bool blockedTarget) {
    if(!press || !blockedTarget) return true;
    ++dropped_;
    return false;
  }
  void reset() { switchers_=0; }
  std::uint64_t dropped() const { return dropped_; }
private:
  unsigned switchers_=0;
  std::uint64_t dropped_=0;
};

// HID usage classification for the SNV2 Input key codes (macOS host).
inline Key hidKey(std::uint16_t usage,bool down) {
  Key key; key.down=down;
  key.modifier=usage>=224 && usage<=231;
  key.switcher=usage==226 || usage==227 || usage==230 || usage==231;
  key.tab=usage==43;
  return key;
}

// Axis-aligned rectangle in pixels, right/bottom exclusive.
struct Rect { long left=0,top=0,right=0,bottom=0; };

// Clips rectangles in virtual-screen coordinates to one output and makes them
// output-local. Empty results are dropped.
inline std::vector<Rect> toOutput(const std::vector<Rect>& rects,long left,long top,long width,long height) {
  std::vector<Rect> out;
  for(const auto& r:rects) {
    Rect local{std::max(0L,r.left-left),std::max(0L,r.top-top),std::min(width,r.right-left),std::min(height,r.bottom-top)};
    if(local.right>local.left && local.bottom>local.top) out.push_back(local);
  }
  return out;
}

// Paints rectangles black in a tightly or loosely packed BGRA buffer.
inline void paintBlack(std::uint8_t* pixels,std::size_t stride,long width,long height,const std::vector<Rect>& rects) {
  for(const auto& r:rects) {
    const long left=std::clamp(r.left,0L,width),right=std::clamp(r.right,0L,width);
    const long top=std::clamp(r.top,0L,height),bottom=std::clamp(r.bottom,0L,height);
    for(long y=top;y<bottom;++y) {
      auto* row=pixels+stride*static_cast<std::size_t>(y);
      for(long x=left;x<right;++x) { auto* p=row+static_cast<std::size_t>(x)*4; p[0]=0; p[1]=0; p[2]=0; p[3]=255; }
    }
  }
}
}
