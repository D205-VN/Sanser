#pragma once
// Host display list and viewer display selection for SNV2 control sessions.
//
// Both messages are optional authenticated Keepalive extensions on stream 0,
// like the other SNV2 capability hints. Older peers ignore them, so a viewer
// that never receives SDL1 keeps the existing single-display behaviour.
//
//   SDL1 host -> viewer: 'S','D','L','1', selected u8, count u8,
//        count * { width u16be, height u16be, flags u8 (bit0 primary) }
//   SDS1 viewer -> host: 'S','D','S','1', index u8
//
// The host owns the list and validates every request against it. A viewer
// can only ask for an index the host itself advertised.
#include <chrono>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace sanser::desktop::display {
using Bytes = std::vector<std::uint8_t>;
inline constexpr std::size_t maxDisplays = 16;

struct Info {
  std::uint16_t width = 0, height = 0;
  bool primary = false;
  bool operator==(const Info&) const = default;
};
struct List {
  std::uint8_t selected = 0;
  std::vector<Info> displays;
  bool operator==(const List&) const = default;
  bool valid() const {
    if(displays.empty() || displays.size()>maxDisplays || selected>=displays.size()) return false;
    for(const auto& item:displays) if(!item.width || !item.height) return false;
    return true;
  }
};

inline Bytes encodeList(const List& list) {
  if(!list.valid()) return {};
  Bytes out{'S','D','L','1',list.selected,static_cast<std::uint8_t>(list.displays.size())};
  for(const auto& item:list.displays) {
    out.push_back(static_cast<std::uint8_t>(item.width>>8)); out.push_back(static_cast<std::uint8_t>(item.width));
    out.push_back(static_cast<std::uint8_t>(item.height>>8)); out.push_back(static_cast<std::uint8_t>(item.height));
    out.push_back(item.primary ? 1 : 0);
  }
  return out;
}
inline bool isList(const Bytes& b) { return b.size()>=6 && b[0]=='S' && b[1]=='D' && b[2]=='L' && b[3]=='1'; }
inline std::optional<List> decodeList(const Bytes& b) {
  if(!isList(b)) return std::nullopt;
  const std::size_t count=b[5];
  if(!count || count>maxDisplays || b.size()!=6+count*5) return std::nullopt;
  List list; list.selected=b[4];
  for(std::size_t i=0;i<count;++i) {
    const auto* p=b.data()+6+i*5;
    if(p[4]>1) return std::nullopt;
    list.displays.push_back(Info{static_cast<std::uint16_t>((p[0]<<8)|p[1]),static_cast<std::uint16_t>((p[2]<<8)|p[3]),p[4]==1});
  }
  if(!list.valid()) return std::nullopt;
  return list;
}
inline Bytes encodeSelect(std::uint8_t index) { return Bytes{'S','D','S','1',index}; }
inline std::optional<std::uint8_t> decodeSelect(const Bytes& b) {
  if(b.size()!=5 || b[0]!='S' || b[1]!='D' || b[2]!='S' || b[3]!='1') return std::nullopt;
  return b[4];
}

// Desktop shell stdin command: "display\t<index>".
inline std::optional<std::uint8_t> parseCommand(const std::string& line) {
  constexpr std::string_view prefix="display\t";
  if(line.size()<=prefix.size() || line.size()>prefix.size()+2 || line.compare(0,prefix.size(),prefix)) return std::nullopt;
  unsigned value=0;
  for(std::size_t i=prefix.size();i<line.size();++i) {
    if(line[i]<'0' || line[i]>'9') return std::nullopt;
    value=value*10+static_cast<unsigned>(line[i]-'0');
  }
  if(value>=maxDisplays) return std::nullopt;
  return static_cast<std::uint8_t>(value);
}
// Desktop shell stdout record. "-" means the host offers no display list
// (an older host, a multi-viewer share, or before the first announcement).
inline std::string formatRecord(const std::optional<List>& list) {
  if(!list) return "SNUSER_DISPLAY\t-\t\n";
  std::string out="SNUSER_DISPLAY\t"+std::to_string(list->selected)+"\t";
  for(std::size_t i=0;i<list->displays.size();++i) {
    const auto& item=list->displays[i];
    if(i) out+=',';
    out+=std::to_string(item.width)+"x"+std::to_string(item.height)+"x"+(item.primary ? "1" : "0");
  }
  return out+"\n";
}

// Viewer-side retry: Keepalive is unreliable, so a pending choice is resent
// until the host's list reports it or a bounded deadline passes.
class Request {
public:
  using Clock=std::chrono::steady_clock;
  bool choose(std::uint8_t index,const std::optional<List>& host,Clock::time_point now) {
    if(!host || index>=host->displays.size()) return false;
    if(index==host->selected) { pending_.reset(); return true; }
    pending_=index; started_=now; return true;
  }
  // Index to (re)send now, if any.
  std::optional<std::uint8_t> due(const std::optional<List>& host,Clock::time_point now) {
    if(!pending_) return std::nullopt;
    if(!host || *pending_>=host->displays.size() || host->selected==*pending_ || now-started_>std::chrono::seconds(5)) {
      pending_.reset(); return std::nullopt;
    }
    return pending_;
  }
  void reset() { pending_.reset(); }
  std::optional<std::uint8_t> pending() const { return pending_; }
private:
  std::optional<std::uint8_t> pending_;
  Clock::time_point started_{};
};
}
