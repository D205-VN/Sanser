#include "shortcuts_panel.h"
#include <iostream>
#include <stdexcept>
using namespace sanser::desktop;
void check(bool value,const char* message){if(!value)throw std::runtime_error(message);}
void click(WindowsShortcuts& ui,int id){SendMessageW(ui.panel(),WM_COMMAND,MAKEWPARAM(id,BN_CLICKED),reinterpret_cast<LPARAM>(GetDlgItem(ui.panel(),id)));}
int main(){try{
  WNDCLASSW klass{};klass.lpfnWndProc=DefWindowProcW;klass.hInstance=GetModuleHandleW(nullptr);klass.lpszClassName=L"SanserShortcutRegression";RegisterClassW(&klass);
  HWND owner=CreateWindowW(klass.lpszClassName,L"Shortcuts regression",WS_OVERLAPPEDWINDOW,0,0,800,600,nullptr,nullptr,klass.hInstance,nullptr);
  check(owner!=nullptr,"native owner creation failed");
  bool available=false,focused=true,release=true;std::uint32_t epoch=1;unsigned begins=0,submits=0;std::vector<std::uint16_t> last;
  WindowsShortcuts ui(owner,{[&]{return available;},[&]{return focused;},[&]{return epoch;},[&]{++begins;return release;},[&](auto keys,auto generation){check(generation==epoch,"wrong generation");last.assign(keys.begin(),keys.end());++submits;return true;}});
  check(!ui.open() && !begins,"unsupported action opened/reset");available=true;release=false;check(!ui.open(),"refused reset still opened UI");release=true;
  check(ui.open(),"modeless panel not created");
  auto list=GetDlgItem(ui.panel(),WindowsShortcuts::listId);
  check(SendMessageW(list,LB_GETCOUNT,0,0)==0 && !IsWindowEnabled(GetDlgItem(ui.panel(),WindowsShortcuts::sendId)),"host OS guessed");
  click(ui,WindowsShortcuts::macId);check(SendMessageW(list,LB_GETCOUNT,0,0)==16,"Mac profile missing");
  SendMessageW(list,LB_SETCURSEL,0,0);ui.refresh();click(ui,WindowsShortcuts::sendId);
  check(submits==1 && last==std::vector<std::uint16_t>{227,43},"Windows UI used local OS keys for Mac host");
  const auto before=submits;
  SendMessageW(ui.panel(),WM_COMMAND,MAKEWPARAM(WindowsShortcuts::sendId,BN_CLICKED),0);
  check(submits==before,"forged/missing button source accepted");
  ++epoch;click(ui,WindowsShortcuts::sendId);check(submits==before && !ui.panel(),"stale generation submitted action");
  check(ui.open(),"cannot reopen");list=GetDlgItem(ui.panel(),WindowsShortcuts::listId);check(SendMessageW(list,LB_GETCOUNT,0,0)==0,"new panel inherited host");
  click(ui,WindowsShortcuts::windowsId);SendMessageW(list,LB_SETCURSEL,0,0);click(ui,WindowsShortcuts::sendId);
  check(last==std::vector<std::uint16_t>{226,43},"Windows host mapping");
  focused=false;ui.refresh();check(!ui.panel(),"focus loss retained panel");focused=true;check(ui.open(),"focus restore open");
  available=false;ui.refresh();check(!ui.panel(),"permission loss retained panel");
  available=true;check(ui.open(),"readiness restore open");ui.detach();check(!ui.panel() && !ui.open(),"detached panel reopened");DestroyWindow(owner);
  std::cout<<"Win32 shortcuts: modeless controls, reset refusal, host selection, button origin and focus/permission/generation fences passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
