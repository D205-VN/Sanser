// CI-only client invoked from the protected, authorized host executable.
#include "client.h"
#include "input_test.h"
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
    <<" targetForeground="<<(foreground==target)<<" pointerInside="<<PtInRect(&bounds,cursor)
    <<" cursor="<<cursor.x<<','<<cursor.y<<" target="<<bounds.left+100<<','<<bounds.top+100<<'\n';
  return sent==3?0:1;
}
