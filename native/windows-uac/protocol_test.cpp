#include "protocol.h"
#include "input_state.h"
#include <iostream>
#include <vector>
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
  request.flags=0x80; request.data=0xffffffff; check(!valid(request));
  request.data=1; check(valid(request));
  Response response; response.status=Status::Frame; response.width=1920; response.height=1080;
  response.stride=7680; response.bytes=7680*1080; check(validFrame(response));
  --response.bytes; check(!validFrame(response)); response.bytes=0xffffffff; check(!validFrame(response));
  response.width=0xffffffff; response.height=0xffffffff; response.stride=0xfffffffc; check(!validFrame(response));
  InputState state;
  Request shift; shift.operation=Operation::Input; shift.type=1; shift.flags=8; shift.data=0x2a;
  state.applied(shift); state.applied(shift); // A repeated down must release once.
  auto right=shift; right.data=0x36; state.applied(right);
  auto a=shift; a.data=0x1e; state.applied(a); a.flags|=2; state.applied(a);
  Request button; button.operation=Operation::Input; button.flags=0xc003; button.x=60000; button.y=42000;
  state.applied(button);
  std::vector<Request> cleanup;
  state.release([&](const Request& event) { check(valid(event)); cleanup.push_back(event); });
  check(cleanup.size()==3);
  unsigned shifts=0,buttons=0;
  for(const auto& event:cleanup) {
    if(event.type==1) { check(event.flags==10 && (event.data==0x2a || event.data==0x36)); ++shifts; }
    else { check(event.flags==4 && !event.x && !event.y); ++buttons; }
  }
  check(shifts==2 && buttons==1);
  state.release([&](const Request&) { throw std::runtime_error("Disconnect released input twice"); });
  std::cout<<"UAC IPC: bounded frames, operation allowlist, scan keys, coordinates and malformed requests passed\n";
}
