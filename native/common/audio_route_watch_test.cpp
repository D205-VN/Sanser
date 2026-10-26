#include "audio_route_watch.h"
#include <iostream>
#include <stdexcept>
int main() {
  using namespace std::chrono;
  using Watch=sanser::audio::LoopbackRouteWatch;
  const auto start=Watch::Clock::time_point{};
  Watch watch(L"headphones",start);
  unsigned queries=0;
  std::optional<std::wstring> endpoint=L"headphones";
  auto query=[&] { ++queries; return endpoint; };
  for(int ms=0;ms<40000;ms+=5) {
    if(ms==20000) endpoint=L"speakers";
    const bool changed=watch.changed(start+milliseconds(ms),query);
    if(changed!=(ms>=20000 && ms%250==0)) throw std::runtime_error("Route change was missed or polled for every PCM block");
  }
  if(queries!=159) throw std::runtime_error("Unexpected route polling cadence");
  Watch recovery(L"speakers",start);
  endpoint.reset();
  if(!recovery.changed(start+milliseconds(250),query)) throw std::runtime_error("Temporary no-device state must restart capture");
  endpoint=L"speakers";
  if(recovery.changed(start+milliseconds(500),query)) throw std::runtime_error("Restored endpoint should remain stable");
  std::cout<<"Loopback route: forty-second stream, hot-unplug, absent/restored default and bounded queries passed\n";
}
