#pragma once
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include "../../desktop/session_shortcuts.h"

namespace sanser::desktop {
// Modeless child UI: no DialogBox/TrackPopupMenu loop and no network waits.
class WindowsShortcuts {
public:
  static constexpr UINT menuCommand=0x1ed0;
  static constexpr int windowsId=1,macId=2,listId=3,sendId=4,closeId=5;
  WindowsShortcuts(HWND owner,ShortcutSession::Callbacks callbacks);
  ~WindowsShortcuts();
  bool open();
  void toggle();
  void cancel();
  void refresh();
  void resize();
  void detach();
  bool preTranslate(MSG& message);
  bool owns(HWND window) const {return panel_ && (window==panel_ || IsChild(panel_,window));}
  HWND panel() const {return panel_;}
private:
  static LRESULT CALLBACK procedure(HWND,UINT,WPARAM,LPARAM);
  LRESULT process(HWND,UINT,WPARAM,LPARAM);
  void choose(ShortcutHost host);
  void send();
  HWND owner_=nullptr,panel_=nullptr,title_=nullptr,windows_=nullptr,mac_=nullptr,list_=nullptr,send_=nullptr,close_=nullptr,status_=nullptr;
  ShortcutSession session_;
  std::vector<SessionShortcut> actions_;
  int announcedAvailable_=-1;
};
}
