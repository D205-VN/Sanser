// LocalSystem capture/input broker. No sockets, shell commands, session tokens,
// encoder or account credentials are accepted by this service.
#include "protocol.h"
#include "input_state.h"
#include "win_util.h"
#include "desktop_duplication.h"
#include "sanser_version.h"
#include <wtsapi32.h>
#include <sddl.h>
#include <shellapi.h>
#include <algorithm>
#include <atomic>
#include <cstdio>
#include <iostream>
#include <fstream>
#include <array>
#include <list>
#include <memory>
#include <thread>
#include <vector>

using namespace sanser::uac;
namespace {
std::atomic<bool> stopping{false};
std::atomic<unsigned> clients{0};
SERVICE_STATUS_HANDLE serviceHandle=nullptr;
SERVICE_STATUS serviceStatus{};
struct LocalMemory { void* value=nullptr; ~LocalMemory() { if(value) LocalFree(value); } };
struct ServiceHandle {
  SC_HANDLE value=nullptr;
  explicit ServiceHandle(SC_HANDLE h):value(h) {}
  ~ServiceHandle() { if(value) CloseServiceHandle(value); }
};
std::wstring tokenSid(HANDLE token) {
  DWORD size=0; GetTokenInformation(token,TokenUser,nullptr,0,&size);
  std::vector<unsigned char> bytes(size);
  require(size && GetTokenInformation(token,TokenUser,bytes.data(),size,&size),"Read setup account");
  LPWSTR value=nullptr;
  require(ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(bytes.data())->User.Sid,&value)!=FALSE,"Setup account SID");
  const std::wstring result=value; LocalFree(value); return result;
}
std::wstring currentSid() {
  HANDLE raw=nullptr; require(OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&raw)!=FALSE,"Current account");
  Handle token(raw); return tokenSid(raw);
}
std::wstring allowedSid() {
  const auto path=installDirectory()/L"allowed-user.sid";
  Handle file(CreateFileW(path.c_str(),GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr));
  if(!file) return {};
  BY_HANDLE_FILE_INFORMATION info{};
  if(!GetFileInformationByHandle(file.value,&info) || (info.dwFileAttributes&FILE_ATTRIBUTE_REPARSE_POINT)
      || info.nFileSizeHigh || !info.nFileSizeLow || info.nFileSizeLow>256) return {};
  std::string text(info.nFileSizeLow,'\0');
  if(!transfer(file.value,text.data(),info.nFileSizeLow,false)) return {};
  std::wstring sid(text.begin(),text.end()); PSID parsed=nullptr;
  if(!ConvertStringSidToSidW(sid.c_str(),&parsed)) return {};
  LocalFree(parsed); return sid;
}
void privilege(const wchar_t* name) {
  HANDLE raw=nullptr; require(OpenProcessToken(GetCurrentProcess(),TOKEN_ADJUST_PRIVILEGES|TOKEN_QUERY,&raw)!=FALSE,"Service token");
  Handle token(raw); TOKEN_PRIVILEGES p{}; p.PrivilegeCount=1;
  require(LookupPrivilegeValueW(nullptr,name,&p.Privileges[0].Luid)!=FALSE,"Service privilege");
  p.Privileges[0].Attributes=SE_PRIVILEGE_ENABLED;
  require(AdjustTokenPrivileges(token.value,FALSE,&p,0,nullptr,nullptr)!=FALSE && GetLastError()!=ERROR_NOT_ALL_ASSIGNED,"Enable service privilege");
}
bool unlockedConsole(DWORD session) {
  if(!session || session!=WTSGetActiveConsoleSessionId()) return false;
  LPWSTR raw=nullptr; DWORD bytes=0;
  if(!WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE,session,WTSSessionInfoEx,&raw,&bytes)) return false;
  const auto* info=reinterpret_cast<WTSINFOEXW*>(raw);
  const bool ok=bytes>=sizeof(WTSINFOEXW) && info->Level==1
    && info->Data.WTSInfoExLevel1.SessionState==WTSActive
    && info->Data.WTSInfoExLevel1.SessionFlags==WTS_SESSIONSTATE_UNLOCK;
  WTSFreeMemory(raw); return ok;
}
class InteractiveStation {
public:
  InteractiveStation() {
    station_=OpenWindowStationW(L"WinSta0",FALSE,
      WINSTA_READATTRIBUTES|WINSTA_WRITEATTRIBUTES|WINSTA_ENUMDESKTOPS|WINSTA_READSCREEN);
    require(station_!=nullptr,"Open interactive worker window station");
    if(!SetProcessWindowStation(station_)) {
      const auto error=GetLastError(); CloseWindowStation(station_); station_=nullptr; SetLastError(error);
      require(false,"Bind interactive worker window station");
    }
  }
  ~InteractiveStation() { if(station_) { SetProcessWindowStation(original_); CloseWindowStation(station_); } }
private:
  HWINSTA original_=GetProcessWindowStation(),station_=nullptr;
};
// Each worker owns its desktop handle on its own GUI-free thread/process.
class Desktop {
public:
  ~Desktop() { if(current_) { SetThreadDesktop(original_); CloseDesktop(current_); } }
  bool bind(bool input=false) {
    HDESK next=OpenInputDesktop(0,FALSE,DESKTOP_READOBJECTS|DESKTOP_WRITEOBJECTS|DESKTOP_SWITCHDESKTOP);
    if(!next) return false;
    wchar_t name[256]{}; DWORD needed=0;
    if(!GetUserObjectInformationW(next,UOI_NAME,name,sizeof(name),&needed)
        || (_wcsicmp(name,L"Winlogon") && (!input || _wcsicmp(name,L"Default")))) { CloseDesktop(next); return false; }
    if(current_ && name_==name) { CloseDesktop(next); return true; }
    if(!SetThreadDesktop(next)) { CloseDesktop(next); return false; }
    if(current_) CloseDesktop(current_);
    current_=next; name_=name; return true;
  }
private:
  HDESK original_=GetThreadDesktop(GetCurrentThreadId()),current_=nullptr;
  std::wstring name_;
};
INPUT winInput(const Request& request) {
  INPUT in{}; in.type=request.type;
  if(in.type==INPUT_KEYBOARD) {
    in.ki.dwFlags=request.flags;
    if(request.flags&KEYEVENTF_SCANCODE) in.ki.wScan=static_cast<WORD>(request.data); else in.ki.wVk=static_cast<WORD>(request.data);
  } else { in.mi.dwFlags=request.flags; in.mi.dx=request.x; in.mi.dy=request.y; in.mi.mouseData=request.data; }
  return in;
}
struct AppliedInput {
  DWORD session; Desktop& desktop;
  InputState state;
  ~AppliedInput() {
    if(!unlockedConsole(session) || !desktop.bind(true)) return;
    state.release([](const Request& event) { INPUT input=winInput(event); SendInput(1,&input,sizeof(input)); });
  }
};
int worker(const wchar_t* channelName) {
  HANDLE raw=nullptr; require(OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&raw)!=FALSE,"Worker identity");
  Handle token(raw); require(localSystem(raw),"Worker requires LocalSystem");
  DWORD session=0; require(ProcessIdToSessionId(GetCurrentProcessId(),&session)!=FALSE,"Worker session");
  require(unlockedConsole(session),"Worker requires an unlocked console");
  InteractiveStation station;
  Desktop desktop; std::unique_ptr<DesktopDuplicator> capture; unsigned adapter=0,output=0;
  AppliedInput applied{session,desktop,{}};
  Handle channel(CreateFileW(channelName,GENERIC_READ|GENERIC_WRITE,0,nullptr,OPEN_EXISTING,0,nullptr));
  require(static_cast<bool>(channel),"Private worker channel");
  const HANDLE input=channel.value,out=channel.value;
  Request request;
  while(transfer(input,&request,sizeof(request),false)) {
    Response response; FrameBgra frame;
    // Lock/logoff/user switch revokes access. We only support UAC in an
    // already unlocked interactive console, not Windows sign-in credentials.
    if(!valid(request)) return 2;
    const bool console=unlockedConsole(session);
    const bool bound=console && desktop.bind(request.operation==Operation::Input);
    const auto desktopError=bound ? ERROR_SUCCESS : GetLastError();
    if(bound) {
      try {
        if(request.operation==Operation::Capture) {
          if(!capture || adapter!=request.adapter || output!=request.output) {
            capture=std::make_unique<DesktopDuplicator>(); adapter=request.adapter; output=request.output;
            capture->initialize(adapter,output); // Worker build excludes broker fallback.
          }
          if(capture->captureFrame(frame,50)) {
            response.status=Status::Frame; response.width=frame.width; response.height=frame.height;
            response.stride=frame.stride; response.bytes=static_cast<std::uint32_t>(frame.pixels.size());
            response.left=capture->left(); response.top=capture->top();
            if(!validFrame(response)) return 2;
          } else if(!capture->recovering()) response.status=Status::Idle;
        } else {
          INPUT in=winInput(request);
          if(SendInput(1,&in,sizeof(in))==1) { applied.state.applied(request); response.status=Status::Applied; }
          else { response.status=Status::InputUnavailable; response.left=static_cast<LONG>(GetLastError()); }
        }
      } catch(...) { capture.reset(); response=Response{}; }
    } else {
      capture.reset();
      response.status=console ? Status::DesktopUnavailable : Status::ConsoleUnavailable;
      response.left=static_cast<LONG>(desktopError);
    }
    if(!transfer(out,&response,sizeof(response),true)) break;
    if(response.bytes && !transfer(out,frame.pixels.data(),response.bytes,true)) break;
  }
  return 0;
}
bool authorized(HANDLE pipe,DWORD& session,Handle& owner) {
  ULONG pid=0; if(!GetNamedPipeClientProcessId(pipe,&pid)) return false;
  owner=Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION|SYNCHRONIZE,FALSE,pid)); if(!owner) return false;
  if(!ProcessIdToSessionId(pid,&session) || !unlockedConsole(session)) return false;
  wchar_t path[32768]; DWORD length=32768;
  if(!QueryFullProcessImageNameW(owner.value,0,path,&length)) return false;
  // Never trust a caller-supplied executable or a per-user writable copy.
  const auto expected=installDirectory()/L"sanser-host-windows.exe";
  if(_wcsicmp(std::wstring(path,length).c_str(),expected.c_str())) return false;
  HANDLE rawClient=nullptr,rawUser=nullptr;
  if(!OpenProcessToken(owner.value,TOKEN_QUERY,&rawClient)) return false;
  Handle client(rawClient);
  if(!WTSQueryUserToken(session,&rawUser)) return false;
  Handle user(rawUser);
  auto sid=[](HANDLE token) {
    DWORD n=0; GetTokenInformation(token,TokenUser,nullptr,0,&n); std::vector<unsigned char> data(n);
    if(!n || !GetTokenInformation(token,TokenUser,data.data(),n,&n)) data.clear(); return data;
  };
  const auto a=sid(client.value),b=sid(user.value);
  return !a.empty() && !b.empty() && tokenSid(client.value)==allowedSid()
    && EqualSid(reinterpret_cast<const TOKEN_USER*>(a.data())->User.Sid,reinterpret_cast<const TOKEN_USER*>(b.data())->User.Sid);
}
struct Worker {
  Handle job,process,channel;
  ~Worker() {
    // Give the worker a bounded chance to release held input before the job
    // kills it. Closing the pipe unblocks its synchronous request read.
    channel=Handle();
    if(process) WaitForSingleObject(process.value,250);
  }
  explicit Worker(DWORD session) {
    // Windows forbids handle inheritance across sessions. Connect the Session 0
    // broker to its interactive worker over a random, SYSTEM-only named pipe.
    GUID id{}; require(SUCCEEDED(CoCreateGuid(&id)),"Worker channel ID");
    wchar_t nonce[40]{}; StringFromGUID2(id,nonce,40);
    const std::wstring channelName=L"\\\\.\\pipe\\Sanser.Uac.Worker."+std::wstring(nonce);
    LocalMemory descriptor;
    require(ConvertStringSecurityDescriptorToSecurityDescriptorW(L"D:P(A;;GA;;;SY)",SDDL_REVISION_1,reinterpret_cast<PSECURITY_DESCRIPTOR*>(&descriptor.value),nullptr)!=FALSE,"Private worker ACL");
    SECURITY_ATTRIBUTES security{sizeof(SECURITY_ATTRIBUTES),descriptor.value,FALSE};
    channel=Handle(CreateNamedPipeW(channelName.c_str(),PIPE_ACCESS_DUPLEX|FILE_FLAG_OVERLAPPED|FILE_FLAG_FIRST_PIPE_INSTANCE,
      PIPE_TYPE_BYTE|PIPE_WAIT|PIPE_REJECT_REMOTE_CLIENTS,1,65536,4096,0,&security));
    require(static_cast<bool>(channel),"Create private worker channel");
    HANDLE original=nullptr,copy=nullptr;
    require(OpenProcessToken(GetCurrentProcess(),TOKEN_DUPLICATE|TOKEN_QUERY,&original)!=FALSE,"System token"); Handle token(original);
    require(DuplicateTokenEx(original,TOKEN_ALL_ACCESS,nullptr,SecurityImpersonation,TokenPrimary,&copy)!=FALSE,"Worker token"); Handle workerToken(copy);
    require(SetTokenInformation(copy,TokenSessionId,&session,sizeof(session))!=FALSE,"Worker interactive session");
    STARTUPINFOW start{}; start.cb=sizeof(start);
    wchar_t desktop[]=L"winsta0\\default"; start.lpDesktop=desktop;
    const auto exe=installDirectory()/L"sanser-uac-service.exe";
    std::wstring command=L"\""+exe.wstring()+L"\" --worker \""+channelName+L"\"";
    PROCESS_INFORMATION info{};
    job=Handle(CreateJobObjectW(nullptr,nullptr)); require(static_cast<bool>(job),"Worker job");
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{}; limits.BasicLimitInformation.LimitFlags=JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    require(SetInformationJobObject(job.value,JobObjectExtendedLimitInformation,&limits,sizeof(limits))!=FALSE,"Worker lifetime");
    const auto directory=installDirectory();
    require(CreateProcessAsUserW(copy,exe.c_str(),command.data(),nullptr,nullptr,FALSE,
      CREATE_NO_WINDOW|CREATE_SUSPENDED,nullptr,directory.c_str(),&start,&info)!=FALSE,"Start UAC worker");
    process=Handle(info.hProcess); Handle thread(info.hThread);
    if(!AssignProcessToJobObject(job.value,process.value)) { TerminateProcess(process.value,1); throw std::runtime_error("Unable to bind UAC worker lifetime"); }
    require(ResumeThread(thread.value)!=static_cast<DWORD>(-1),"Resume UAC worker");
    Handle event(CreateEventW(nullptr,TRUE,FALSE,nullptr)); require(static_cast<bool>(event),"Worker accept event");
    OVERLAPPED io{}; io.hEvent=event.value;
    if(!ConnectNamedPipe(channel.value,&io)) {
      const auto error=GetLastError();
      if(error==ERROR_IO_PENDING) {
        if(WaitForSingleObject(event.value,3000)!=WAIT_OBJECT_0) { CancelIoEx(channel.value,&io); WaitForSingleObject(event.value,INFINITE); throw std::runtime_error("UAC worker startup timed out"); }
        DWORD n=0; require(GetOverlappedResult(channel.value,&io,&n,FALSE)!=FALSE,"UAC worker connection");
      } else require(error==ERROR_PIPE_CONNECTED,"UAC worker connection");
    }
  }
};
bool nextRequest(HANDLE pipe,HANDLE owner,Request& request) {
  Handle event(CreateEventW(nullptr,TRUE,FALSE,nullptr)); if(!event) return false;
  OVERLAPPED io{}; io.hEvent=event.value; DWORD n=0;
  if(ReadFile(pipe,&request,sizeof(request),&n,&io)) return n==sizeof(request);
  if(GetLastError()!=ERROR_IO_PENDING) return false;
  while(!stopping.load() && WaitForSingleObject(owner,0)==WAIT_TIMEOUT) {
    if(WaitForSingleObject(event.value,100)==WAIT_OBJECT_0) return GetOverlappedResult(pipe,&io,&n,FALSE) && n==sizeof(request);
  }
  CancelIoEx(pipe,&io); WaitForSingleObject(event.value,INFINITE); return false;
}
void serve(Handle pipe) {
  struct Count { ~Count() { --clients; } } count;
  try {
    DWORD session=0; Handle owner;
    if(!authorized(pipe.value,session,owner)) return;
    Worker workerProcess(session); Request request;
    while(nextRequest(pipe.value,owner.value,request)) {
      if(!valid(request) || !unlockedConsole(session)) break;
      Response response; std::vector<unsigned char> pixels;
      bool ok=timedTransfer(workerProcess.channel.value,&request,sizeof(request),true,250)
        && timedTransfer(workerProcess.channel.value,&response,sizeof(response),false,1000) && response.signature==magic;
      if(ok && response.bytes) {
        ok=validFrame(response);
        if(ok) { pixels.resize(response.bytes); ok=timedTransfer(workerProcess.channel.value,pixels.data(),response.bytes,false,1000); }
      }
      if(!ok) break;
      if(!unlockedConsole(session)) break; // A lock may arrive during GPU readback.
      if(!timedTransfer(pipe.value,&response,sizeof(response),true,1000)) break;
      if(response.bytes && !timedTransfer(pipe.value,pixels.data(),response.bytes,true,1000)) break;
    }
  } catch(...) { /* Fail closed. The caller retains its last frame and session. */ }
}
void WINAPI control(DWORD code) { if(code==SERVICE_CONTROL_STOP || code==SERVICE_CONTROL_SHUTDOWN) stopping=true; }
void WINAPI serviceMain(DWORD,wchar_t**) {
  serviceHandle=RegisterServiceCtrlHandlerW(serviceName,control); if(!serviceHandle) return;
  serviceStatus.dwServiceType=SERVICE_WIN32_OWN_PROCESS; serviceStatus.dwCurrentState=SERVICE_START_PENDING;
  SetServiceStatus(serviceHandle,&serviceStatus);
  try {
    privilege(SE_TCB_NAME); privilege(SE_ASSIGNPRIMARYTOKEN_NAME); privilege(SE_INCREASE_QUOTA_NAME);
    // No client can replace this first pipe instance before the service starts.
    LocalMemory descriptor;
    const auto allowed=allowedSid(); require(!allowed.empty(),"UAC setup account is missing");
    const std::wstring pipeAcl=L"D:P(A;;GA;;;SY)(A;;GRGW;;;"+allowed+L")";
    require(ConvertStringSecurityDescriptorToSecurityDescriptorW(pipeAcl.c_str(),SDDL_REVISION_1,reinterpret_cast<PSECURITY_DESCRIPTOR*>(&descriptor.value),nullptr)!=FALSE,"Pipe ACL");
    SECURITY_ATTRIBUTES security{sizeof(SECURITY_ATTRIBUTES),descriptor.value,FALSE};
    struct Handlers {
      struct Entry { std::thread thread; std::shared_ptr<std::atomic<bool>> done; };
      std::list<Entry> entries;
      ~Handlers() { stopping=true; for(auto& e:entries) if(e.thread.joinable()) e.thread.join(); }
      void reap() { for(auto i=entries.begin();i!=entries.end();) { if(i->done->load()) { i->thread.join(); i=entries.erase(i); } else ++i; } }
      void add(Handle pipe) {
        auto done=std::make_shared<std::atomic<bool>>(false);
        entries.push_back({std::thread([p=std::move(pipe),done]() mutable { serve(std::move(p)); *done=true; }),done});
      }
    } handlers;
    bool first=true;
    serviceStatus.dwCurrentState=SERVICE_RUNNING; serviceStatus.dwControlsAccepted=SERVICE_ACCEPT_STOP|SERVICE_ACCEPT_SHUTDOWN; SetServiceStatus(serviceHandle,&serviceStatus);
    while(!stopping.load()) {
      handlers.reap();
      if(clients.load()>=4) { Sleep(100); continue; }
      Handle pipe(CreateNamedPipeW(pipeName,PIPE_ACCESS_DUPLEX|FILE_FLAG_OVERLAPPED|(first?FILE_FLAG_FIRST_PIPE_INSTANCE:0),
        PIPE_TYPE_MESSAGE|PIPE_READMODE_MESSAGE|PIPE_WAIT|PIPE_REJECT_REMOTE_CLIENTS,4,65536,4096,0,&security));
      require(static_cast<bool>(pipe),"Create local UAC pipe"); first=false;
      Handle event(CreateEventW(nullptr,TRUE,FALSE,nullptr)); require(static_cast<bool>(event),"Pipe accept event");
      OVERLAPPED io{}; io.hEvent=event.value;
      bool connected=ConnectNamedPipe(pipe.value,&io)!=FALSE;
      if(!connected) {
        const auto error=GetLastError(); connected=error==ERROR_PIPE_CONNECTED;
        if(error==ERROR_IO_PENDING) {
          while(!stopping.load() && WaitForSingleObject(event.value,100)==WAIT_TIMEOUT) {}
          DWORD n=0; connected=!stopping.load() && GetOverlappedResult(pipe.value,&io,&n,FALSE);
          if(stopping.load()) { CancelIoEx(pipe.value,&io); WaitForSingleObject(event.value,INFINITE); }
        }
      }
      if(connected) { ++clients; handlers.add(std::move(pipe)); }
    }
  } catch(...) { serviceStatus.dwWin32ExitCode=ERROR_SERVICE_SPECIFIC_ERROR; serviceStatus.dwServiceSpecificExitCode=1; }
  serviceStatus.dwCurrentState=SERVICE_STOPPED; serviceStatus.dwControlsAccepted=0; SetServiceStatus(serviceHandle,&serviceStatus);
}
bool running() {
  ServiceHandle manager(OpenSCManagerW(nullptr,nullptr,SC_MANAGER_CONNECT)); if(!manager.value) return false;
  ServiceHandle service(OpenServiceW(manager.value,serviceName,SERVICE_QUERY_STATUS)); if(!service.value) return false;
  SERVICE_STATUS status{}; return QueryServiceStatus(service.value,&status) && status.dwCurrentState==SERVICE_RUNNING;
}
bool registered() {
  ServiceHandle manager(OpenSCManagerW(nullptr,nullptr,SC_MANAGER_CONNECT)); if(!manager.value) return false;
  ServiceHandle service(OpenServiceW(manager.value,serviceName,SERVICE_QUERY_STATUS));
  return service.value!=nullptr;
}
void stop(SC_HANDLE service) {
  SERVICE_STATUS_PROCESS details{}; DWORD bytes=0;
  QueryServiceStatusEx(service,SC_STATUS_PROCESS_INFO,reinterpret_cast<BYTE*>(&details),sizeof(details),&bytes);
  Handle process(details.dwProcessId ? OpenProcess(SYNCHRONIZE,FALSE,details.dwProcessId) : nullptr);
  SERVICE_STATUS status{}; ControlService(service,SERVICE_CONTROL_STOP,&status);
  const auto deadline=GetTickCount64()+10000;
  while(QueryServiceStatus(service,&status) && status.dwCurrentState!=SERVICE_STOPPED && GetTickCount64()<deadline) Sleep(50);
  require(status.dwCurrentState==SERVICE_STOPPED,"Stop existing UAC service");
  if(process) require(WaitForSingleObject(process.value,5000)==WAIT_OBJECT_0,"Wait for UAC service exit");
}
bool sameFileContents(const std::filesystem::path& a,const std::filesystem::path& b) {
  std::error_code error;
  const auto size=std::filesystem::file_size(a,error); if(error) return false;
  if(size!=std::filesystem::file_size(b,error) || error) return false;
  std::ifstream first(a,std::ios::binary),second(b,std::ios::binary);
  if(!first || !second) return false;
  std::array<char,65536> x{},y{};
  while(first) {
    first.read(x.data(),x.size()); second.read(y.data(),y.size());
    if(first.gcount()!=second.gcount() || !std::equal(x.begin(),x.begin()+first.gcount(),y.begin())) return false;
  }
  return first.eof() && second.eof();
}
bool installedCurrent() {
  const auto source=imagePath().parent_path(),target=installDirectory();
  return sameFileContents(source/L"sanser-uac-service.exe",target/L"sanser-uac-service.exe")
    && sameFileContents(source/L"sanser-host-windows.exe",target/L"sanser-host-windows.exe");
}
void install(bool enable,bool preserveBinding=false) {
  // Updates may run elevated as another administrator or SYSTEM. Never rebind
  // an existing installation to the updater's token instead of its owner.
  const auto previous=preserveBinding?allowedSid():std::wstring{};
  const auto account=previous.empty()?currentSid():previous;
  if(enable) require(account!=L"S-1-5-18","Initial UAC setup requires an interactive administrator account");
  const auto disabledFile=installDirectory()/L"disabled";
  if(enable && preserveBinding && GetFileAttributesW(disabledFile.c_str())!=INVALID_FILE_ATTRIBUTES) return;
  ServiceHandle manager(OpenSCManagerW(nullptr,nullptr,SC_MANAGER_CONNECT|SC_MANAGER_CREATE_SERVICE)); require(manager.value!=nullptr,"Administrator approval required");
  ServiceHandle old(OpenServiceW(manager.value,serviceName,SERVICE_STOP|SERVICE_QUERY_STATUS|DELETE));
  if(old.value) stop(old.value);
  if(!enable) {
    if(old.value) require(DeleteService(old.value)!=FALSE,"Remove UAC service");
    // Protected preference survives an updater running under another token or
    // with the account's HKCU hive unloaded. This marker can only reduce access.
    if(!preserveBinding && GetFileAttributesW(installDirectory().c_str())!=INVALID_FILE_ATTRIBUTES) {
      Handle marker(CreateFileW(disabledFile.c_str(),GENERIC_WRITE,0,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr));
      require(marker || GetLastError()==ERROR_FILE_EXISTS,"Preserve disabled UAC preference");
    }
    return;
  }
  privilege(SE_RESTORE_NAME);
  const auto target=installDirectory(); const auto source=imagePath().parent_path();
  LocalMemory descriptor;
  require(ConvertStringSecurityDescriptorToSecurityDescriptorW(L"O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)",SDDL_REVISION_1,reinterpret_cast<PSECURITY_DESCRIPTOR*>(&descriptor.value),nullptr)!=FALSE,"Install ACL");
  SECURITY_ATTRIBUTES security{sizeof(SECURITY_ATTRIBUTES),descriptor.value,FALSE};
  if(!CreateDirectoryW(target.c_str(),&security)) require(GetLastError()==ERROR_ALREADY_EXISTS,"Create protected UAC directory");
  const DWORD attributes=GetFileAttributesW(target.c_str());
  require(attributes!=INVALID_FILE_ATTRIBUTES && !(attributes&FILE_ATTRIBUTE_REPARSE_POINT),"Reject redirected UAC directory");
  require(SetFileSecurityW(target.c_str(),OWNER_SECURITY_INFORMATION|GROUP_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION|PROTECTED_DACL_SECURITY_INFORMATION,descriptor.value)!=FALSE,"Protect UAC directory");
  for(const auto* name:{L"sanser-uac-service.exe",L"sanser-host-windows.exe"}) {
    const auto destination=target/name;
    const DWORD attr=GetFileAttributesW(destination.c_str());
    require(attr==INVALID_FILE_ATTRIBUTES || !(attr&FILE_ATTRIBUTE_REPARSE_POINT),"Reject redirected UAC file");
    if(attr!=INVALID_FILE_ATTRIBUTES) require(DeleteFileW(destination.c_str())!=FALSE,"Replace protected UAC binary");
    require(CopyFileW((source/name).c_str(),destination.c_str(),TRUE)!=FALSE,"Install UAC binary");
    require(SetFileSecurityW(destination.c_str(),OWNER_SECURITY_INFORMATION|GROUP_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION|PROTECTED_DACL_SECURITY_INFORMATION,descriptor.value)!=FALSE,"Protect UAC binary");
  }
  // Bind this privileged feature to the administrator who explicitly enabled
  // it. A different local/standard user must not use it to read UAC credentials.
  const auto sid=account; const std::string encoded(sid.begin(),sid.end());
  const auto allowedFile=target/L"allowed-user.sid";
  const auto existing=GetFileAttributesW(allowedFile.c_str());
  require(existing==INVALID_FILE_ATTRIBUTES || !(existing&FILE_ATTRIBUTE_REPARSE_POINT),"Reject redirected UAC account file");
  if(existing!=INVALID_FILE_ATTRIBUTES) require(DeleteFileW(allowedFile.c_str())!=FALSE,"Replace UAC account binding");
  {
    Handle file(CreateFileW(allowedFile.c_str(),GENERIC_WRITE,0,&security,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr));
    require(static_cast<bool>(file),"Bind UAC setup account");
    require(transfer(file.value,const_cast<char*>(encoded.data()),static_cast<DWORD>(encoded.size()),true),"Write UAC setup account");
  }
  const std::wstring command=L"\""+(target/L"sanser-uac-service.exe").wstring()+L"\" --service";
  ServiceHandle service(old.value?OpenServiceW(manager.value,serviceName,SERVICE_CHANGE_CONFIG|SERVICE_START):
    CreateServiceW(manager.value,serviceName,L"Sanser UAC support",SERVICE_START,SERVICE_WIN32_OWN_PROCESS,SERVICE_AUTO_START,SERVICE_ERROR_NORMAL,command.c_str(),nullptr,nullptr,nullptr,L"LocalSystem",nullptr));
  require(service.value!=nullptr,"Register UAC service");
  if(old.value) require(ChangeServiceConfigW(service.value,SERVICE_NO_CHANGE,SERVICE_AUTO_START,SERVICE_NO_CHANGE,command.c_str(),nullptr,nullptr,nullptr,L"LocalSystem",nullptr,nullptr)!=FALSE,"Update UAC service");
  require(StartServiceW(service.value,0,nullptr)!=FALSE,"Start UAC service");
  const auto deadline=GetTickCount64()+10000; while(!running() && GetTickCount64()<deadline) Sleep(50);
  require(running(),"UAC service startup");
  if(GetFileAttributesW(disabledFile.c_str())!=INVALID_FILE_ATTRIBUTES)
    require(DeleteFileW(disabledFile.c_str())!=FALSE,"Enable UAC preference");
}
int elevate(const wchar_t* mode) {
  const auto exe=imagePath(); SHELLEXECUTEINFOW launch{}; launch.cbSize=sizeof(launch);
  launch.fMask=SEE_MASK_NOCLOSEPROCESS|SEE_MASK_NOASYNC; launch.lpVerb=L"runas"; launch.lpFile=exe.c_str();
  launch.lpParameters=mode; launch.nShow=SW_HIDE;
  require(ShellExecuteExW(&launch)!=FALSE,"UAC approval cancelled or unavailable"); Handle process(launch.hProcess);
  require(WaitForSingleObject(process.value,120000)==WAIT_OBJECT_0,"UAC setup timed out");
  DWORD code=1; GetExitCodeProcess(process.value,&code); return static_cast<int>(code);
}
}
int wmain(int argc,wchar_t** argv) {
  SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32|LOAD_LIBRARY_SEARCH_APPLICATION_DIR);
  // Match the native host before DXGI reads output geometry. Otherwise GetDesc
  // virtualizes high-DPI dimensions and the capture loop rejects every frame.
  SetProcessDPIAware();
  try {
    if(argc==3 && std::wstring(argv[1])==L"--worker") return worker(argv[2]);
    if(argc!=2) return 2;
    const std::wstring mode=argv[1];
    if(mode==L"--install" || mode==L"--uninstall") { install(mode==L"--install"); return 0; }
    if(mode==L"--installer-install") { install(true,true); return 0; }
    if(mode==L"--installer-uninstall") { install(false,true); return 0; }
    if(mode==L"--installer-enable") {
      // The installer owns the elevation prompt; the normal desktop app never
      // silently elevates itself. An explicit in-app disable survives updates.
      DWORD disabled=0,bytes=sizeof(disabled);
      if(RegGetValueW(HKEY_CURRENT_USER,L"Software\\Sanser",L"UacDisabled",RRF_RT_REG_DWORD,nullptr,&disabled,&bytes)==ERROR_SUCCESS && disabled) return 0;
      if(GetFileAttributesW((installDirectory()/L"disabled").c_str())!=INVALID_FILE_ATTRIBUTES) return 0;
      if(running() && installedCurrent()) return 0;
      const auto result=elevate(L"--installer-install");
      if(result==0 && currentSid()!=allowedSid())
        throw std::runtime_error("UAC support remains bound to its Windows account; sign in with that administrator account");
      return result;
    }
    if(mode==L"--installer-remove") {
      // A stopped or failed AUTO_START service is still installed and would
      // otherwise survive app uninstall and restart after the next reboot.
      if(currentSid()!=allowedSid() || !registered()) return 0;
      return elevate(L"--installer-uninstall");
    }
    if(mode==L"--enable" || mode==L"--disable") {
      const auto result=elevate(mode==L"--enable"?L"--install":L"--uninstall");
      if(result==0 && mode==L"--enable" && currentSid()!=allowedSid())
        throw std::runtime_error("Sign in with the Windows administrator account that enabled UAC support");
      if(result==0) {
        const DWORD disabled=mode==L"--disable"?1:0;
        require(RegSetKeyValueW(HKEY_CURRENT_USER,L"Software\\Sanser",L"UacDisabled",REG_DWORD,&disabled,sizeof(disabled))==ERROR_SUCCESS,"Save UAC preference");
      }
      return result;
    }
    if(mode==L"--status") {
      std::cout<<"{\"enabled\":"<<((running() && currentSid()==allowedSid())?"true":"false")<<",\"current\":"<<(installedCurrent()?"true":"false")<<",\"dpiAware\":"<<(IsProcessDPIAware()?"true":"false")<<",\"version\":\""<<sanser::kVersion<<"\"}\n"; return 0;
    }
    if(mode==L"--host-path") {
      const auto path=(installDirectory()/L"sanser-host-windows.exe").wstring();
      const int size=WideCharToMultiByte(CP_UTF8,0,path.c_str(),-1,nullptr,0,nullptr,nullptr);
      std::string utf8(size,'\0'); WideCharToMultiByte(CP_UTF8,0,path.c_str(),-1,utf8.data(),size,nullptr,nullptr);
      utf8.pop_back(); std::cout<<utf8<<'\n'; return 0;
    }
    if(mode!=L"--service") return 2;
    SERVICE_TABLE_ENTRYW table[]={{const_cast<wchar_t*>(serviceName),serviceMain},{nullptr,nullptr}};
    require(StartServiceCtrlDispatcherW(table)!=FALSE,"Service dispatcher"); return 0;
  } catch(const std::exception& error) { std::cerr<<"Sanser UAC: "<<error.what()<<'\n'; return 1; }
}
