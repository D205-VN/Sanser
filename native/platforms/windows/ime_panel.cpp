#include "ime_panel.h"
#include "ime_commit.h"
#include <imm.h>
#include <algorithm>
#include <stdexcept>
#include <utility>

namespace sanser::desktop {
namespace {
constexpr UINT refreshMessage=WM_APP+73,cancelMessage=WM_APP+74;
constexpr wchar_t panelClass[]=L"SanserLocalImeDraft";
constexpr std::size_t localDraftLimit=4096;
bool modifier(int key) { return (GetKeyState(key)&0x8000)!=0; }
}
WindowsTextInput::WindowsTextInput(HWND owner,Callbacks callbacks):owner_(owner),callbacks_(std::move(callbacks)) {
  if(!owner || !callbacks_.available || !callbacks_.focused || !callbacks_.generation || !callbacks_.submit || !callbacks_.begin) throw std::invalid_argument("Incomplete native text callbacks");
  WNDCLASSW klass{}; klass.hInstance=GetModuleHandleW(nullptr); klass.lpfnWndProc=procedure;
  klass.lpszClassName=panelClass; klass.hCursor=LoadCursorW(nullptr,MAKEINTRESOURCEW(32512)); klass.hbrBackground=GetSysColorBrush(COLOR_WINDOW);
  if(!RegisterClassW(&klass) && GetLastError()!=ERROR_CLASS_ALREADY_EXISTS) throw std::runtime_error("Unable to register native text panel");
  if(auto menu=GetSystemMenu(owner_,FALSE)) {
    AppendMenuW(menu,MF_SEPARATOR,0,nullptr);
    AppendMenuW(menu,MF_STRING|MF_GRAYED,menuCommand,L"Text input...\tCtrl+Alt+I");
  }
}
WindowsTextInput::~WindowsTextInput() { cancel(); }
bool WindowsTextInput::current() const {
  return panel_ && !fenced_ && epoch_ && callbacks_.available() && callbacks_.generation()==epoch_;
}
bool WindowsTextInput::open() {
  if(panel_) return true;
  if(!owner_ || !callbacks_.available() || !callbacks_.focused()) return false;
  if(serial_==~UINT_PTR{0}) return false;
  const auto epoch=callbacks_.generation(); if(!epoch) return false;
  callbacks_.begin();
  if(!callbacks_.available() || callbacks_.generation()!=epoch) return false;
  ++serial_; epoch_=epoch; fenced_=composing_=sending_=false; notice_.clear();
  panel_=CreateWindowExW(WS_EX_CONTROLPARENT,panelClass,L"Text input",WS_CHILD|WS_CLIPCHILDREN,0,0,1,1,owner_,nullptr,GetModuleHandleW(nullptr),this);
  if(!panel_) { epoch_=0; return false; }
  const auto child=[&](const wchar_t* kind,const wchar_t* text,DWORD style,int id) {
    auto window=CreateWindowExW(0,kind,text,WS_CHILD|WS_VISIBLE|style,0,0,1,1,panel_,reinterpret_cast<HMENU>(static_cast<INT_PTR>(id)),GetModuleHandleW(nullptr),nullptr);
    if(window) SendMessageW(window,WM_SETFONT,reinterpret_cast<WPARAM>(GetStockObject(DEFAULT_GUI_FONT)),TRUE);
    return window;
  };
  title_=child(L"STATIC",L"Text input · compose locally, then Send (Ctrl+Enter)",SS_LEFT,101);
  edit_=child(L"EDIT",L"",WS_TABSTOP|WS_BORDER|WS_VSCROLL|ES_MULTILINE|ES_WANTRETURN|ES_AUTOVSCROLL,editId);
  status_=child(L"STATIC",L"",SS_LEFT,102);
  send_=child(L"BUTTON",L"Send",WS_TABSTOP|BS_PUSHBUTTON,sendId);
  cancel_=child(L"BUTTON",L"Cancel",WS_TABSTOP|BS_PUSHBUTTON,cancelId);
  if(!title_ || !edit_ || !status_ || !send_ || !cancel_) { cancel(); return false; }
  // A larger local editing limit avoids cutting a candidate at the wire limit.
  // Submission independently validates the complete final 256-unit edit.
  SendMessageW(edit_,EM_SETLIMITTEXT,localDraftLimit,0);
  if(!SetWindowSubclass(edit_,editProcedure,1,reinterpret_cast<DWORD_PTR>(this))) { cancel(); return false; }
  resize(); ShowWindow(panel_,SW_SHOW); SetFocus(edit_);
  SetWindowTextW(owner_,L"Sanser · Local text draft · Ctrl+Enter: Send · Esc: Cancel");
  refresh(); return true;
}
void WindowsTextInput::toggle() { if(panel_) cancel(true); else (void)open(); }
void WindowsTextInput::cancel(bool restoreFocus) {
  if(closing_ || !panel_) return;
  closing_=true; fenced_=true; epoch_=0;
  const auto panel=panel_,edit=edit_;
  // Fence before canceling the IME: synchronous finalization/EN_CHANGE messages
  // must not become network submissions or recreate the old draft.
  panel_=edit_=title_=status_=send_=cancel_=nullptr;
  composing_=false; notice_.clear();
  if(edit && IsWindow(edit)) {
    if(auto context=ImmGetContext(edit)) { ImmNotifyIME(context,NI_COMPOSITIONSTR,CPS_CANCEL,0); ImmReleaseContext(edit,context); }
    RemoveWindowSubclass(edit,editProcedure,1);
  }
  if(IsWindow(panel)) DestroyWindow(panel);
  if(owner_ && IsWindow(owner_)) SetWindowTextW(owner_,L"Sanser · Input released · Click the stream to resume");
  if(restoreFocus && owner_ && IsWindow(owner_) && GetForegroundWindow()==owner_) SetFocus(owner_);
  closing_=false;
}
void WindowsTextInput::detach() { cancel(); owner_=nullptr; }
bool WindowsTextInput::hasComposition() const {
  if(composing_) return true;
  if(!edit_) return false;
  if(auto context=ImmGetContext(edit_)) {
    const auto count=ImmGetCompositionStringW(context,GCS_COMPSTR,nullptr,0); ImmReleaseContext(edit_,context);
    return count>0;
  }
  return false;
}
std::optional<std::u16string> WindowsTextInput::draft() const {
  static_assert(sizeof(wchar_t)==sizeof(char16_t));
  if(!edit_) return std::nullopt;
  const int length=GetWindowTextLengthW(edit_);
  if(length<0 || static_cast<std::size_t>(length)>localDraftLimit) return std::nullopt;
  std::wstring buffer(static_cast<std::size_t>(length)+1,L'\0');
  const auto copied=GetWindowTextW(edit_,buffer.data(),static_cast<int>(buffer.size()));
  if(copied!=length) return std::nullopt;
  std::u16string result; result.reserve(static_cast<std::size_t>(copied));
  for(int i=0;i<copied;++i) result.push_back(static_cast<char16_t>(buffer[static_cast<std::size_t>(i)]));
  return result;
}
void WindowsTextInput::refresh() {
  if(owner_) if(auto menu=GetSystemMenu(owner_,FALSE)) {
    EnableMenuItem(menu,menuCommand,MF_BYCOMMAND|(callbacks_.available() ? MF_ENABLED:MF_GRAYED));
    CheckMenuItem(menu,menuCommand,MF_BYCOMMAND|(panel_ ? MF_CHECKED:MF_UNCHECKED));
  }
  if(!panel_) return;
  if(!current()) { cancel(); return; }
  const auto value=draft();
  const auto encoded=value ? sanser::imeCommitUtf8(*value):std::nullopt;
  const bool composing=hasComposition();
  EnableWindow(send_,!sending_ && !composing && encoded.has_value());
  std::wstring status=notice_;
  if(status.empty()) {
    if(composing) status=L"Choosing characters · finish the candidate before sending.";
    else if(!value || (!value->empty() && !encoded)) status=L"Text is too long or unsupported. Emoji and accents may count more than once.";
    else status=L"Only Send types this draft on the host. Local editing does not change host text.";
  }
  // Avoid repainting controls on every media wake if no UI state changed.
  wchar_t old[256]{}; GetWindowTextW(status_,old,256);
  if(status!=old) SetWindowTextW(status_,status.c_str());
}
void WindowsTextInput::submit() {
  if(!panel_ || sending_ || hasComposition()) return;
  if(!current() || !callbacks_.focused()) { cancel(); return; }
  const auto value=draft(); const auto text=value ? sanser::imeCommitUtf8(*value):std::nullopt;
  if(!text) { refresh(); return; }
  const auto epoch=epoch_; sending_=true; EnableWindow(send_,FALSE);
  bool accepted=false;
  try { accepted=current() && callbacks_.focused() && callbacks_.submit(*text,epoch); } catch(...) { /* Keep draft; never auto-retry an ambiguous commit. */ }
  sending_=false;
  if(!panel_) return;
  if(accepted) { cancel(true); return; }
  if(!current()) { cancel(); return; }
  notice_=L"Text was not confirmed as queued. Review the draft and connection before trying again.";
  refresh();
}
void WindowsTextInput::resize() {
  if(!panel_ || !owner_) return;
  RECT bounds{}; GetClientRect(owner_,&bounds);
  const auto dpi=GetDpiForWindow(owner_); const auto px=[&](int value) { return MulDiv(value,static_cast<int>(dpi ? dpi:96),96); };
  const int margin=px(12),width=std::max(1,std::min(px(760),static_cast<int>(bounds.right)-margin*2));
  const int height=std::max(1,std::min(px(222),static_cast<int>(bounds.bottom)-margin*2));
  MoveWindow(panel_,margin,margin,width,height,TRUE);
  const int inner=std::max(1,width-margin*2),buttons=px(30),bottom=std::max(margin,height-margin-buttons);
  const int buttonWidth=std::max(1,std::min(px(90),(inner-px(8))/2));
  MoveWindow(title_,margin,px(10),inner,px(30),TRUE);
  MoveWindow(edit_,margin,px(42),inner,std::max(1,bottom-px(88)),TRUE);
  MoveWindow(status_,margin,std::max(px(43),bottom-px(42)),inner,px(38),TRUE);
  MoveWindow(send_,margin,bottom,buttonWidth,buttons,TRUE);
  MoveWindow(cancel_,margin+buttonWidth+px(8),bottom,buttonWidth,buttons,TRUE);
}
bool WindowsTextInput::preTranslate(MSG& message) {
  if(!panel_ || !owns(message.hwnd)) return false;
  if(message.message==WM_KEYDOWN && !hasComposition()) {
    if(message.wParam==VK_ESCAPE) { cancel(true); return true; }
    if(message.wParam==VK_RETURN && message.hwnd==send_) { if(!(message.lParam&(1LL<<30))) submit(); return true; }
    if(message.wParam==VK_RETURN && message.hwnd==cancel_) { cancel(true); return true; }
  }
  return IsDialogMessageW(panel_,&message);
}
LRESULT CALLBACK WindowsTextInput::procedure(HWND window,UINT message,WPARAM w,LPARAM l) {
  auto* self=reinterpret_cast<WindowsTextInput*>(GetWindowLongPtrW(window,GWLP_USERDATA));
  if(message==WM_NCCREATE) { self=static_cast<WindowsTextInput*>(reinterpret_cast<CREATESTRUCTW*>(l)->lpCreateParams); SetWindowLongPtrW(window,GWLP_USERDATA,reinterpret_cast<LONG_PTR>(self)); }
  if(!self) return DefWindowProcW(window,message,w,l);
  try { return self->process(window,message,w,l); } catch(...) { self->fenced_=true; PostMessageW(window,cancelMessage,self->serial_,0); return 0; }
}
LRESULT WindowsTextInput::process(HWND window,UINT message,WPARAM w,LPARAM l) {
  if(message==WM_COMMAND) {
    if(LOWORD(w)==sendId && HIWORD(w)==BN_CLICKED && send_ && reinterpret_cast<HWND>(l)==send_) { submit(); return 0; }
    if(LOWORD(w)==cancelId && HIWORD(w)==BN_CLICKED && cancel_ && reinterpret_cast<HWND>(l)==cancel_) { cancel(true); return 0; }
    if(LOWORD(w)==editId && HIWORD(w)==EN_CHANGE) { notice_.clear(); if(panel_) PostMessageW(panel_,refreshMessage,serial_,0); return 0; }
  }
  if(message==refreshMessage) { if(w==serial_) refresh(); return 0; }
  if(message==cancelMessage) { if(w==serial_) cancel(); return 0; }
  if(message==WM_NCDESTROY) { if(panel_==window) { panel_=edit_=title_=status_=send_=cancel_=nullptr; epoch_=0; fenced_=true; } SetWindowLongPtrW(window,GWLP_USERDATA,0); }
  return DefWindowProcW(window,message,w,l);
}
LRESULT CALLBACK WindowsTextInput::editProcedure(HWND window,UINT message,WPARAM w,LPARAM l,UINT_PTR id,DWORD_PTR data) {
  auto* self=reinterpret_cast<WindowsTextInput*>(data);
  if(message==WM_NCDESTROY) { RemoveWindowSubclass(window,editProcedure,id); return DefSubclassProc(window,message,w,l); }
  if(!self || self->closing_ || self->edit_!=window) return DefSubclassProc(window,message,w,l);
  try {
    if(message==WM_IME_STARTCOMPOSITION) {
      self->composing_=true;
      if(self->send_) EnableWindow(self->send_,FALSE);
      if(self->panel_) PostMessageW(self->panel_,refreshMessage,self->serial_,0);
    }
    if(message==WM_IME_ENDCOMPOSITION) {
      const auto result=DefSubclassProc(window,message,w,l);
      self->composing_=false; if(self->panel_) PostMessageW(self->panel_,refreshMessage,self->serial_,0); return result;
    }
    if(message==WM_GETDLGCODE && (self->hasComposition() || w==VK_RETURN)) return DefSubclassProc(window,message,w,l)|DLGC_WANTALLKEYS;
    if(message==WM_KEYDOWN || message==WM_SYSKEYDOWN) {
      if(w=='I' && modifier(VK_CONTROL) && modifier(VK_MENU) && !modifier(VK_RMENU)) { if(!(l&(1LL<<30))) self->cancel(true); return 0; }
      if(w==VK_ESCAPE && modifier(VK_CONTROL) && modifier(VK_MENU)) { self->cancel(true); return 0; }
      if(w==VK_ESCAPE && !self->hasComposition()) { self->cancel(true); return 0; }
      if(w==VK_RETURN && modifier(VK_CONTROL) && !self->hasComposition()) { if(!(l&(1LL<<30))) self->submit(); return 0; }
    }
    if(message==WM_KILLFOCUS && self->panel_ && !self->owns(reinterpret_cast<HWND>(w))) {
      self->fenced_=true; PostMessageW(self->panel_,cancelMessage,self->serial_,0);
    }
  } catch(...) { self->fenced_=true; if(self->panel_) PostMessageW(self->panel_,cancelMessage,self->serial_,0); return 0; }
  return DefSubclassProc(window,message,w,l);
}
}
