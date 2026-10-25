#include "client.h"
#include "protocol.h"
#include "win_util.h"
#include "desktop_duplication.h"
#include <atomic>
#include <iostream>
#include <mutex>

namespace sanser::uac {
namespace {
std::atomic<ULONGLONG> servedUntil{0};
std::mutex healthMutex;
Health brokerHealth;
ULONGLONG lastFailureLog=0;
enum class Stage : std::uint32_t { Open=1,Verify=2,Ready=3,Request=4,Response=5,Frame=6,Rejected=7 };
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
        if(!pipe_) return failed(Stage::Open,GetLastError());
        const auto identity=verifyServiceEndpoint(pipe_.value);
        if(!identity.trusted) return failed(Stage::Verify,identity.error,Status::Unavailable,identity.check);
        // Worker creation can take up to 3 seconds. Establish readiness before
        // sending input, with a distinct bounded cold-start deadline. A failed
        // handshake has not submitted any click/key for execution.
        Request probe; probe.operation=Operation::Probe; Response ready;
        if(!timedTransfer(pipe_.value,&probe,sizeof(probe),true,250)
            || !timedTransfer(pipe_.value,&ready,sizeof(ready),false,readyTimeoutMs))
          return failed(Stage::Ready,GetLastError());
        if(ready.signature!=magic) return failed(Stage::Ready,ERROR_INVALID_DATA);
        if(ready.status!=Status::Ready || ready.bytes)
          return failed(Stage::Ready,static_cast<DWORD>(ready.left),ready.status);
        { std::lock_guard lock(healthMutex); ++brokerHealth.readyConnections; }
      }
      const auto timeout=request.operation==Operation::Capture && firstCapture_ ? captureStartupTimeoutMs : responseTimeoutMs;
      if(request.operation==Operation::Capture) firstCapture_=false;
      if(!timedTransfer(pipe_.value,&request,sizeof(request),true,250)) return failed(Stage::Request,GetLastError());
      if(!timedTransfer(pipe_.value,&response,sizeof(response),false,timeout)) return failed(Stage::Response,GetLastError());
      if(response.signature!=magic) return failed(Stage::Response,ERROR_INVALID_DATA);
      if(response.status==Status::Frame) {
        if(!frame || !validFrame(response)) return failed(Stage::Frame,ERROR_INVALID_DATA);
        frame->pixels.resize(response.bytes);
        if(!timedTransfer(pipe_.value,frame->pixels.data(),response.bytes,false,1000)) return failed(Stage::Frame,GetLastError());
        frame->width=response.width; frame->height=response.height; frame->stride=response.stride;
        frame->texture.reset();
      } else if(response.bytes) return failed(Stage::Response,ERROR_INVALID_DATA);
      if(response.status==Status::ConsoleUnavailable || response.status==Status::DesktopUnavailable
          || response.status==Status::InputUnavailable || response.status==Status::CaptureUnavailable
          || response.status==Status::Unavailable) {
        // Retain the explicit status for the caller; close this worker before
        // a later retry rather than silently classifying it as healthy idle.
        failed(Stage::Rejected,static_cast<DWORD>(response.left),response.status);
      }
      return true;
    } catch(...) { return failed(Stage::Response,ERROR_GEN_FAILURE); }
  }
private:
  bool failed(Stage stage,DWORD error,Status status=Status::Unavailable,std::uint32_t check=0) {
    const auto now=GetTickCount64(); bool log=false;
    {
      std::lock_guard lock(healthMutex);
      ++brokerHealth.failures; brokerHealth.lastStage=static_cast<std::uint32_t>(stage);
      brokerHealth.lastError=error; brokerHealth.lastStatus=static_cast<std::uint32_t>(status);
      brokerHealth.lastCheck=check;
      if(!lastFailureLog || now-lastFailureLog>=1000) { lastFailureLog=now; log=true; }
    }
    if(log) std::cerr<<"SNUAC_HEALTH uacStage="<<static_cast<std::uint32_t>(stage)
      <<" uacError="<<error<<" uacStatus="<<static_cast<std::uint32_t>(status)<<" uacCheck="<<check<<'\n';
    pipe_=Handle(); firstCapture_=true; retryAt_=now+1000; return false;
  }
  Handle pipe_;
  ULONGLONG retryAt_=0;
  bool firstCapture_=true;
};
}
ServerIdentity verifyServiceEndpoint(HANDLE pipe) {
  ULONG pid=0;
  if(!GetNamedPipeServerProcessId(pipe,&pid)) return {false,GetLastError(),1};
  // SCM is authoritative for the running own-process service. A standard
  // user's token may not open/query a SYSTEM process, even when that service
  // is correctly installed and its pipe is accessible. Do not require that
  // unrelated process-object permission or grant it to users.
  struct ServiceGuard { SC_HANDLE value; ~ServiceGuard() { if(value) CloseServiceHandle(value); } };
  ServiceGuard manager{OpenSCManagerW(nullptr,nullptr,SC_MANAGER_CONNECT)};
  if(!manager.value) return {false,GetLastError(),2};
  ServiceGuard service{OpenServiceW(manager.value,serviceName,SERVICE_QUERY_STATUS|SERVICE_QUERY_CONFIG)};
  if(!service.value) return {false,GetLastError(),3};
  SERVICE_STATUS_PROCESS status{}; DWORD bytes=0;
  if(!QueryServiceStatusEx(service.value,SC_STATUS_PROCESS_INFO,reinterpret_cast<BYTE*>(&status),sizeof(status),&bytes))
    return {false,GetLastError(),4};
  if(status.dwCurrentState!=SERVICE_RUNNING || !pid || status.dwProcessId!=pid)
    return {false,ERROR_INVALID_OWNER,4};
  if(status.dwServiceType!=SERVICE_WIN32_OWN_PROCESS) return {false,ERROR_INVALID_SERVICE_ACCOUNT,5};
  DWORD needed=0;
  QueryServiceConfigW(service.value,nullptr,0,&needed);
  if(GetLastError()!=ERROR_INSUFFICIENT_BUFFER || needed<sizeof(QUERY_SERVICE_CONFIGW) || needed>65536)
    return {false,ERROR_INVALID_DATA,5};
  std::vector<BYTE> storage(needed);
  auto* config=reinterpret_cast<QUERY_SERVICE_CONFIGW*>(storage.data());
  if(!QueryServiceConfigW(service.value,config,needed,&needed)) return {false,GetLastError(),5};
  if(config->dwServiceType!=SERVICE_WIN32_OWN_PROCESS || !config->lpServiceStartName
      || _wcsicmp(config->lpServiceStartName,L"LocalSystem")) return {false,ERROR_INVALID_SERVICE_ACCOUNT,6};
  const auto expected=L"\""+(installDirectory()/L"sanser-uac-service.exe").wstring()+L"\" --service";
  if(!config->lpBinaryPathName || _wcsicmp(config->lpBinaryPathName,expected.c_str()))
    return {false,ERROR_BAD_PATHNAME,7};
  return {true,ERROR_SUCCESS,0};
}
struct CaptureClient::Impl { Connection connection; };
CaptureClient::CaptureClient():impl_(std::make_unique<Impl>()) {}
CaptureClient::~CaptureClient()=default;
Health health() { std::lock_guard lock(healthMutex); return brokerHealth; }
bool CaptureClient::capture(FrameBgra& frame,unsigned adapter,unsigned output,bool& changed) {
  Request request; request.adapter=adapter; request.output=output; Response response;
  changed=false;
  if(!impl_->connection.exchange(request,response,&frame)
      || (response.status!=Status::Frame && response.status!=Status::Idle)) return false;
  servedUntil.store(GetTickCount64()+1500);
  { std::lock_guard lock(healthMutex); if(response.status==Status::Frame) ++brokerHealth.captureFrames; else ++brokerHealth.captureIdle; }
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
