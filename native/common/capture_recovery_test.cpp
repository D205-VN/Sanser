#include "capture_recovery.h"
#include <iostream>
#include <stdexcept>
#ifdef _WIN32
#include <windows.h>
#include <dxgi.h>
#endif
using namespace sanser::capture;
void check(bool condition,const char* message) { if(!condition) throw std::runtime_error(message); }
int main() {
  try {
#ifdef _WIN32
    check(recoverable(static_cast<std::uint32_t>(DXGI_ERROR_ACCESS_DENIED)),"DXGI denial must recover, not only Win32 denial");
    check(recoverable(static_cast<std::uint32_t>(E_ACCESSDENIED)),"Win32 desktop denial must recover");
    check(recoverable(WAIT_ABANDONED),"abandoned desktop synchronization must recreate capture");
    check(!recoverable(static_cast<std::uint32_t>(DXGI_ERROR_WAIT_TIMEOUT)),"idle desktop must not recreate capture");
    check(!recoverable(static_cast<std::uint32_t>(DXGI_ERROR_INVALID_CALL)),"invalid API usage must remain visible");
#endif
    for(const auto result : {0x00000080U,0x80070005U,0x887a0005U,0x887a0007U,0x887a0020U,0x887a0022U,0x887a0026U,0x887a0028U,0x887a002bU}) {
      bool mapped=false,released=false,pending=false; unsigned attempts=0,frames=0;
      struct AcquiredFrame { bool& mapped; bool& released; ~AcquiredFrame() { mapped=false; released=true; } };
      const bool captured=guard([&]() -> bool {
        AcquiredFrame resource{mapped,released}; mapped=true;
        throw OperationError(result,"Map staging after a desktop/GPU transition");
      },[&](const OperationError&) {
        check(!mapped && released,"acquired resource must be released before recovery");
        pending=true; ++attempts;
      });
      check(!captured && pending && attempts==1,"recoverable capture loss must not exit its host loop");
      check(guard([&] { pending=false; ++frames; return true; },[&](const OperationError&) { ++attempts; }),"next valid capture must resume");
      check(frames==1 && !pending && attempts==1,"recovered frames must not stay paused");
    }
    unsigned recoveries=0;
    check(!guard([] { return false; },[&](const OperationError&) { ++recoveries; }),"ordinary idle poll has no picture");
    check(recoveries==0,"unchanged desktop must not cause a recovery loop");
    for(const auto result : {0x80070057U,0x8007000eU,0x887a0001U,0x887a0006U}) {
      bool failed=false;
      try { guard([&]() -> bool { throw OperationError(result,"invalid call, memory failure or GPU command bug"); },[&](const OperationError&) { ++recoveries; }); }
      catch(const OperationError&) { failed=true; }
      check(failed && recoveries==0,"programming/resource failures must not be hidden as transient desktop loss");
    }
    bool failed=false;
    try { guard([]() -> bool { throw std::runtime_error("corrupt capture bounds"); },[&](const OperationError&) { ++recoveries; }); }
    catch(const std::runtime_error&) { failed=true; }
    check(failed && recoveries==0,"non-DXGI failures must preserve their error");
    std::cout<<"Capture recovery: desktop/GPU transitions, cleanup before recreate, idle and fatal failures passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
