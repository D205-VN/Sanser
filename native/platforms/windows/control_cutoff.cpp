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
  while(!stop_ && GetMessageW(&message,nullptr,0,0)>0)
    if(message.message==WM_HOTKEY && message.wParam==hotkeyId && cutoff::revoke()) cutoff::announce();
  UnregisterHotKey(nullptr,hotkeyId);
}
}
