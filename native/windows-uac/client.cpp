#include "client.h"
#include "protocol.h"
#include "win_util.h"
#include "desktop_duplication.h"
#include <atomic>

namespace sanser::uac {
namespace {
std::atomic<ULONGLONG> servedUntil{0};
class Connection {
public:
  bool exchange(Request request,Response& response,FrameBgra* frame=nullptr) {
    if(GetTickCount64()<retryAt_) return false;
    try {
      if(!pipe_) {
        pipe_=Handle(CreateFileW(pipeName,GENERIC_READ|GENERIC_WRITE,0,nullptr,OPEN_EXISTING,
          FILE_FLAG_OVERLAPPED|SECURITY_SQOS_PRESENT|SECURITY_IDENTIFICATION,nullptr));
        if(!pipe_ || !trustedServer()) return failed();
      }
      if(!timedTransfer(pipe_.value,&request,sizeof(request),true,250)
          || !timedTransfer(pipe_.value,&response,sizeof(response),false,1000)
          || response.signature!=magic) return failed();
      if(response.status==Status::Frame) {
        if(!frame || !validFrame(response)) return failed();
        frame->pixels.resize(response.bytes);
        if(!timedTransfer(pipe_.value,frame->pixels.data(),response.bytes,false,1000)) return failed();
        frame->width=response.width; frame->height=response.height; frame->stride=response.stride;
        frame->texture.reset();
      } else if(response.bytes) return failed();
      return true;
    } catch(...) { return failed(); }
  }
private:
  bool trustedServer() {
    ULONG pid=0; if(!GetNamedPipeServerProcessId(pipe_.value,&pid)) return false;
    Handle process(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,FALSE,pid)); if(!process) return false;
    wchar_t path[32768]; DWORD length=32768;
    if(!QueryFullProcessImageNameW(process.value,0,path,&length)) return false;
    const auto expected=installDirectory()/L"sanser-uac-service.exe";
    if(_wcsicmp(std::wstring(path,length).c_str(),expected.c_str())) return false;
    // A medium-integrity host cannot query a SYSTEM process token. Ask SCM
    // for the running service PID instead of weakening the service's DACL.
    SC_HANDLE manager=OpenSCManagerW(nullptr,nullptr,SC_MANAGER_CONNECT); if(!manager) return false;
    SC_HANDLE service=OpenServiceW(manager,serviceName,SERVICE_QUERY_STATUS);
    SERVICE_STATUS_PROCESS status{}; DWORD bytes=0;
    const bool ok=service && QueryServiceStatusEx(service,SC_STATUS_PROCESS_INFO,
      reinterpret_cast<BYTE*>(&status),sizeof(status),&bytes)
      && status.dwCurrentState==SERVICE_RUNNING && status.dwProcessId==pid;
    if(service) CloseServiceHandle(service); CloseServiceHandle(manager); return ok;
  }
  bool failed() { pipe_=Handle(); retryAt_=GetTickCount64()+1000; return false; }
  Handle pipe_;
  ULONGLONG retryAt_=0;
};
}
struct CaptureClient::Impl { Connection connection; };
CaptureClient::CaptureClient():impl_(std::make_unique<Impl>()) {}
CaptureClient::~CaptureClient()=default;
bool CaptureClient::capture(FrameBgra& frame,unsigned adapter,unsigned output,bool& changed) {
  Request request; request.adapter=adapter; request.output=output; Response response;
  changed=false;
  if(!impl_->connection.exchange(request,response,&frame)
      || (response.status!=Status::Frame && response.status!=Status::Idle)) return false;
  servedUntil.store(GetTickCount64()+1500);
  changed=response.status==Status::Frame;
  return true;
}
bool active() { return GetTickCount64()<servedUntil.load(); }
void resumed() { servedUntil=0; }
UINT sendInput(UINT count,INPUT* inputs,int size) {
  if(!active()) return ::SendInput(count,inputs,size);
  if(size!=sizeof(INPUT) || count>16) return 0;
  thread_local Connection connection;
  UINT sent=0;
  for(;sent<count;++sent) {
    const auto& in=inputs[sent]; Request request; request.operation=Operation::Input; request.type=in.type;
    if(in.type==INPUT_KEYBOARD) { request.flags=in.ki.dwFlags; request.data=(in.ki.dwFlags&KEYEVENTF_SCANCODE)?in.ki.wScan:in.ki.wVk; }
    else if(in.type==INPUT_MOUSE) { request.flags=in.mi.dwFlags; request.x=in.mi.dx; request.y=in.mi.dy; request.data=in.mi.mouseData; }
    if(!valid(request)) break;
    Response response;
    if(!connection.exchange(request,response)) break;
    if(response.status==Status::Unavailable) {
      // UAC closed between capture and input. A positively rejected event can
      // safely use the normal desktop; never replay a timed-out transaction.
      return sent+::SendInput(count-sent,inputs+sent,size);
    }
    if(response.status!=Status::Applied) break;
  }
  return sent;
}
} // namespace sanser::uac
