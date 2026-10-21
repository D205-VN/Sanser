// CI-only client invoked from the protected, authorized host executable.
#include "client.h"
#include "input_test.h"
#include <iostream>
#include <string>

int testUacInput(int argc,char** argv) {
  if(argc!=3) return 2;
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
  const auto sent=sanser::uac::sendInput(3,events,sizeof(INPUT));
  std::cout<<"UAC fixture input events accepted="<<sent<<'\n';
  return sent==3?0:1;
}
