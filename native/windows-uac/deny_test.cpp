#include "protocol.h"
#include "win_util.h"
#include <iostream>
using namespace sanser::uac;
int main() {
  try {
    for(const auto operation:{Operation::Probe,Operation::Capture,Operation::Input}) {
      Handle pipe;
      for(unsigned attempt=0;attempt<20 && !pipe;++attempt) {
        pipe=Handle(CreateFileW(pipeName,GENERIC_READ|GENERIC_WRITE,0,nullptr,OPEN_EXISTING,
          FILE_FLAG_OVERLAPPED|SECURITY_SQOS_PRESENT|SECURITY_IDENTIFICATION,nullptr));
        if(!pipe) Sleep(10);
      }
      require(static_cast<bool>(pipe),"Connect test client to UAC service");
      Request request; Response response; request.operation=operation;
      if(operation==Operation::Input) { request.type=1; request.flags=8; request.data=0x1e; }
      timedTransfer(pipe.value,&request,sizeof(request),true,1000);
      // This executable is not the protected host. Even the side-effect-free
      // readiness probe must be rejected before a worker is created.
      require(!timedTransfer(pipe.value,&response,sizeof(response),false,1500),"UAC service accepted an unauthorized executable");
    }
    std::cout<<"Unauthorized executable rejected; no capture/input requested\n"; return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
