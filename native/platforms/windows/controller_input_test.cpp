#include "controller_input.h"
#include <iostream>
#include <stdexcept>
using namespace sanser::desktop;
void check(bool value,const char* text){if(!value)throw std::runtime_error(text);}
int main(){try{
  std::atomic<bool> available{false};bool focus=true;unsigned resets=0,reads=0;
  ControllerSession session({[&]{return available.load();},[]{return 1U;},[&](auto){++resets;return true;},[](auto,auto){return true;}});
  HWND window=CreateWindowExW(0,L"STATIC",L"Controller test",WS_OVERLAPPEDWINDOW,0,0,800,600,nullptr,nullptr,GetModuleHandleW(nullptr),nullptr);
  check(window,"window creation");
  WindowsControllerInput ui(window,session,[&]{return focus;},[&]{++reads;ControllerSample s;s.connected=s.sampled=1;return s;});
  ui.toggle();check(!session.active() && !resets,"unsupported action reset");
  available=true;ui.refresh();ui.toggle();check(session.active() && reads==1 && (session.status()&4),"native source did not arm");
  focus=false;ui.refresh();check(!session.active(),"focus did not cancel");focus=true;ui.refresh();check(!session.active(),"focus auto-rearmed");
  ui.toggle();available=false;ui.refresh();check(!session.active(),"capability loss retained capture");
  available=true;ui.refresh();ui.toggle();ui.stop();check(!session.active(),"reset did not stop");
  ui.detach();ui.toggle();check(!session.active() && GetMenuState(GetSystemMenu(window,FALSE),WindowsControllerInput::menuCommand,MF_BYCOMMAND)==UINT(-1),"detach retained menu or capture");
  DestroyWindow(window);std::cout<<"Win32 analog: menu, fake source, focus/capability gates and detach passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
