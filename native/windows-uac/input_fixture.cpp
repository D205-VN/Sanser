// Isolated Windows CI fixture. This never changes UAC policy or opens a real
// installer: its only elevated target is the Cancel button created below.
#include "win_util.h"
#include <sddl.h>
#include <iostream>
#include <cstdint>
#include <algorithm>

namespace {
bool clicked=false;
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
    WNDCLASSW cls{}; cls.lpfnWndProc=fixture; cls.hInstance=GetModuleHandleW(nullptr); cls.lpszClassName=L"SanserUacCancelFixture";
    require(RegisterClassW(&cls)!=0,"Register fixture");
    HWND window=CreateWindowExW(WS_EX_TOPMOST,cls.lpszClassName,L"Sanser CI elevated Cancel fixture",WS_OVERLAPPEDWINDOW|WS_VISIBLE,
      80,80,360,240,nullptr,nullptr,cls.hInstance,nullptr);
    require(window!=nullptr,"Create fixture");
    require(CreateWindowW(L"BUTTON",L"Cancel",WS_CHILD|WS_VISIBLE|BS_PUSHBUTTON,30,30,220,100,window,
      reinterpret_cast<HMENU>(42),cls.hInstance,nullptr)!=nullptr,"Create Cancel button");
    SetForegroundWindow(window); UpdateWindow(window);
    HANDLE mediumRaw=nullptr;
    require(CreateRestrictedToken(token.value,DISABLE_MAX_PRIVILEGE,0,nullptr,0,nullptr,0,nullptr,&mediumRaw)!=FALSE,"Restricted input token");
    Handle medium(mediumRaw); PSID sid=nullptr;
    require(ConvertStringSidToSidW(L"S-1-16-8192",&sid)!=FALSE,"Medium integrity SID");
    TOKEN_MANDATORY_LABEL label{}; label.Label.Attributes=SE_GROUP_INTEGRITY; label.Label.Sid=sid;
    const auto changed=SetTokenInformation(medium.value,TokenIntegrityLevel,&label,sizeof(label)+GetLengthSid(sid));
    LocalFree(sid); require(changed!=FALSE,"Medium integrity token");
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
    DestroyWindow(window);
    require(code==0 && clicked,"Medium-integrity host did not click elevated Cancel through authorized broker");
    std::cout<<"Actual elevated Cancel button received click from medium-integrity protected host\n"; return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
