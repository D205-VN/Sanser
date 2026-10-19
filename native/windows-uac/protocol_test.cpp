#include "protocol.h"
#include <iostream>
#include <stdexcept>
using namespace sanser::uac;
void check(bool ok) { if(!ok) throw std::runtime_error("Invalid UAC IPC validation"); }
int main() {
  Request request; check(valid(request)); request.adapter=8; check(!valid(request));
  request=Request{}; request.signature=0; check(!valid(request));
  request=Request{}; request.operation=Operation::Input; request.type=1; request.flags=8; request.data=0x2a;
  check(valid(request)); request.flags=4; check(!valid(request));
  request.flags=8; request.data=256; check(!valid(request));
  request=Request{}; request.operation=Operation::Input; request.flags=0xc001; request.x=65535; request.y=0;
  check(valid(request)); request.x=-1; check(!valid(request));
  request.x=0; request.flags=0xffffffff; check(!valid(request));
  Response response; response.status=Status::Frame; response.width=1920; response.height=1080;
  response.stride=7680; response.bytes=7680*1080; check(validFrame(response));
  --response.bytes; check(!validFrame(response)); response.bytes=0xffffffff; check(!validFrame(response));
  response.width=0xffffffff; response.height=0xffffffff; response.stride=0xfffffffc; check(!validFrame(response));
  std::cout<<"UAC IPC: bounded frames, operation allowlist, scan keys, coordinates and malformed requests passed\n";
}
