#include "controller_input.h"
#include <xinput.h>
#include <memory>
namespace sanser::desktop {
namespace {
class XInputSource {
public:
  XInputSource(){for(const auto name:{L"xinput1_4.dll",L"xinput9_1_0.dll"}){
    module_=LoadLibraryExW(name,nullptr,LOAD_LIBRARY_SEARCH_SYSTEM32);
    if(module_){get_=reinterpret_cast<Get>(GetProcAddress(module_,"XInputGetState"));if(get_)break;FreeLibrary(module_);module_=nullptr;}
  }}
  ~XInputSource(){if(module_)FreeLibrary(module_);}
  ControllerSample read(){
    if(!get_)return {};
    XINPUT_STATE state{};
    if(index_<4){if(get_(index_,&state)==ERROR_SUCCESS)return sample(state.Gamepad);index_=4;return {};}
    const auto now=std::chrono::steady_clock::now();if(now<retry_)return {};retry_=now+std::chrono::milliseconds(500);
    for(DWORD index=0;index<4;++index)if(get_(index,&state)==ERROR_SUCCESS){index_=index;return sample(state.Gamepad);}return {};
  }
private:
  static ControllerSample sample(XINPUT_GAMEPAD value){
    ControllerSample s;s.connected=s.sampled=1;
    constexpr WORD masks[]={0x1000,0x2000,0x4000,0x8000,0x100,0x200,0x10,0x20,0x40,0x80,1,2,4,8};
    for(unsigned i=0;i<14;++i)if(value.wButtons&masks[i])s.buttons|=1U<<i;
    const auto axis=[](SHORT n){return n/static_cast<double>(n<0?32768:32767);};
    s.lx=axis(value.sThumbLX);s.ly=axis(value.sThumbLY);s.rx=axis(value.sThumbRX);s.ry=axis(value.sThumbRY);
    s.lt=value.bLeftTrigger/255.0;s.rt=value.bRightTrigger/255.0;return s;
  }
  using Get=DWORD(WINAPI*)(DWORD,XINPUT_STATE*);HMODULE module_=nullptr;Get get_=nullptr;DWORD index_=4;
  std::chrono::steady_clock::time_point retry_{};
};
}
WindowsControllerInput::WindowsControllerInput(HWND owner,ControllerSession& session,std::function<bool()> focused,std::function<ControllerSample()> source)
 :owner_(owner),session_(&session),focused_(std::move(focused)),source_(std::move(source)) {
  if(!source_){auto device=std::make_shared<XInputSource>();source_=[device]{return device->read();};}
  AppendMenuW(GetSystemMenu(owner_,FALSE),MF_STRING|MF_GRAYED,menuCommand,L"Analog controller (off) · Ctrl+Alt+G");refresh();
}
void WindowsControllerInput::refresh(){
  if(!session_)return;session_->refresh(focused_ && focused_());
  const auto now=std::chrono::steady_clock::now();
  if(session_->active() && now-polled_>=std::chrono::milliseconds(16)){
    polled_=now;session_->sample(source_());session_->refresh(focused_ && focused_());
  }
  if(session_->active()!=timer_){timer_=session_->active();if(timer_)SetTimer(owner_,timerId,16,nullptr);else KillTimer(owner_,timerId);}
  const auto status=session_->status();const bool enabled=session_->available();
  std::wstring title=!session_->active()?L"Analog controller (off) · Ctrl+Alt+G":
    !(status&8)?L"Analog controller: connect gamepad · Ctrl+Alt+G":
    !(status&4)?L"Analog controller: center sticks · Ctrl+Alt+G":L"Analog controller: active · Ctrl+Alt+G";
  if(caption_!=title || enabled_!=enabled){caption_=std::move(title);enabled_=enabled;
    ModifyMenuW(GetSystemMenu(owner_,FALSE),menuCommand,MF_BYCOMMAND|MF_STRING|(enabled?MF_ENABLED:MF_GRAYED),menuCommand,caption_.c_str());}
}
void WindowsControllerInput::toggle(){if(session_){session_->refresh(focused_ && focused_());session_->toggle();refresh();}}
void WindowsControllerInput::stop(){if(session_)session_->stop();refresh();}
void WindowsControllerInput::detach(){if(session_)session_->stop();session_=nullptr;
  if(owner_){KillTimer(owner_,timerId);if(IsWindow(owner_))RemoveMenu(GetSystemMenu(owner_,FALSE),menuCommand,MF_BYCOMMAND);}
  owner_=nullptr;timer_=false;source_={};focused_={};}
}
