#pragma once
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <commctrl.h>
#include <cstdint>
#include <functional>
#include <optional>
#include <string>

namespace sanser::desktop {
// A local OS EDIT control owns composition/selection; only explicit submission
// reaches the host. Modeless and independent of the media/decode threads.
class WindowsTextInput {
public:
  static constexpr UINT menuCommand=0x1ee0;
  static constexpr int sendId=1,cancelId=2,editId=100;
  struct Callbacks {
    std::function<bool()> available,focused;
    std::function<std::uint32_t()> generation;
    std::function<bool(const std::string&,std::uint32_t)> submit;
    std::function<void()> begin;
  };
  WindowsTextInput(HWND owner,Callbacks callbacks);
  ~WindowsTextInput();
  WindowsTextInput(const WindowsTextInput&)=delete;
  WindowsTextInput& operator=(const WindowsTextInput&)=delete;
  bool open();
  void toggle();
  void cancel(bool restoreFocus=false);
  void refresh();
  void resize();
  void detach();
  bool preTranslate(MSG& message);
  bool isOpen() const { return panel_!=nullptr; }
  bool owns(HWND window) const { return panel_ && (window==panel_ || IsChild(panel_,window)); }
  HWND editor() const { return edit_; }
  HWND panel() const { return panel_; }
private:
  static LRESULT CALLBACK procedure(HWND,UINT,WPARAM,LPARAM);
  static LRESULT CALLBACK editProcedure(HWND,UINT,WPARAM,LPARAM,UINT_PTR,DWORD_PTR);
  LRESULT process(HWND,UINT,WPARAM,LPARAM);
  void submit();
  bool hasComposition() const;
  bool current() const;
  std::optional<std::u16string> draft() const;
  HWND owner_=nullptr,panel_=nullptr,edit_=nullptr,title_=nullptr,status_=nullptr,send_=nullptr,cancel_=nullptr;
  Callbacks callbacks_;
  std::uint32_t epoch_=0;
  UINT_PTR serial_=0;
  bool composing_=false,fenced_=false,sending_=false,closing_=false;
  std::wstring notice_;
};
}
