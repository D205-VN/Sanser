// Isolated Windows CI fixture. This never changes UAC policy or opens a real
// installer: its only elevated target is the Cancel button created below.
#include "win_util.h"
#include "client.h"
#include "protocol.h"
#include <aclapi.h>
#include <sddl.h>
#include <iostream>
#include <cstdint>
#include <algorithm>

using namespace sanser::uac;
namespace {
bool clicked=false;
class DenyServerQuery {
public:
  DenyServerQuery() {
    SC_HANDLE manager=OpenSCManagerW(nullptr,nullptr,SC_MANAGER_CONNECT);
    require(manager!=nullptr,"Fixture SCM query");
    SC_HANDLE service=OpenServiceW(manager,serviceName,SERVICE_QUERY_STATUS);
    SERVICE_STATUS_PROCESS status{}; DWORD bytes=0;
    const bool found=service && QueryServiceStatusEx(service,SC_STATUS_PROCESS_INFO,reinterpret_cast<BYTE*>(&status),sizeof(status),&bytes);
    if(service) CloseServiceHandle(service); CloseServiceHandle(manager);
    require(found && status.dwCurrentState==SERVICE_RUNNING,"Fixture running service PID");
    HANDLE debugRaw=nullptr; require(OpenProcessToken(GetCurrentProcess(),TOKEN_ADJUST_PRIVILEGES|TOKEN_QUERY,&debugRaw)!=FALSE,"Fixture debug token");
    Handle debug(debugRaw); TOKEN_PRIVILEGES privilege{}; privilege.PrivilegeCount=1;
    require(LookupPrivilegeValueW(nullptr,L"SeDebugPrivilege",&privilege.Privileges[0].Luid)!=FALSE,"Fixture debug privilege");
    privilege.Privileges[0].Attributes=SE_PRIVILEGE_ENABLED;
    require(AdjustTokenPrivileges(debug.value,FALSE,&privilege,0,nullptr,nullptr) && GetLastError()!=ERROR_NOT_ALL_ASSIGNED,"Enable fixture process DACL test");
    process_=Handle(OpenProcess(READ_CONTROL|WRITE_DAC,FALSE,status.dwProcessId));
    require(static_cast<bool>(process_),"Fixture SYSTEM process security handle");
    require(GetSecurityInfo(process_.value,SE_KERNEL_OBJECT,DACL_SECURITY_INFORMATION,nullptr,nullptr,&original_,nullptr,&descriptor_)==ERROR_SUCCESS,"Save fixture process DACL");
    HANDLE raw=nullptr; require(OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&raw)!=FALSE,"Fixture user SID");
    Handle token(raw); DWORD size=0; GetTokenInformation(token.value,TokenUser,nullptr,0,&size);
    std::vector<BYTE> user(size); require(size && GetTokenInformation(token.value,TokenUser,user.data(),size,&size),"Read fixture user SID");
    EXPLICIT_ACCESSW deny{}; deny.grfAccessPermissions=PROCESS_QUERY_LIMITED_INFORMATION;
    deny.grfAccessMode=DENY_ACCESS; deny.grfInheritance=NO_INHERITANCE;
    deny.Trustee.TrusteeForm=TRUSTEE_IS_SID; deny.Trustee.TrusteeType=TRUSTEE_IS_USER;
    deny.Trustee.ptstrName=reinterpret_cast<LPWSTR>(reinterpret_cast<TOKEN_USER*>(user.data())->User.Sid);
    PACL changed=nullptr;
    require(SetEntriesInAclW(1,&deny,original_,&changed)==ERROR_SUCCESS,"Deny fixture process query only");
    const auto error=SetSecurityInfo(process_.value,SE_KERNEL_OBJECT,DACL_SECURITY_INFORMATION,nullptr,nullptr,changed,nullptr);
    LocalFree(changed); require(error==ERROR_SUCCESS,"Apply fixture process query denial");
  }
  ~DenyServerQuery() {
    if(process_ && descriptor_) SetSecurityInfo(process_.value,SE_KERNEL_OBJECT,DACL_SECURITY_INFORMATION,nullptr,nullptr,original_,nullptr);
    if(descriptor_) LocalFree(descriptor_);
  }
private:
  sanser::uac::Handle process_;
  PSECURITY_DESCRIPTOR descriptor_=nullptr;
  PACL original_=nullptr;
};
LRESULT CALLBACK fixture(HWND window,UINT message,WPARAM w,LPARAM l) {
  if(message==WM_COMMAND && LOWORD(w)==42 && HIWORD(w)==BN_CLICKED) { clicked=true; return 0; }
  return DefWindowProcW(window,message,w,l);
}
}
int wmain(int argc,wchar_t** argv) {
  using namespace sanser::uac;
  try {
    require(argc==2,"Protected host path required");
    SetProcessDPIAware();
    DWORD session=0; ProcessIdToSessionId(GetCurrentProcessId(),&session);
    HDESK desktop=OpenInputDesktop(0,FALSE,DESKTOP_READOBJECTS);
    wchar_t desktopName[256]{}; DWORD desktopBytes=0;
    const bool defaultDesktop=desktop && GetUserObjectInformationW(desktop,UOI_NAME,desktopName,sizeof(desktopName),&desktopBytes)
      && _wcsicmp(desktopName,L"Default")==0;
    if(!session || session!=WTSGetActiveConsoleSessionId() || !defaultDesktop) {
      if(desktop) CloseDesktop(desktop);
      std::cout<<"SKIP: elevated input fixture requires an unlocked interactive Windows CI desktop\n"; return 77;
    }
    CloseDesktop(desktop);
    HANDLE raw=nullptr; require(OpenProcessToken(GetCurrentProcess(),TOKEN_ALL_ACCESS,&raw)!=FALSE,"Fixture token");
    Handle token(raw); TOKEN_ELEVATION elevated{}; DWORD bytes=0;
    require(GetTokenInformation(raw,TokenElevation,&elevated,sizeof(elevated),&bytes) && elevated.TokenIsElevated,"Run fixture elevated");
    // A user-owned pipe must never pass identity verification while the real
    // service is running. No request or input is sent to this fake endpoint.
    {
      const auto name=L"\\\\.\\pipe\\Sanser.CI.FakeServer."+std::to_wstring(GetCurrentProcessId());
      Handle server(CreateNamedPipeW(name.c_str(),PIPE_ACCESS_DUPLEX|FILE_FLAG_FIRST_PIPE_INSTANCE,
        PIPE_TYPE_BYTE|PIPE_WAIT|PIPE_REJECT_REMOTE_CLIENTS,1,4096,4096,0,nullptr));
      require(static_cast<bool>(server),"Create fake endpoint fixture");
      Handle client(CreateFileW(name.c_str(),GENERIC_READ|GENERIC_WRITE,0,nullptr,OPEN_EXISTING,0,nullptr));
      require(static_cast<bool>(client),"Connect fake endpoint fixture");
      const auto fake=verifyServiceEndpoint(client.value);
      require(!fake.trusted && fake.check==4,"SCM must reject a user-owned pipe with the wrong service PID");
      std::cout<<"User-owned fake service endpoint rejected\n";
    }
    WNDCLASSW cls{}; cls.lpfnWndProc=fixture; cls.hInstance=GetModuleHandleW(nullptr); cls.lpszClassName=L"SanserUacCancelFixture";
    require(RegisterClassW(&cls)!=0,"Register fixture");
    HWND window=CreateWindowExW(WS_EX_TOPMOST,cls.lpszClassName,L"Sanser CI elevated Cancel fixture",WS_OVERLAPPEDWINDOW|WS_VISIBLE,
      80,80,360,240,nullptr,nullptr,cls.hInstance,nullptr);
    require(window!=nullptr,"Create fixture");
    const auto button=CreateWindowW(L"BUTTON",L"Cancel",WS_CHILD|WS_VISIBLE|BS_PUSHBUTTON,30,30,220,100,window,
      reinterpret_cast<HMENU>(42),cls.hInstance,nullptr);
    require(button!=nullptr,"Create Cancel button");
    SetForegroundWindow(window); UpdateWindow(window);
    // A visible/unlocked CI desktop can still reject synthetic input. Prove
    // that this process can click its own button before testing the broker.
    RECT target{}; require(GetWindowRect(button,&target)!=FALSE,"Baseline button bounds");
    const POINT point{(target.left+target.right)/2,(target.top+target.bottom)/2};
    if(GetForegroundWindow()!=window || WindowFromPoint(point)!=button) {
      std::cout<<"SKIP: fixture cannot own an unobscured foreground input window\n"; DestroyWindow(window); return 77;
    }
    INPUT baseline[3]{}; baseline[0].type=INPUT_MOUSE;
    baseline[0].mi.dwFlags=MOUSEEVENTF_MOVE|MOUSEEVENTF_ABSOLUTE|MOUSEEVENTF_VIRTUALDESK;
    baseline[0].mi.dx=(point.x-GetSystemMetrics(SM_XVIRTUALSCREEN))*65535/(GetSystemMetrics(SM_CXVIRTUALSCREEN)-1);
    baseline[0].mi.dy=(point.y-GetSystemMetrics(SM_YVIRTUALSCREEN))*65535/(GetSystemMetrics(SM_CYVIRTUALSCREEN)-1);
    baseline[1].type=baseline[2].type=INPUT_MOUSE;
    baseline[1].mi.dwFlags=MOUSEEVENTF_LEFTDOWN; baseline[2].mi.dwFlags=MOUSEEVENTF_LEFTUP;
    const auto baselineSent=SendInput(3,baseline,sizeof(INPUT)); const auto baselineError=GetLastError();
    const auto baselineDeadline=GetTickCount64()+1000;
    while(GetTickCount64()<baselineDeadline && !clicked) {
      MSG message{}; while(PeekMessageW(&message,nullptr,0,0,PM_REMOVE)) { TranslateMessage(&message); DispatchMessageW(&message); }
      Sleep(5);
    }
    if(!clicked) {
      POINT cursor{}; GetCursorPos(&cursor);
      std::cout<<"SKIP: same-integrity synthetic input baseline failed accepted="<<baselineSent<<" error="<<baselineError
        <<" cursor="<<cursor.x<<','<<cursor.y<<" expected="<<point.x<<','<<point.y<<"; cross-integrity input is not testable here\n";
      DestroyWindow(window); return 77;
    }
    clicked=false;
    HANDLE mediumRaw=nullptr;
    BYTE adminBuffer[SECURITY_MAX_SID_SIZE]{}; DWORD adminSize=sizeof(adminBuffer);
    require(CreateWellKnownSid(WinBuiltinAdministratorsSid,nullptr,adminBuffer,&adminSize)!=FALSE,"Administrator SID fixture");
    SID_AND_ATTRIBUTES denyAdmin{adminBuffer,0};
    require(CreateRestrictedToken(token.value,DISABLE_MAX_PRIVILEGE,1,&denyAdmin,0,nullptr,0,nullptr,&mediumRaw)!=FALSE,"Filtered input token");
    Handle medium(mediumRaw); PSID sid=nullptr;
    require(ConvertStringSidToSidW(L"S-1-16-8192",&sid)!=FALSE,"Medium integrity SID");
    TOKEN_MANDATORY_LABEL label{}; label.Label.Attributes=SE_GROUP_INTEGRITY; label.Label.Sid=sid;
    const auto changed=SetTokenInformation(medium.value,TokenIntegrityLevel,&label,sizeof(label)+GetLengthSid(sid));
    LocalFree(sid); require(changed!=FALSE,"Medium integrity token");
    // Deterministically reproduce the permission the old client demanded.
    // This only tightens the isolated CI service process DACL; the saved DACL
    // is restored before fixture exit. No production service DACL is changed.
    DenyServerQuery blockedQuery;
    SECURITY_ATTRIBUTES inherit{sizeof(SECURITY_ATTRIBUTES),nullptr,TRUE};
    HANDLE readRaw=nullptr,writeRaw=nullptr;
    require(CreatePipe(&readRaw,&writeRaw,&inherit,0)!=FALSE,"Fixture diagnostic pipe");
    Handle diagnostics(readRaw),diagnosticWriter(writeRaw);
    require(SetHandleInformation(readRaw,HANDLE_FLAG_INHERIT,0)!=FALSE,"Private diagnostic reader");
    Handle nullInput(CreateFileW(L"NUL",GENERIC_READ,FILE_SHARE_READ|FILE_SHARE_WRITE,&inherit,OPEN_EXISTING,0,nullptr));
    require(static_cast<bool>(nullInput),"Fixture null input");
    std::wstring command=L"\""+std::wstring(argv[1])+L"\" --test-uac-input "+std::to_wstring(reinterpret_cast<std::uintptr_t>(window));
    STARTUPINFOW startup{}; startup.cb=sizeof(startup); wchar_t workerDesktop[]=L"winsta0\\default"; startup.lpDesktop=workerDesktop;
    startup.dwFlags=STARTF_USESTDHANDLES; startup.hStdOutput=writeRaw; startup.hStdError=writeRaw; startup.hStdInput=nullInput.value;
    PROCESS_INFORMATION info{};
    require(CreateProcessAsUserW(medium.value,argv[1],command.data(),nullptr,nullptr,TRUE,CREATE_NO_WINDOW,
      nullptr,nullptr,&startup,&info)!=FALSE,"Launch medium-integrity protected input client");
    diagnosticWriter=Handle(); nullInput=Handle();
    Handle process(info.hProcess),thread(info.hThread);
    const auto deadline=GetTickCount64()+10000;
    while(GetTickCount64()<deadline && !clicked) {
      MSG message{}; while(PeekMessageW(&message,nullptr,0,0,PM_REMOVE)) { TranslateMessage(&message); DispatchMessageW(&message); }
      Sleep(5);
    }
    require(WaitForSingleObject(process.value,2000)==WAIT_OBJECT_0,"Input client exit");
    DWORD code=1; GetExitCodeProcess(process.value,&code);
    char details[2048]{}; DWORD available=0,received=0;
    if(PeekNamedPipe(diagnostics.value,nullptr,0,nullptr,&available,nullptr) && available &&
       ReadFile(diagnostics.value,details,std::min<DWORD>(available,sizeof(details)-1),&received,nullptr))
      std::cout.write(details,received);
    std::cout<<"Fixture client exit="<<code<<" clicked="<<clicked
      <<" foreground="<<(GetForegroundWindow()==window)<<'\n';
    POINT cursor{}; GetCursorPos(&cursor);
    std::cout<<"Fixture final pointer="<<cursor.x<<','<<cursor.y<<'\n';
    DestroyWindow(window);
    require(code==0 && clicked,"Medium-integrity host did not click elevated Cancel through authorized broker");
    std::cout<<"Actual elevated Cancel button received click from medium-integrity protected host\n"; return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
