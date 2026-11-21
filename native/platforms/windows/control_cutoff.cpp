#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include "control_cutoff.h"
#include <iostream>

namespace sanser::desktop {
namespace {
constexpr int hotkeyId=0x534B; // 'SK'
constexpr UINT_PTR hideTimer=1;
// A short on-screen notice at the host PC, so the person who pressed the
// hotkey sees whether control is now off or on.
LRESULT CALLBACK noticeProc(HWND window,UINT message,WPARAM w,LPARAM l) {
  if(message==WM_TIMER && w==hideTimer) { KillTimer(window,hideTimer); ShowWindow(window,SW_HIDE); return 0; }
  if(message==WM_NCHITTEST) return HTTRANSPARENT;
  if(message==WM_PAINT) {
    PAINTSTRUCT paint{}; HDC dc=BeginPaint(window,&paint);
    RECT area{}; GetClientRect(window,&area);
    HBRUSH background=CreateSolidBrush(RGB(20,20,20)); FillRect(dc,&area,background); DeleteObject(background);
    wchar_t text[160]{}; GetWindowTextW(window,text,160);
    HFONT font=CreateFontW(-20,0,0,0,FW_SEMIBOLD,0,0,0,DEFAULT_CHARSET,0,0,CLEARTYPE_QUALITY,0,L"Segoe UI");
    HGDIOBJ old=SelectObject(dc,font); SetBkMode(dc,TRANSPARENT); SetTextColor(dc,RGB(255,255,255));
    DrawTextW(dc,text,-1,&area,DT_CENTER|DT_VCENTER|DT_SINGLELINE);
    SelectObject(dc,old); DeleteObject(font); EndPaint(window,&paint); return 0;
  }
  return DefWindowProcW(window,message,w,l);
}
HWND noticeWindow() {
  static const wchar_t* className=L"SanserControlNotice";
  WNDCLASSW klass{}; klass.lpfnWndProc=noticeProc; klass.hInstance=GetModuleHandleW(nullptr); klass.lpszClassName=className;
  klass.hCursor=LoadCursor(nullptr,IDC_ARROW); RegisterClassW(&klass);
  HWND window=CreateWindowExW(WS_EX_TOPMOST|WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE|WS_EX_LAYERED|WS_EX_TRANSPARENT,className,L"",WS_POPUP,
    0,0,520,60,nullptr,nullptr,klass.hInstance,nullptr);
  if(window) SetLayeredWindowAttributes(window,0,235,LWA_ALPHA);
  return window;
}
void showNotice(HWND window,bool off) {
  if(!window) return;
  const char* utf8=cutoff::notice(off); wchar_t text[160]{};
  MultiByteToWideChar(CP_UTF8,0,utf8,-1,text,160); SetWindowTextW(window,text);
  RECT work{}; SystemParametersInfoW(SPI_GETWORKAREA,0,&work,0);
  SetWindowPos(window,HWND_TOPMOST,(work.left+work.right)/2-260,work.top+40,520,60,SWP_NOACTIVATE|SWP_SHOWWINDOW);
  InvalidateRect(window,nullptr,TRUE);
  SetTimer(window,hideTimer,2500,nullptr);
}
}
WindowsControlCutoff::WindowsControlCutoff(bool input) {
  cutoff::reset();
  if(input) worker_=std::thread([this] { run(); });
}
WindowsControlCutoff::~WindowsControlCutoff() {
  stop_=true;
  if(const auto id=thread_.load()) PostThreadMessageW(id,WM_QUIT,0,0);
  if(worker_.joinable()) worker_.join();
}
void WindowsControlCutoff::run() {
  MSG message{};
  PeekMessageW(&message,nullptr,WM_USER,WM_USER,PM_NOREMOVE); // create the queue
  thread_=GetCurrentThreadId();
  if(!RegisterHotKey(nullptr,hotkeyId,MOD_CONTROL|MOD_ALT|MOD_NOREPEAT,'K')) {
    std::cerr<<"SNCONTROL hotkey=unavailable platform=windows win32="<<GetLastError()<<std::endl;
    return;
  }
  HWND notice=noticeWindow();
  while(!stop_ && GetMessageW(&message,nullptr,0,0)>0) {
    if(message.message==WM_HOTKEY && message.wParam==hotkeyId) {
      const bool off=cutoff::toggle(); cutoff::announce(off); showNotice(notice,off);
      continue;
    }
    DispatchMessageW(&message);
  }
  if(notice) DestroyWindow(notice);
  UnregisterHotKey(nullptr,hotkeyId);
}
}
