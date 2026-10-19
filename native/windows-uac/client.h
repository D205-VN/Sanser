#pragma once
#include <windows.h>
#include <memory>
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
UINT sendInput(UINT count,INPUT* inputs,int size);
} // namespace sanser::uac
