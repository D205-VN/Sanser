#include "gamepad.h"
#include <atomic>
#include <mutex>
#include <utility>
#ifdef _WIN32
#include <windows.h>
#include <array>
#include <string>
#endif
namespace sanser::desktop {
namespace {
#ifdef _WIN32
// ABI only; no third-party implementation or driver is bundled. Report layout
// matches the public ViGEmClient XUSB_REPORT (12 bytes, natural alignment).
struct Report {std::uint16_t buttons;std::uint8_t lt,rt;std::int16_t lx,ly,rx,ry;};
static_assert(sizeof(Report)==12);
class Vigem {
public:
  ~Vigem(){detach();if(client_ && disconnect_)disconnect_(client_);if(client_ && free_)free_(client_);if(module_)FreeLibrary(module_);}
  bool open() {
    std::wstring path(32768,L'\0');const auto n=GetModuleFileNameW(nullptr,path.data(),static_cast<DWORD>(path.size()));
    if(!n || n>=path.size())return false;path.resize(n);
    const auto slash=path.find_last_of(L"\\/");if(slash==std::wstring::npos)return false;
    path.resize(slash+1);path+=L"ViGEmClient.dll";
    // Never search CWD/PATH or a download directory for a privileged DLL.
    module_=LoadLibraryExW(path.c_str(),nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32);
    if(!module_)return false;
    if(!load(alloc_,"vigem_alloc") || !load(free_,"vigem_free") || !load(connect_,"vigem_connect") ||
       !load(disconnect_,"vigem_disconnect") || !load(create_,"vigem_target_x360_alloc") ||
       !load(destroy_,"vigem_target_free") || !load(add_,"vigem_target_add") ||
       !load(remove_,"vigem_target_remove") || !load(update_,"vigem_target_x360_update"))return false;
    client_=alloc_();return client_ && connect_(client_)==0x20000000U;
  }
  bool attach(){if(target_)return true;target_=create_();if(!target_)return false;
    if(add_(client_,target_)==0x20000000U)return true;destroy_(target_);target_=nullptr;return false;}
  bool update(GamepadState s){return target_ && update_(client_,target_,{s.buttons,s.lt,s.rt,s.lx,s.ly,s.rx,s.ry})==0x20000000U;}
  void detach(){if(!target_)return;(void)update_(client_,target_,{});remove_(client_,target_);destroy_(target_);target_=nullptr;}
private:
  template<class T>bool load(T& proc,const char* name){proc=reinterpret_cast<T>(GetProcAddress(module_,name));return proc!=nullptr;}
  HMODULE module_=nullptr;void* client_=nullptr;void* target_=nullptr;
  void* (__cdecl *alloc_)()=nullptr;void (__cdecl *free_)(void*)=nullptr;
  std::uint32_t (__cdecl *connect_)(void*)=nullptr;void (__cdecl *disconnect_)(void*)=nullptr;
  void* (__cdecl *create_)()=nullptr;void (__cdecl *destroy_)(void*)=nullptr;
  std::uint32_t (__cdecl *add_)(void*,void*)=nullptr;std::uint32_t (__cdecl *remove_)(void*,void*)=nullptr;
  std::uint32_t (__cdecl *update_)(void*,void*,Report)=nullptr;
};
#endif
WindowsGamepad::Driver optionalDriver(bool enabled){
#ifdef _WIN32
  if(enabled){auto driver=std::make_shared<Vigem>();if(driver->open())return {
    [driver]{return driver->attach();},[driver](auto state){return driver->update(state);},[driver]{driver->detach();}};}
#else
  (void)enabled;
#endif
  return {};
}
}
struct WindowsGamepad::State {
  explicit State(Driver value):driver(std::move(value)),ready(driver.attach && driver.update && driver.detach){}
  Driver driver;std::atomic<bool> ready;bool attached=false;std::mutex mutex;
};
WindowsGamepad::WindowsGamepad(bool enabled):WindowsGamepad(optionalDriver(enabled)){}
WindowsGamepad::WindowsGamepad(Driver driver):state_(std::make_unique<State>(std::move(driver))){}
WindowsGamepad::~WindowsGamepad(){(void)apply({});}
bool WindowsGamepad::available()const{return state_->ready;}
bool WindowsGamepad::apply(GamepadState value){
  std::lock_guard lock(state_->mutex);
  if(!validGamepad(value))return false;
  if(!value.connected){if(state_->attached){state_->driver.detach();state_->attached=false;}return true;}
  if(!state_->ready)return false;
  if(!state_->attached){if(!state_->driver.attach()){state_->ready=false;return false;}state_->attached=true;}
  if(state_->driver.update(value))return true;
  state_->driver.detach();state_->attached=false;state_->ready=false;return false;
}
}
