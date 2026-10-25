// CI-only client invoked from the protected, authorized host executable.
#include "client.h"
#include "input_test.h"
#include "protocol.h"
#include "win_util.h"
#include <iostream>
#include <string>

int testUacInput(int argc,char** argv) {
  if(argc!=3) return 2;
  SetProcessDPIAware(); // Match the normal host's physical-screen coordinates.
  const auto window=reinterpret_cast<HWND>(std::stoull(argv[2]));
  wchar_t title[128]{}; GetWindowTextW(window,title,128);
  if(std::wstring(title)!=L"Sanser CI elevated Cancel fixture") return 2;
  RECT bounds{}; if(!GetWindowRect(window,&bounds)) return 2;
  const int width=GetSystemMetrics(SM_CXVIRTUALSCREEN),height=GetSystemMetrics(SM_CYVIRTUALSCREEN);
  if(width<=1 || height<=1) return 2;
  HANDLE rawToken=nullptr;
  if(!OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&rawToken)) return 1;
  sanser::uac::Handle token(rawToken); DWORD groupBytes=0;
  GetTokenInformation(token.value,TokenGroups,nullptr,0,&groupBytes);
  std::vector<BYTE> groupData(groupBytes);
  if(!groupBytes || !GetTokenInformation(token.value,TokenGroups,groupData.data(),groupBytes,&groupBytes)) return 1;
  bool adminDenyOnly=false;
  const auto* groups=reinterpret_cast<const TOKEN_GROUPS*>(groupData.data());
  for(DWORD i=0;i<groups->GroupCount;++i) {
    if(IsWellKnownSid(groups->Groups[i].Sid,WinBuiltinAdministratorsSid))
      adminDenyOnly=(groups->Groups[i].Attributes&SE_GROUP_USE_FOR_DENY_ONLY)!=0;
  }
  SC_HANDLE manager=OpenSCManagerW(nullptr,nullptr,SC_MANAGER_CONNECT);
  SC_HANDLE service=manager ? OpenServiceW(manager,sanser::uac::serviceName,SERVICE_QUERY_STATUS) : nullptr;
  SERVICE_STATUS_PROCESS serviceStatus{}; DWORD serviceBytes=0;
  const bool known=service && QueryServiceStatusEx(service,SC_STATUS_PROCESS_INFO,
    reinterpret_cast<BYTE*>(&serviceStatus),sizeof(serviceStatus),&serviceBytes);
  if(service) CloseServiceHandle(service); if(manager) CloseServiceHandle(manager);
  if(!known) return 1;
  sanser::uac::Handle legacyQuery(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,FALSE,serviceStatus.dwProcessId));
  const bool legacyQueryDenied=!legacyQuery && GetLastError()==ERROR_ACCESS_DENIED;
  std::cout<<"Filtered host adminDenyOnly="<<adminDenyOnly<<" legacySystemQueryDenied="<<legacyQueryDenied<<'\n';
  if(!adminDenyOnly || !legacyQueryDenied) return 1;
  INPUT events[3]{};
  events[0].type=INPUT_MOUSE;
  events[0].mi.dwFlags=MOUSEEVENTF_MOVE|MOUSEEVENTF_ABSOLUTE|MOUSEEVENTF_VIRTUALDESK;
  events[0].mi.dx=(bounds.left+100-GetSystemMetrics(SM_XVIRTUALSCREEN))*65535/(width-1);
  events[0].mi.dy=(bounds.top+100-GetSystemMetrics(SM_YVIRTUALSCREEN))*65535/(height-1);
  events[1].type=INPUT_MOUSE; events[1].mi.dwFlags=MOUSEEVENTF_LEFTDOWN;
  events[2].type=INPUT_MOUSE; events[2].mi.dwFlags=MOUSEEVENTF_LEFTUP;
  // Exercise the same fallback as real clicks after UAC closes. Do not force
  // active() or call the broker directly; that would miss the regression.
  sanser::uac::InputDispatch dispatch;
  const auto sent=sanser::uac::sendInput(3,events,sizeof(INPUT),&dispatch);
  const auto error=GetLastError();
  POINT cursor{}; GetCursorPos(&cursor);
  DWORD foreground=0; GetWindowThreadProcessId(GetForegroundWindow(),&foreground);
  DWORD target=0; GetWindowThreadProcessId(window,&target);
  std::cout<<"UAC fixture input events accepted="<<sent<<" error="<<error
    <<" direct="<<dispatch.direct<<" privileged="<<dispatch.privileged
    <<" sourceIL="<<dispatch.sourceIntegrity<<" targetIL="<<dispatch.targetIntegrity
    <<" failureStatus="<<dispatch.failureStatus<<" failureError="<<dispatch.failureError
    <<" targetForeground="<<(foreground==target)<<" pointerInside="<<PtInRect(&bounds,cursor)
    <<" cursor="<<cursor.x<<','<<cursor.y<<" target="<<bounds.left+100<<','<<bounds.top+100<<'\n';
  return sent==3 && dispatch.privileged==3 && dispatch.direct==0?0:1;
}
