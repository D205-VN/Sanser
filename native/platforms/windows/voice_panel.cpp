#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include "voice_panel.h"
#include <stdexcept>
namespace sanser::desktop {
struct VoicePanel::Impl {
  std::function<VoiceRuntime::Status()> read;
  std::function<void(voice::State)> write;
  HWND window=nullptr,toggle=nullptr,join=nullptr,microphone=nullptr,deafen=nullptr,leave=nullptr,status=nullptr;
  bool expanded=false;
  static LRESULT CALLBACK procedure(HWND window,UINT message,WPARAM w,LPARAM l) {
    auto* self=reinterpret_cast<Impl*>(GetWindowLongPtrW(window,GWLP_USERDATA));
    if(message==WM_NCCREATE) { self=static_cast<Impl*>(reinterpret_cast<CREATESTRUCTW*>(l)->lpCreateParams); SetWindowLongPtrW(window,GWLP_USERDATA,reinterpret_cast<LONG_PTR>(self)); }
    if(self && message==WM_TIMER) { self->refresh(); return 0; }
    if(self && message==WM_CLOSE) { self->write({false,false,false}); self->expanded=false; self->layout(); return 0; }
    if(self && message==WM_COMMAND && HIWORD(w)==BN_CLICKED) { self->activate(LOWORD(w)); return 0; }
    if(message==WM_MOUSEACTIVATE) return MA_NOACTIVATE;
    return DefWindowProcW(window,message,w,l);
  }
  explicit Impl(VoiceRuntime& value):Impl([&value] { return value.snapshot(); },[&value](voice::State state) { value.configure(state); }) {}
  Impl(std::function<VoiceRuntime::Status()> readStatus,std::function<void(voice::State)> configure):read(std::move(readStatus)),write(std::move(configure)) {
    WNDCLASSW klass{}; klass.lpfnWndProc=procedure; klass.hInstance=GetModuleHandleW(nullptr);
    klass.lpszClassName=L"SanserGroupVoicePanel"; klass.hbrBackground=reinterpret_cast<HBRUSH>(COLOR_WINDOW+1); klass.hCursor=LoadCursorW(nullptr,MAKEINTRESOURCEW(32512)); RegisterClassW(&klass);
    RECT area{}; SystemParametersInfoW(SPI_GETWORKAREA,0,&area,0);
    window=CreateWindowExW(WS_EX_TOPMOST|WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE,klass.lpszClassName,L"Sanser voice",WS_POPUP|WS_BORDER,area.right-340,area.top+80,320,44,nullptr,nullptr,klass.hInstance,this);
    if(!window) throw std::runtime_error("Unable to create floating voice controls");
    const auto button=[&](const wchar_t* title,int id,int x,int y,int width) {
      auto result=CreateWindowW(L"BUTTON",title,WS_CHILD|WS_VISIBLE|BS_PUSHBUTTON,x,y,width,30,window,reinterpret_cast<HMENU>(static_cast<INT_PTR>(id)),klass.hInstance,nullptr);
      SendMessageW(result,WM_SETFONT,reinterpret_cast<WPARAM>(GetStockObject(DEFAULT_GUI_FONT)),TRUE); return result;
    };
    toggle=button(L"Sanser · Voice",10,8,5,304); join=button(L"Join voice",11,8,46,145);
    microphone=button(L"Unmute mic",12,163,46,149); deafen=button(L"Deafen voice",13,8,84,145); leave=button(L"Leave voice",14,163,84,149);
    status=CreateWindowW(L"STATIC",L"",WS_CHILD|WS_VISIBLE,12,124,296,36,window,nullptr,klass.hInstance,nullptr);
    SendMessageW(status,WM_SETFONT,reinterpret_cast<WPARAM>(GetStockObject(DEFAULT_GUI_FONT)),TRUE);
    SetTimer(window,1,100,nullptr); layout(); refresh(); ShowWindow(window,SW_SHOWNOACTIVATE);
  }
  void layout() {
    SetWindowPos(window,HWND_TOPMOST,0,0,320,expanded ? 168 : 44,SWP_NOMOVE|SWP_NOACTIVATE);
    for(auto child:{join,microphone,deafen,leave,status}) ShowWindow(child,expanded ? SW_SHOWNOACTIVATE : SW_HIDE);
  }
  void activate(unsigned id) {
    if(id==10) { expanded=!expanded; layout(); return; }
    auto next=read().state;
    if(id==11) next={true,false,false};
    if(id==12 && next.joined) { next.microphone=!next.microphone; next.deafened=false; }
    if(id==13 && next.joined) { next.deafened=!next.deafened; next.microphone=false; }
    if(id==14) next={false,false,false};
    write(next);
  }
  void refresh() {
    const auto value=read(); const auto& state=value.state;
    SetWindowTextW(toggle,value.capturing ? L"Sanser · Voice · Mic on" : state.joined ? L"Sanser · Voice · Mic off" : L"Sanser · Voice");
    EnableWindow(join,!state.joined); EnableWindow(microphone,state.joined); EnableWindow(deafen,state.joined); EnableWindow(leave,state.joined);
    SetWindowTextW(microphone,state.microphone ? L"Mute mic" : L"Unmute mic"); SetWindowTextW(deafen,state.deafened ? L"Hear voice" : L"Deafen voice");
    SetWindowTextW(status,value.code=="permission-denied" ? L"Allow microphone in Windows privacy settings." : value.code=="no-device" ? L"No microphone is available." : value.code=="unavailable" ? L"Microphone unavailable. Unmute to retry." : value.capturing ? L"Microphone on" : state.microphone ? L"Opening microphone…" : state.deafened ? L"Voice deafened · microphone off" : state.joined ? L"Listening · microphone off" : L"Join to listen. Unmute to talk.");
  }
  ~Impl() { if(window) { KillTimer(window,1); DestroyWindow(window); } }
};
VoicePanel::VoicePanel(bool enabled,VoiceRuntime& voice) { if(enabled) impl_=std::make_unique<Impl>(voice); }
VoicePanel::~VoicePanel()=default;
#ifdef SANSER_CODEC_TEST
void testVoicePanelControls() {
  VoiceRuntime::Status status; unsigned changes=0;
  VoicePanel::Impl panel([&] { return status; },[&](voice::State value) { status.state=value; ++changes; });
  if(!panel.window || changes || !(GetWindowLongPtrW(panel.window,GWL_EXSTYLE)&WS_EX_NOACTIVATE)) throw std::runtime_error("Floating voice must start without capture or stealing activation");
  SendMessageW(panel.window,WM_COMMAND,MAKEWPARAM(10,BN_CLICKED),0);
  if(!panel.expanded || changes) throw std::runtime_error("Opening voice controls must not join or capture");
  SendMessageW(panel.window,WM_COMMAND,MAKEWPARAM(11,BN_CLICKED),0);
  if(!status.state.joined || status.state.microphone) throw std::runtime_error("Floating Join must listen only");
  SendMessageW(panel.window,WM_COMMAND,MAKEWPARAM(12,BN_CLICKED),0);
  if(!status.state.microphone) throw std::runtime_error("Floating Unmute did not request the microphone");
  const auto beforeCollapse=changes;
  SendMessageW(panel.window,WM_COMMAND,MAKEWPARAM(10,BN_CLICKED),0); SendMessageW(panel.window,WM_COMMAND,MAKEWPARAM(10,BN_CLICKED),0);
  if(changes!=beforeCollapse || !status.state.microphone) throw std::runtime_error("Collapsing floating controls must not leave the call");
  SendMessageW(panel.window,WM_COMMAND,MAKEWPARAM(12,BN_CLICKED),0);
  if(status.state.microphone) throw std::runtime_error("Floating Mute did not revoke microphone consent");
  SendMessageW(panel.window,WM_COMMAND,MAKEWPARAM(12,BN_CLICKED),0);
  SendMessageW(panel.window,WM_COMMAND,MAKEWPARAM(13,BN_CLICKED),0);
  if(!status.state.deafened || status.state.microphone) throw std::runtime_error("Floating Deafen must mute capture");
  SendMessageW(panel.window,WM_COMMAND,MAKEWPARAM(14,BN_CLICKED),0);
  if(status.state.joined || status.state.microphone) throw std::runtime_error("Floating Leave must revoke microphone consent");
}
#endif
}
