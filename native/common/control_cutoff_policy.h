#pragma once
// Host-local "take back control" hotkey, shared by the Windows and macOS
// hosts: Ctrl+Alt+K on Windows, Control+Command+K on macOS.
//
// Pressing it at the host computer ends remote input for the rest of the
// session (this host engine process). It cannot be undone from either side;
// the next connection starts with control again. Only key and button releases
// (and the viewer's "release everything" reset) still pass, so nothing the
// viewer was holding stays pressed. The screen keeps streaming.
#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <iostream>

namespace sanser::cutoff {
inline std::atomic<bool>& flag() { static std::atomic<bool> value{false}; return value; }
inline bool revoked() { return flag().load(std::memory_order_relaxed); }
// True only for the first press, so the announcement is printed once.
inline bool revoke() { return !flag().exchange(true); }
// For tests.
inline void reset() { flag().store(false); }
// While revoked, only releases pass.
inline bool allows(bool release) { return !revoked() || release; }
// Diagnostics on stderr; the desktop shell reads the private stdout record
// and tells the viewer through chat.
inline void announce() {
  std::cerr<<"SNCONTROL remote=revoked"<<std::endl;
  if(std::getenv("SANSER_CONTROL_STDIN")) { std::fputs("SNUSER_CONTROL\trevoked\n",stdout); std::fflush(stdout); }
}
}
