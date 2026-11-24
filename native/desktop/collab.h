#pragma once
#include "user_ipc.h"
#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace sanser::desktop {
// In-session chat and annotation. The native engine only moves bounded opaque
// payloads over the authenticated SUD1 user-data lane; the desktop shell owns
// parsing, plain-text validation, rate policy, relaying and the session-only
// history. Nothing here is written to disk or to diagnostics.
//
// Capability keepalive: "SCO1" (both directions, optional; old peers ignore it).
// Message body inside SUD1: "SCL1" + payload.
// stdin:  collab-send\t<target>\t<hex>   target "-" (single peer), "*" (every
//                                        sharing viewer) or a sharing viewer id
//         collab-exclude\t<0|1>          macOS host: exclude the local overlay
// stdout: SNUSER_COLLAB\t<source>\t<hex>  source "-" or the sharing viewer id
//         SNUSER_COLLAB_CAP\t<source>\t<0|1>
inline constexpr std::size_t collabMaxPayload=4096;
inline constexpr std::uint8_t collabMagic[4]={'S','C','L','1'};

inline bool collabTarget(std::string_view id) {
  if(id=="-" || id=="*") return true;
  return !id.empty() && id.size()<=64 && std::all_of(id.begin(),id.end(),[](unsigned char c) { return (c>='0'&&c<='9')||(c>='a'&&c<='z')||(c>='A'&&c<='Z')||c=='-'||c=='_'; });
}
struct CollabCommand { std::string target; std::vector<std::uint8_t> payload; };
inline bool isCollabCommand(std::string_view line) { return line.starts_with("collab-send\t"); }
inline std::optional<CollabCommand> parseCollabCommand(std::string_view line) {
  CollabCommand command;
  if(!line.starts_with("collab-send\t")) return std::nullopt;
  line.remove_prefix(12);
  const auto tab=line.find('\t');
  if(tab==std::string_view::npos || !collabTarget(line.substr(0,tab))) return std::nullopt;
  command.target=std::string(line.substr(0,tab));
  const auto hex=line.substr(tab+1);
  if(hex.empty() || hex.size()>collabMaxPayload*2) return std::nullopt;
  const auto body=userUnhex(hex); if(!body || body->empty() || body->size()>collabMaxPayload) return std::nullopt;
  command.payload.assign(body->begin(),body->end());
  return command;
}
inline std::vector<std::uint8_t> collabBody(const std::vector<std::uint8_t>& payload) {
  std::vector<std::uint8_t> body(collabMagic,collabMagic+4); body.insert(body.end(),payload.begin(),payload.end()); return body;
}
inline std::optional<std::vector<std::uint8_t>> collabPayload(const std::vector<std::uint8_t>& body) {
  if(body.size()<5 || body.size()>collabMaxPayload+4 || !std::equal(body.begin(),body.begin()+4,collabMagic)) return std::nullopt;
  return std::vector<std::uint8_t>(body.begin()+4,body.end());
}
// Coarse per-peer flood guard below the shell's own chat/stroke policy.
class CollabRate {
public:
  using Clock=std::chrono::steady_clock;
  static constexpr double perSecond=30,burst=30;
  bool take(Clock::time_point now=Clock::now()) {
    if(last_!=Clock::time_point{}) tokens_=std::min(burst,tokens_+std::chrono::duration<double>(now-last_).count()*perSecond);
    last_=now;
    if(tokens_<1) return false;
    tokens_-=1; return true;
  }
private:
  double tokens_=burst; Clock::time_point last_{};
};
// Native viewer geometry so the shell can align its click-through annotation
// overlay over the aspect-fit video. unit "pt" = macOS points with a top-left
// origin on the primary screen; "px" = Windows physical screen pixels.
class CollabViewReport {
public:
  using Clock=std::chrono::steady_clock;
  struct View { int x=0,y=0,width=0,height=0; unsigned videoWidth=0,videoHeight=0; bool visible=false; bool operator==(const View&) const=default; };
  // Changes are reported at most every 50 ms; an unchanged view every 2 s.
  bool due(const View& view,Clock::time_point now=Clock::now()) {
    if(last_ && *last_==view ? now-at_<std::chrono::seconds(2) : now-at_<std::chrono::milliseconds(50)) return false;
    last_=view; at_=now; return true;
  }
  static std::string line(const View& view,const char* unit) {
    return "SNUSER_COLLAB_VIEW\t"+std::to_string(view.x)+"\t"+std::to_string(view.y)+"\t"+std::to_string(view.width)+"\t"+std::to_string(view.height)+"\t"
      +std::to_string(view.videoWidth)+"\t"+std::to_string(view.videoHeight)+"\t"+(view.visible ? "1":"0")+"\t"+unit+"\n";
  }
  void publish(const View& view,const char* unit,Clock::time_point now=Clock::now()) {
    if(!due(view,now)) return;
    const auto text=line(view,unit); std::fputs(text.c_str(),stdout); std::fflush(stdout);
  }
private:
  std::optional<View> last_; Clock::time_point at_{};
};
// Chat in the native floating panel. The shell stays the only chat hub: it
// pushes each chat it records and validates/sends what the panel posts.
// stdin:  panel-chat\t<0|1 own>\t<hex name>\t<hex text>
// stdout: SNUSER_PANEL_CHAT\t<hex text>
inline constexpr std::size_t panelChatMaxText=1024,panelChatMaxName=64*4;
struct PanelChatLine { bool own=false; std::string name,text; };
inline bool isPanelChatCommand(std::string_view line) { return line.starts_with("panel-chat\t"); }
inline std::optional<PanelChatLine> parsePanelChat(std::string_view line) {
  if(!isPanelChatCommand(line)) return std::nullopt;
  line.remove_prefix(11);
  const auto a=line.find('\t'); if(a!=1 || (line[0]!='0' && line[0]!='1')) return std::nullopt;
  const auto rest=line.substr(2); const auto b=rest.find('\t'); if(b==std::string_view::npos) return std::nullopt;
  auto name=userUnhex(rest.substr(0,b)); auto text=userUnhex(rest.substr(b+1));
  if(!name || !text || text->empty() || text->size()>panelChatMaxText || name->size()>panelChatMaxName) return std::nullopt;
  for(auto& c:*name) if(static_cast<unsigned char>(c)<32) c=' ';
  for(auto& c:*text) if(static_cast<unsigned char>(c)<32 && c!='\n') c=' ';
  return PanelChatLine{line[0]=='1',std::move(*name),std::move(*text)};
}
inline std::string panelChatRecord(std::string_view text) { return "SNUSER_PANEL_CHAT\t"+userHex(text)+"\n"; }
inline void panelChatSend(std::string_view text) {
  if(text.empty() || text.size()>panelChatMaxText) return;
  const auto line=panelChatRecord(text); std::fputs(line.c_str(),stdout); std::fflush(stdout);
}
// A file dropped on the remote-desktop window. The shell checks the path,
// copies the file privately and offers it through the consented transfer.
constexpr std::size_t fileDropMaxPath=4096;
inline std::string fileDropRecord(std::string_view path) { return "SNUSER_FILE_DROP\t"+userHex(path)+"\n"; }
inline void fileDropSend(std::string_view path) {
  if(path.empty() || path.size()>fileDropMaxPath) return;
  const auto line=fileDropRecord(path); std::fputs(line.c_str(),stdout); std::fflush(stdout);
}
inline std::string collabSource(const std::string& id) { return id.empty() ? std::string("-") : id; }
inline void collabRecord(const std::string& source,const std::vector<std::uint8_t>& payload) {
  const auto encoded=userHex(std::string_view(reinterpret_cast<const char*>(payload.data()),payload.size()));
  std::fprintf(stdout,"SNUSER_COLLAB\t%s\t%s\n",collabSource(source).c_str(),encoded.c_str()); std::fflush(stdout);
}
inline void collabCapability(const std::string& source,bool available) {
  std::fprintf(stdout,"SNUSER_COLLAB_CAP\t%s\t%d\n",collabSource(source).c_str(),available ? 1:0); std::fflush(stdout);
}
}
