#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include "privacy_screen.h"
#include "../../windows-uac/client.h"
#include <iostream>

#ifndef WDA_EXCLUDEFROMCAPTURE
#define WDA_EXCLUDEFROMCAPTURE 0x00000011
#endif

namespace sanser::desktop {
namespace {
constexpr UINT updateMessage=WM_APP+41;
constexpr int hotkeyId=0x5350; // 'SP'
constexpr UINT_PTR topmostTimer=1;
LRESULT CALLBACK overlayProc(HWND window,UINT message,WPARAM wParam,LPARAM lParam) {
  switch(message) {
    case WM_NCHITTEST: return HTTRANSPARENT; // remote and local clicks pass through
    case WM_MOUSEACTIVATE: return MA_NOACTIVATE;
    case WM_ERASEBKGND: { RECT r; GetClientRect(window,&r); FillRect(reinterpret_cast<HDC>(wParam),&r,static_cast<HBRUSH>(GetStockObject(BLACK_BRUSH))); return 1; }
    case WM_CLOSE: return 0; // only the engine removes it
    default: return DefWindowProcW(window,message,wParam,lParam);
  }
}
void coverVirtualScreen(HWND window) {
  SetWindowPos(window,HWND_TOPMOST,GetSystemMetrics(SM_XVIRTUALSCREEN),GetSystemMetrics(SM_YVIRTUALSCREEN),
    GetSystemMetrics(SM_CXVIRTUALSCREEN),GetSystemMetrics(SM_CYVIRTUALSCREEN),SWP_NOACTIVATE);
}
}
WindowsPrivacyScreen::WindowsPrivacyScreen(bool enabled,bool blockInput):enabled_(enabled),blockInput_(enabled && blockInput) {
  if(enabled_) worker_=std::thread([this] { run(); });
}
WindowsPrivacyScreen::~WindowsPrivacyScreen() {
  sanser::uac::setLocalInputBlock(false);
  stop_=true;
  if(const auto id=thread_.load()) PostThreadMessageW(id,WM_QUIT,0,0);
  if(worker_.joinable()) worker_.join();
}
void WindowsPrivacyScreen::update(bool sessionReady) {
  if(!enabled_ || ready_.exchange(sessionReady)==sessionReady) return;
  if(const auto id=thread_.load()) PostThreadMessageW(id,updateMessage,0,0);
}
void WindowsPrivacyScreen::run() {
  MSG message{};
  PeekMessageW(&message,nullptr,WM_USER,WM_USER,PM_NOREMOVE); // create the queue
  thread_=GetCurrentThreadId();
  const auto instance=GetModuleHandleW(nullptr);
  WNDCLASSW klass{}; klass.lpfnWndProc=overlayProc; klass.hInstance=instance; klass.lpszClassName=L"SanserPrivacyScreen";
  klass.hbrBackground=static_cast<HBRUSH>(GetStockObject(BLACK_BRUSH)); klass.hCursor=LoadCursor(nullptr,IDC_ARROW);
  RegisterClassW(&klass);
  HWND window=CreateWindowExW(WS_EX_TOPMOST|WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE|WS_EX_LAYERED|WS_EX_TRANSPARENT,
    klass.lpszClassName,L"Sanser privacy screen",WS_POPUP,0,0,1,1,nullptr,nullptr,instance,nullptr);
  privacy::State state; state.enabled=true; state.blockRequested=blockInput_;
  if(window) {
    SetLayeredWindowAttributes(window,0,255,LWA_ALPHA);
    // Never show an overlay that Desktop Duplication would capture.
    state.captureExcluded=SetWindowDisplayAffinity(window,WDA_EXCLUDEFROMCAPTURE)!=FALSE;
    if(!state.captureExcluded) std::cerr<<"SNPRIVACY captureExclusion=unavailable win32="<<GetLastError()<<" requires=Windows10-2004"<<std::endl;
  }
  if(!RegisterHotKey(nullptr,hotkeyId,MOD_CONTROL|MOD_ALT|MOD_SHIFT|MOD_NOREPEAT,'P'))
    std::cerr<<"SNPRIVACY hotkey=unavailable win32="<<GetLastError()<<std::endl;
  std::string marker; bool visible=false;
  const auto apply=[&] {
    state.sessionReady=ready_.load();
    // The helper reports the real outcome (SNUAC_BLOCK_INPUT) and retries a
    // transient failure; local input stays usable whenever it is not applied.
    state.blockAvailable=blockInput_;
    const bool show=window && privacy::showOverlay(state);
    if(show!=visible) {
      visible=show;
      if(show) { coverVirtualScreen(window); ShowWindow(window,SW_SHOWNOACTIVATE); SetTimer(window,topmostTimer,1000,nullptr); }
      else { KillTimer(window,topmostTimer); ShowWindow(window,SW_HIDE); }
    }
    sanser::uac::setLocalInputBlock(privacy::blockLocalInput(state));
    if(auto line=privacy::marker(state); line!=marker) { marker=line; std::cerr<<line<<std::endl; }
  };
  apply();
  while(!stop_ && GetMessageW(&message,nullptr,0,0)>0) {
    if(message.message==WM_HOTKEY && message.wParam==hotkeyId) { state.dismissed=true; apply(); continue; }
    if(message.message==updateMessage) { apply(); continue; }
    if(message.message==WM_TIMER && message.hwnd==window && message.wParam==topmostTimer) {
      // Re-assert above newly opened topmost windows; follow display changes.
      coverVirtualScreen(window); apply(); continue;
    }
    TranslateMessage(&message); DispatchMessageW(&message);
  }
  sanser::uac::setLocalInputBlock(false);
  UnregisterHotKey(nullptr,hotkeyId);
  if(window) DestroyWindow(window);
  UnregisterClassW(klass.lpszClassName,instance);
}
}
