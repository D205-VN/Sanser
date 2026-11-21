#pragma once
// Host-local "take back control" hotkey, shared by the Windows and macOS
// hosts: Ctrl+Alt+K on Windows, Control+Command+K on macOS.
//
// Pressing it at the host computer turns remote input off; pressing it again
// turns it back on. Only the person at the host can do either; every new
// connection starts with control on. While off, only key and button releases
// (and the viewer's "release everything" reset) still pass, so nothing the
// viewer was holding stays pressed: no moves, clicks, scrolls, keys, text or
// controller input. The screen keeps streaming, and viewers are told (SRC1
// keepalive) so they hide their pointer and stop sending input.
#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <iostream>

namespace sanser::cutoff {
inline std::atomic<bool>& flag() { static std::atomic<bool> value{false}; return value; }
inline bool revoked() { return flag().load(std::memory_order_relaxed); }
// True only when this call turned control off.
inline bool revoke() { return !flag().exchange(true); }
// The hotkey: flips the state and returns true when control is now off.
inline bool toggle() {
  bool current=flag().load();
  while(!flag().compare_exchange_weak(current,!current)) {}
  return !current;
}
// For tests.
inline void reset() { flag().store(false); }
// While revoked, only releases pass.
inline bool allows(bool release) { return !revoked() || release; }
// Diagnostics on stderr; the desktop shell reads the private stdout record
// and tells the viewer through chat.
inline void announce(bool off=true) {
  std::cerr<<"SNCONTROL remote="<<(off ? "revoked":"restored")<<std::endl;
  if(std::getenv("SANSER_CONTROL_STDIN")) { std::fputs(off ? "SNUSER_CONTROL\trevoked\n":"SNUSER_CONTROL\trestored\n",stdout); std::fflush(stdout); }
}
// Host notice text, Vietnamese then English.
inline const char* notice(bool off) {
  return off ? "\xC4\x90\xC3\xA3 t\xE1\xBA\xAFt \xC4\x91i\xE1\xBB\x81u khi\xE1\xBB\x83n t\xE1\xBB\xAB xa \xC2\xB7 Remote control off"
             : "\xC4\x90\xC3\xA3 b\xE1\xBA\xADt l\xE1\xBA\xA1i \xC4\x91i\xE1\xBB\x81u khi\xE1\xBB\x83n t\xE1\xBB\xAB xa \xC2\xB7 Remote control on";
}
}
