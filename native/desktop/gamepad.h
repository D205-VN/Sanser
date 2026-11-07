#pragma once
#include <algorithm>
#include <chrono>
#include <cstdint>
#include <optional>
#include <span>
#include <vector>

namespace sanser::desktop {
// One Xbox-layout controller, Y positive upwards. Separate from keyboard HID.
struct GamepadState {
  bool connected=false;
  std::uint16_t buttons=0;
  std::uint8_t lt=0,rt=0;
  std::int16_t lx=0,ly=0,rx=0,ry=0;
  bool operator==(const GamepadState&) const=default;
};
inline bool validGamepad(const GamepadState& s) {
  return !(s.buttons & ~0xf3ffU) && (s.connected || s==GamepadState{});
}
inline std::vector<std::uint8_t> encodeGamepad(const GamepadState& s) {
  if(!validGamepad(s))return {};
  std::vector<std::uint8_t> b{'S','G','P','1',static_cast<std::uint8_t>(s.connected),0,
    static_cast<std::uint8_t>(s.buttons>>8),static_cast<std::uint8_t>(s.buttons),s.lt,s.rt};
  for(auto axis:{s.lx,s.ly,s.rx,s.ry}) {
    const auto n=static_cast<std::uint16_t>(axis);b.push_back(n>>8);b.push_back(n&255);
  }return b;
}
inline std::optional<GamepadState> decodeGamepad(std::span<const std::uint8_t> b) {
  if(b.size()!=18 || b[0]!='S' || b[1]!='G' || b[2]!='P' || b[3]!='1' || b[4]>1 || b[5])return {};
  GamepadState s;s.connected=b[4]!=0;s.buttons=static_cast<std::uint16_t>((b[6]<<8)|b[7]);s.lt=b[8];s.rt=b[9];
  const auto axis=[&](unsigned i){const int n=(b[i]<<8)|b[i+1];return static_cast<std::int16_t>(n<32768 ? n:n-65536);};
  s.lx=axis(10);s.ly=axis(12);s.rx=axis(14);s.ry=axis(16);
  return validGamepad(s) ? std::optional{s}:std::nullopt;
}
// Runs only on the OS-input worker. A stale/failed/disconnected capture must
// return to neutral before it can press anything in a replacement scope.
class GamepadLease {
public:
  using Clock=std::chrono::steady_clock;
  static constexpr auto lifetime=std::chrono::milliseconds(250);
  std::optional<GamepadState> accept(GamepadState s,Clock::time_point received,Clock::time_point now) {
    if(!validGamepad(s))return {};
    if(!s.connected || now-received>=lifetime) {reset();return GamepadState{};}
    if(active_ && now-at_>=lifetime)reset();
    const auto centered=[](std::int16_t n){return n>=-4096 && n<=4096;};
    if(!active_ && (s.buttons || s.lt>8 || s.rt>8 || !centered(s.lx) || !centered(s.ly) || !centered(s.rx) || !centered(s.ry)))return {};
    active_=true;at_=received;return s;
  }
  bool expired(Clock::time_point now) {if(!active_ || now-at_<lifetime)return false;reset();return true;}
  void reset(){active_=false;at_={};}
  bool active()const{return active_;}
  Clock::time_point deadline()const{return at_+lifetime;}
private:
  bool active_=false;Clock::time_point at_{};
};
}
