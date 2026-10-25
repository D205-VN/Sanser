#pragma once
#include <windows.h>
#include <memory>
#include <cstdint>
struct FrameBgra;
namespace sanser::uac {
class CaptureClient {
public:
  CaptureClient();
  ~CaptureClient();
  // True means the helper is serving the current UAC desktop, including idle.
  bool capture(FrameBgra& frame,unsigned adapter,unsigned output,bool& changed);
private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};
bool active();
void resumed();
// Bounded numeric diagnostics only. Last failure can predate later success.
struct Health {
  std::uint64_t failures=0,readyConnections=0,captureFrames=0,captureIdle=0;
  std::uint32_t lastStage=0,lastError=0,lastStatus=0;
};
Health health();
struct InputDispatch { UINT direct=0,privileged=0; DWORD sourceIntegrity=0,targetIntegrity=0,failureStatus=0,failureError=0; };
UINT sendInput(UINT count,INPUT* inputs,int size,InputDispatch* dispatch=nullptr);
} // namespace sanser::uac
