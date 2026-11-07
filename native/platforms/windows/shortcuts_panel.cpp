#include "shortcuts_panel.h"
#include <algorithm>
#include <string>

namespace sanser::desktop {
namespace {constexpr wchar_t panelClass[]=L"SanserHostShortcuts";
std::wstring wide(std::string_view value){return std::wstring(value.begin(),value.end());}}
WindowsShortcuts::WindowsShortcuts(HWND owner,ShortcutSession::Callbacks callbacks):owner_(owner),session_(std::move(callbacks)) {
  WNDCLASSW klass{};klass.lpfnWndProc=procedure;klass.hInstance=GetModuleHandleW(nullptr);
  klass.lpszClassName=panelClass;klass.hCursor=LoadCursorW(nullptr,MAKEINTRESOURCEW(32512));klass.hbrBackground=GetSysColorBrush(COLOR_WINDOW);
  if(!RegisterClassW(&klass) && GetLastError()!=ERROR_CLASS_ALREADY_EXISTS)throw std::runtime_error("Unable to register shortcuts panel");
  if(auto menu=GetSystemMenu(owner_,FALSE))AppendMenuW(menu,MF_STRING|MF_GRAYED,menuCommand,L"Host shortcuts...\tCtrl+Alt+K");
}
WindowsShortcuts::~WindowsShortcuts(){cancel();}
bool WindowsShortcuts::open(){
  if(panel_)return session_.current();
  RECT area{};
  if(!owner_ || !GetClientRect(owner_,&area) || area.right<320 || area.bottom<240) {
    if(owner_)SetWindowTextW(owner_,L"Sanser - Enlarge viewer to open host shortcuts");return false;
  }
  if(!owner_ || !session_.open())return false;
  panel_=CreateWindowExW(WS_EX_CONTROLPARENT,panelClass,L"Host shortcuts",WS_CHILD|WS_CLIPCHILDREN,0,0,1,1,owner_,nullptr,GetModuleHandleW(nullptr),this);
  if(!panel_){session_.close();return false;}
  const auto child=[&](const wchar_t* kind,const wchar_t* text,DWORD style,int id){
    auto value=CreateWindowExW(0,kind,text,WS_CHILD|WS_VISIBLE|style,0,0,1,1,panel_,reinterpret_cast<HMENU>(static_cast<INT_PTR>(id)),GetModuleHandleW(nullptr),nullptr);
    if(value)SendMessageW(value,WM_SETFONT,reinterpret_cast<WPARAM>(GetStockObject(DEFAULT_GUI_FONT)),TRUE);return value;
  };
  title_=child(L"STATIC",L"Host shortcuts - choose the REMOTE computer's OS",SS_LEFT,10);
  windows_=child(L"BUTTON",L"Windows host",BS_AUTORADIOBUTTON|WS_TABSTOP|WS_GROUP,windowsId);
  mac_=child(L"BUTTON",L"Mac host",BS_AUTORADIOBUTTON|WS_TABSTOP,macId);
  list_=child(L"LISTBOX",L"",LBS_NOTIFY|WS_BORDER|WS_VSCROLL|WS_TABSTOP,listId);
  send_=child(L"BUTTON",L"Send shortcut",BS_PUSHBUTTON|WS_TABSTOP,sendId);
  close_=child(L"BUTTON",L"Close",BS_PUSHBUTTON|WS_TABSTOP,closeId);
  status_=child(L"STATIC",L"No OS is assumed. Copy/Paste uses the host clipboard.",SS_LEFT,11);
  if(!title_ || !windows_ || !mac_ || !list_ || !send_ || !close_ || !status_){cancel();return false;}
  // Focusing an auto-radio button can emit BN_CLICKED on Win32. Start on the
  // empty list so opening the panel cannot silently choose the remote OS.
  resize();if(!panel_)return false;ShowWindow(panel_,SW_SHOW);SetFocus(list_);refresh();return true;
}
void WindowsShortcuts::toggle(){if(panel_)cancel();else (void)open();}
void WindowsShortcuts::cancel(){
  session_.close();actions_.clear();const auto old=panel_;
  panel_=title_=windows_=mac_=list_=send_=close_=status_=nullptr;
  if(old && IsWindow(old))DestroyWindow(old);
}
void WindowsShortcuts::detach(){cancel();owner_=nullptr;}
void WindowsShortcuts::choose(ShortcutHost host){
  if(!session_.select(host))return;
  actions_=sessionShortcuts(host);SendMessageW(list_,LB_RESETCONTENT,0,0);
  for(const auto& action:actions_){const auto label=wide(action.label)+L"     "+wide(action.caption);SendMessageW(list_,LB_ADDSTRING,0,reinterpret_cast<LPARAM>(label.c_str()));}
  CheckRadioButton(panel_,windowsId,macId,host==ShortcutHost::Windows ? windowsId:macId);
  SetWindowTextW(status_,L"Choose an action, then Send. It affects the host's active window.");refresh();
}
void WindowsShortcuts::send(){
  const auto selected=SendMessageW(list_,LB_GETCURSEL,0,0);
  if(!session_.current() || selected==LB_ERR || static_cast<std::size_t>(selected)>=actions_.size()){refresh();return;}
  const bool queued=session_.send(actions_[selected].id);
  if(panel_)SetWindowTextW(status_,queued ? L"Shortcut queued. The host/application controls the result.":L"Not queued. Check the connection or try again after input settles.");
}
void WindowsShortcuts::refresh(){
  const int available=session_.available()?1:0;
  if(owner_ && available!=announcedAvailable_)if(auto menu=GetSystemMenu(owner_,FALSE)) {
    EnableMenuItem(menu,menuCommand,MF_BYCOMMAND|(available ? MF_ENABLED:MF_GRAYED));announcedAvailable_=available;
  }
  if(!panel_)return;
  if(!session_.current()){cancel();return;}
  const auto selected=SendMessageW(list_,LB_GETCURSEL,0,0);
  const bool selectedAction=selected!=LB_ERR && static_cast<std::size_t>(selected)<actions_.size();
  if((IsWindowEnabled(send_)!=FALSE)!=selectedAction)EnableWindow(send_,selectedAction);
}
void WindowsShortcuts::resize(){
  if(!owner_ || !panel_)return;
  RECT rect{};GetClientRect(owner_,&rect);
  if(rect.right<320 || rect.bottom<240){cancel();return;}
  const int width=std::min(580L,std::max(1L,rect.right-24)),height=std::min(540L,std::max(1L,rect.bottom-24));
  MoveWindow(panel_,std::max(0L,(rect.right-width)/2),std::max(0L,(rect.bottom-height)/2),width,height,TRUE);
  MoveWindow(title_,12,10,std::max(1,width-24),32,TRUE);
  MoveWindow(windows_,12,44,(width-24)/2,26,TRUE);MoveWindow(mac_,width/2,44,(width-24)/2,26,TRUE);
  MoveWindow(list_,12,78,std::max(1,width-24),std::max(1,height-168),TRUE);
  MoveWindow(status_,12,std::max(80,height-82),std::max(1,width-24),34,TRUE);
  MoveWindow(send_,12,std::max(120,height-40),std::max(1,(width-36)/2),28,TRUE);
  MoveWindow(close_,width/2+6,std::max(120,height-40),std::max(1,(width-36)/2),28,TRUE);
}
bool WindowsShortcuts::preTranslate(MSG& message){
  if(!panel_ || !owns(message.hwnd))return false;
  if(message.message==WM_KEYDOWN && message.wParam==VK_ESCAPE){cancel();return true;}
  return IsDialogMessageW(panel_,&message)!=FALSE;
}
LRESULT CALLBACK WindowsShortcuts::procedure(HWND window,UINT message,WPARAM w,LPARAM l){
  auto* self=reinterpret_cast<WindowsShortcuts*>(GetWindowLongPtrW(window,GWLP_USERDATA));
  if(message==WM_NCCREATE){self=static_cast<WindowsShortcuts*>(reinterpret_cast<CREATESTRUCTW*>(l)->lpCreateParams);SetWindowLongPtrW(window,GWLP_USERDATA,reinterpret_cast<LONG_PTR>(self));}
  return self ? self->process(window,message,w,l):DefWindowProcW(window,message,w,l);
}
LRESULT WindowsShortcuts::process(HWND window,UINT message,WPARAM w,LPARAM l){
  if(message==WM_CLOSE){cancel();return 0;}
  if(message==WM_COMMAND && panel_==window){
    const auto source=reinterpret_cast<HWND>(l);const auto id=LOWORD(w),notification=HIWORD(w);
    if(id==closeId && source==close_ && notification==BN_CLICKED){cancel();return 0;}
    if(!session_.current()){cancel();return 0;}
    if(id==windowsId && source==windows_ && notification==BN_CLICKED)choose(ShortcutHost::Windows);
    else if(id==macId && source==mac_ && notification==BN_CLICKED)choose(ShortcutHost::Mac);
    else if(id==sendId && source==send_ && notification==BN_CLICKED)send();
    else if(id==listId && source==list_ && notification==LBN_SELCHANGE)refresh();
    return 0;
  }
  return DefWindowProcW(window,message,w,l);
}
}
