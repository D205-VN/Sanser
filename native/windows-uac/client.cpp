#include "client.h"
#include "protocol.h"
#include "win_util.h"
#include "desktop_duplication.h"
#include <atomic>
#include <iostream>

namespace sanser::uac {
namespace {
std::atomic<ULONGLONG> servedUntil{0};
DWORD integrityLevel(HANDLE process) {
  HANDLE raw=nullptr;
  if(!OpenProcessToken(process,TOKEN_QUERY,&raw)) return 0;
  Handle token(raw); DWORD size=0;
  GetTokenInformation(token.value,TokenIntegrityLevel,nullptr,0,&size);
  std::vector<unsigned char> bytes(size);
  if(!size || !GetTokenInformation(token.value,TokenIntegrityLevel,bytes.data(),size,&size)) return 0;
  const auto* label=reinterpret_cast<TOKEN_MANDATORY_LABEL*>(bytes.data());
  const auto sid=label->Label.Sid;
  if(!IsValidSid(sid) || !*GetSidSubAuthorityCount(sid)) return 0;
  return *GetSidSubAuthority(sid,*GetSidSubAuthorityCount(sid)-1);
}
bool elevatedForeground(InputDispatch* dispatch) {
  const static DWORD own=integrityLevel(GetCurrentProcess());
  const static bool protectedHost=[] {
    try { return _wcsicmp(imagePath().c_str(),(installDirectory()/L"sanser-host-windows.exe").c_str())==0; }
    catch(...) { return false; }
  }();
  DWORD pid=0; GetWindowThreadProcessId(GetForegroundWindow(),&pid);
  if(dispatch) dispatch->sourceIntegrity=own;
  if(!pid || pid==GetCurrentProcessId()) return false;
  thread_local DWORD previous=0;
  thread_local ULONGLONG checked=0;
  thread_local bool elevated=false;
  thread_local DWORD target=0;
  const auto now=GetTickCount64();
  if(pid!=previous || now-checked>=50) {
    Handle process(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,FALSE,pid));
    target=process ? integrityLevel(process.value) : 0;
    // A denied token query means unknown, not unelevated. Only the protected
    // host authorized for the broker uses privileged input for unknown targets;
    // ordinary/uninstalled hosts retain their normal input path.
    elevated=(own && target && target>own) || ((!own || !target) && protectedHost);
    previous=pid; checked=now;
  }
  if(dispatch) dispatch->targetIntegrity=target;
  return elevated;
}
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
UINT sendInput(UINT count,INPUT* inputs,int size,InputDispatch* dispatch) {
  if(dispatch) *dispatch={};
  if(size!=sizeof(INPUT) || count>16) return 0;
  thread_local Connection connection;
  // Ordinary input remains in-process. The helper is also needed for a high-
  // integrity installer window after its UAC prompt returns to Default.
  // An elevated installer on Default is also privileged, even though capture
  // has already returned from Winlogon. Route it deliberately, rather than
  // interpreting a low-integrity SendInput result as permission to control it.
  UINT sent=(active() || elevatedForeground(dispatch)) ? 0 : ::SendInput(count,inputs,size);
  if(dispatch) dispatch->direct=sent;
  if(sent==count) return sent;
  for(;sent<count;++sent) {
    const auto& in=inputs[sent]; Request request; request.operation=Operation::Input; request.type=in.type;
    if(in.type==INPUT_KEYBOARD) { request.flags=in.ki.dwFlags; request.data=(in.ki.dwFlags&KEYEVENTF_SCANCODE)?in.ki.wScan:in.ki.wVk; }
    else if(in.type==INPUT_MOUSE) { request.flags=in.mi.dwFlags; request.x=in.mi.dx; request.y=in.mi.dy; request.data=in.mi.mouseData; }
    if(!valid(request)) break;
    Response response;
    if(!connection.exchange(request,response)) break;
    if(response.status==Status::Unavailable) {
      if(dispatch) dispatch->failureStatus=static_cast<DWORD>(response.status);
      // UAC closed between capture and input. A positively rejected event can
      // safely use the normal desktop; never replay a timed-out transaction.
      const auto direct=::SendInput(count-sent,inputs+sent,size);
      if(dispatch) dispatch->direct+=direct;
      return sent+direct;
    }
    if(response.status!=Status::Applied) {
      if(dispatch) { dispatch->failureStatus=static_cast<DWORD>(response.status); dispatch->failureError=static_cast<DWORD>(response.left); }
      thread_local ULONGLONG lastFailure=0;
      const auto now=GetTickCount64();
      if(!lastFailure || now-lastFailure>=1000) {
        std::cerr<<"SNUAC_INPUT_FAILED status="<<static_cast<DWORD>(response.status)<<" win32="<<response.left<<'\n';
        lastFailure=now;
      }
      break;
    }
    if(dispatch) ++dispatch->privileged;
  }
  return sent;
}
} // namespace sanser::uac
