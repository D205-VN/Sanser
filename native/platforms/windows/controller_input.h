#pragma once
#include <windows.h>
#include <string>
#include "../../desktop/controller_session.h"
namespace sanser::desktop {
class WindowsControllerInput {
public:
  static constexpr UINT menuCommand=0x1ec0;
  static constexpr UINT_PTR timerId=0x5347;
  WindowsControllerInput(HWND owner,ControllerSession& session,std::function<bool()> focused,
    std::function<ControllerSample()> source={});
  ~WindowsControllerInput(){detach();}
  void refresh();
  void toggle();
  void stop();
  void detach();
private:
  HWND owner_;ControllerSession* session_;
  std::function<bool()> focused_;std::function<ControllerSample()> source_;
  std::chrono::steady_clock::time_point polled_{};bool timer_=false;
  std::wstring caption_;bool enabled_=false;
};
}
