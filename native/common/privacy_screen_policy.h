#pragma once
// Host privacy screen decisions shared by the Windows and macOS hosts.
//
// The host's own launch arguments are the only input that can enable it. The
// overlay is shown only while an authenticated one-to-one control session is
// ready, and only when the platform confirmed the overlay is excluded from
// capture (otherwise the viewer would see the black overlay instead of the
// desktop). The local hotkey dismisses it for the rest of the session.
#include <string>

namespace sanser::privacy {
struct State {
  bool enabled=false;          // host preference, --privacy-screen
  bool blockRequested=false;   // host preference, --privacy-block-input
  bool sessionReady=false;     // authenticated viewer is connected
  bool captureExcluded=false;  // platform excluded the overlay from capture
  bool blockAvailable=false;   // elevated helper can block local input
  bool dismissed=false;        // local escape hotkey pressed this session
};
inline bool showOverlay(const State& s) { return s.enabled && s.sessionReady && s.captureExcluded && !s.dismissed; }
inline bool blockLocalInput(const State& s) { return showOverlay(s) && s.blockRequested && s.blockAvailable; }
// One stderr marker per visible change, for diagnostics (never user content).
inline std::string marker(const State& s) {
  if(!s.enabled) return {};
  std::string line="SNPRIVACY overlay=";
  line+=showOverlay(s) ? "shown" : s.dismissed ? "dismissed" : !s.captureExcluded ? "unavailable" : "waiting";
  line+=" blockInput=";
  line+=!s.blockRequested ? "off" : !s.blockAvailable ? "unavailable" : blockLocalInput(s) ? "on" : "idle";
  return line;
}
}
