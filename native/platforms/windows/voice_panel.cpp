#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <commctrl.h>
#include <objidl.h>
#include <gdiplus.h>
#include <uxtheme.h>
#include "voice_panel.h"
#include "external_voice.h"
#include "collab.h"
#include "transport.h"
#include "../../common/design_tokens.h"
#include <algorithm>
#include <cmath>
#include <cwchar>
#include <deque>
#include <string>
#include <stdexcept>
#include <vector>
#ifdef SANSER_CODEC_TEST
#include <cstdlib>
#include <filesystem>
#endif
namespace sanser::desktop {
namespace {
constexpr COLORREF panelColor(std::uint32_t rgb) { return RGB((rgb>>16)&255,(rgb>>8)&255,rgb&255); }
constexpr COLORREF surface=panelColor(design::raised),ink=panelColor(design::text),muted=panelColor(design::muted),accent=panelColor(design::success),red=panelColor(design::danger),edge=panelColor(design::borderStrong);
Gdiplus::Color tint(COLORREF value) { return Gdiplus::Color(255,GetRValue(value),GetGValue(value),GetBValue(value)); }
Gdiplus::Rect pixels(RECT value) {
  return Gdiplus::Rect(static_cast<INT>(value.left),static_cast<INT>(value.top),static_cast<INT>(value.right-value.left),static_cast<INT>(value.bottom-value.top));
}
struct DrawingSession {
  ULONG_PTR token=0;
  DrawingSession() {
    Gdiplus::GdiplusStartupInput input;
    if(Gdiplus::GdiplusStartup(&token,&input,nullptr)!=Gdiplus::Ok) throw std::runtime_error("Unable to initialize floating controls drawing");
  }
  ~DrawingSession() { if(token) Gdiplus::GdiplusShutdown(token); }
};
// Paint a complete control before presenting it; state/hover updates never
// expose the native button erase pass or a partially drawn icon.
struct PaintBuffer {
  HDC target,dc; HBITMAP bitmap=nullptr; HGDIOBJ previous=nullptr; int width,height;
  PaintBuffer(HDC destination,RECT bounds):target(destination),dc(destination),width(bounds.right-bounds.left),height(bounds.bottom-bounds.top) {
    auto memory=CreateCompatibleDC(target); if(!memory) return;
    bitmap=CreateCompatibleBitmap(target,width,height);
    if(!bitmap) { DeleteDC(memory); return; }
    dc=memory; previous=SelectObject(dc,bitmap);
  }
  ~PaintBuffer() {
    if(bitmap) { BitBlt(target,0,0,width,height,dc,0,0,SRCCOPY); SelectObject(dc,previous); DeleteObject(bitmap); DeleteDC(dc); }
  }
};
std::wstring wide(const std::string& value) {
  if(value.empty()) return {};
  const auto size=MultiByteToWideChar(CP_UTF8,0,value.data(),static_cast<int>(value.size()),nullptr,0);
  std::wstring out(static_cast<std::size_t>(std::max(size,0)),L'\0');
  if(size>0) MultiByteToWideChar(CP_UTF8,0,value.data(),static_cast<int>(value.size()),out.data(),size);
  return out;
}
std::string narrow(const std::wstring& value) {
  if(value.empty()) return {};
  const auto size=WideCharToMultiByte(CP_UTF8,0,value.data(),static_cast<int>(value.size()),nullptr,0,nullptr,nullptr);
  std::string out(static_cast<std::size_t>(std::max(size,0)),'\0');
  if(size>0) WideCharToMultiByte(CP_UTF8,0,value.data(),static_cast<int>(value.size()),out.data(),size,nullptr,nullptr);
  return out;
}
void roundedPath(Gdiplus::GraphicsPath& path,float left,float top,float right,float bottom,float diameter) {
  diameter=std::min(diameter,std::min(right-left,bottom-top));
  path.AddArc(left,top,diameter,diameter,180,90);
  path.AddArc(right-diameter,top,diameter,diameter,270,90);
  path.AddArc(right-diameter,bottom-diameter,diameter,diameter,0,90);
  path.AddArc(left,bottom-diameter,diameter,diameter,90,90); path.CloseFigure();
}
}
struct VoicePanel::Impl {
  DrawingSession drawing;
  std::function<VoiceRuntime::Status()> read;
  std::function<void(voice::State)> write;
  std::function<void()> stop;
  std::function<std::vector<std::string>()> chatLines;
  std::function<void(std::string)> chatSend;
  VoiceRuntime::Status current;
  HWND window=nullptr,toggle=nullptr,shortcut=nullptr,chevron=nullptr,stopButton=nullptr,join=nullptr,microphone=nullptr,deafen=nullptr,leave=nullptr,tooltip=nullptr,hovered=nullptr;
  HWND chatLog=nullptr,chatInput=nullptr,chatButton=nullptr;
  HFONT titleFont=nullptr,detailFont=nullptr,controlFont=nullptr,chatFont=nullptr;
  HBRUSH logBrush=nullptr,inputBrush=nullptr;
  bool expanded=false,voiceEnabled=true,chatEnabled=false,hasSnapshot=false;
  std::deque<std::wstring> messages;
  unsigned unread=0,shownUnread=0;
  mutable std::wstring subtitleText;
  float scale=1;
  static constexpr int voiceHeight=112,chatHeight=168;
  int px(double value) const { return static_cast<int>(std::lround(value*scale)); }
  int width() const { return stop ? 352 : 312; }
  int height() const { return expanded ? 52+(voiceEnabled ? voiceHeight : 0)+(chatEnabled ? chatHeight : 0) : 52; }
  int chatTop() const { return 52+(voiceEnabled ? voiceHeight : 0); }
  // Enter in the message box sends; the box never inserts a new line.
  static LRESULT CALLBACK inputProcedure(HWND input,UINT message,WPARAM w,LPARAM l,UINT_PTR,DWORD_PTR data) {
    auto* self=reinterpret_cast<Impl*>(data);
    if(message==WM_KEYDOWN && w==VK_RETURN) { self->sendChat(); return 0; }
    if(message==WM_CHAR && (w==L'\r' || w==L'\n')) return 0;
    return DefSubclassProc(input,message,w,l);
  }
  static LRESULT CALLBACK procedure(HWND window,UINT message,WPARAM w,LPARAM l) {
    auto* self=reinterpret_cast<Impl*>(GetWindowLongPtrW(window,GWLP_USERDATA));
    if(message==WM_NCCREATE) { self=static_cast<Impl*>(reinterpret_cast<CREATESTRUCTW*>(l)->lpCreateParams); SetWindowLongPtrW(window,GWLP_USERDATA,reinterpret_cast<LONG_PTR>(self)); }
    if(self && message==WM_TIMER) { self->refresh(); return 0; }
    if(self && message==WM_CLOSE) { self->write({false,false,false}); self->expanded=false; self->layout(); self->refresh(true); return 0; }
    if(self && message==WM_COMMAND && HIWORD(w)==BN_CLICKED) { self->activate(LOWORD(w)); return 0; }
    if(self && message==WM_DRAWITEM) { self->drawButton(*reinterpret_cast<DRAWITEMSTRUCT*>(l)); return TRUE; }
    if(self && message==WM_PAINT) { self->paint(); return 0; }
    if(message==WM_ERASEBKGND) return 1;
    if(self && (message==WM_CTLCOLOREDIT || message==WM_CTLCOLORSTATIC) && (reinterpret_cast<HWND>(l)==self->chatLog || reinterpret_cast<HWND>(l)==self->chatInput)) {
      const auto dc=reinterpret_cast<HDC>(w); const bool input=reinterpret_cast<HWND>(l)==self->chatInput;
      SetTextColor(dc,ink); SetBkColor(dc,input ? surface : panelColor(design::surface));
      return reinterpret_cast<LRESULT>(input ? self->inputBrush : self->logBrush);
    }
    // The panel never takes focus, except to type in the open chat box.
    if(message==WM_MOUSEACTIVATE) {
      if(self && self->expanded && self->chatEnabled) {
        POINT cursor{}; GetCursorPos(&cursor); ScreenToClient(window,&cursor);
        const auto child=ChildWindowFromPointEx(window,cursor,CWP_SKIPINVISIBLE);
        if(child==self->chatInput || child==self->chatLog) return MA_ACTIVATE;
      }
      return MA_NOACTIVATE;
    }
    // Empty space is a drag handle. Native child buttons keep their own hit tests.
    if(self && message==WM_NCHITTEST) {
      const auto hit=DefWindowProcW(window,message,w,l); return hit==HTCLIENT ? HTCAPTION : hit;
    }
    return DefWindowProcW(window,message,w,l);
  }
  Impl(bool enabled,VoiceRuntime& value,std::function<void()> stopSharing,bool chat):Impl([&value] { return value.snapshot(); },[&value](voice::State state) { value.configure(state); },enabled,std::move(stopSharing),
    chat ? std::function<std::vector<std::string>()>([] { return takePanelChatCommands(); }) : std::function<std::vector<std::string>()>{},
    chat ? std::function<void(std::string)>([](std::string text) { panelChatSend(text); }) : std::function<void(std::string)>{}) {}
  Impl(std::function<VoiceRuntime::Status()> readStatus,std::function<void(voice::State)> configure,bool enabled=true,std::function<void()> stopSharing={},
       std::function<std::vector<std::string>()> lines={},std::function<void(std::string)> send={})
    :read(std::move(readStatus)),write(std::move(configure)),stop(std::move(stopSharing)),chatLines(std::move(lines)),chatSend(std::move(send)),voiceEnabled(enabled) {
    chatEnabled=chatLines && chatSend;
    const HDC dc=GetDC(nullptr); scale=GetDeviceCaps(dc,LOGPIXELSX)/96.f; ReleaseDC(nullptr,dc);
    titleFont=CreateFontW(-px(12),0,0,0,FW_SEMIBOLD,FALSE,FALSE,FALSE,DEFAULT_CHARSET,OUT_DEFAULT_PRECIS,CLIP_DEFAULT_PRECIS,CLEARTYPE_QUALITY,DEFAULT_PITCH,L"Segoe UI");
    controlFont=CreateFontW(-px(11),0,0,0,FW_SEMIBOLD,FALSE,FALSE,FALSE,DEFAULT_CHARSET,OUT_DEFAULT_PRECIS,CLIP_DEFAULT_PRECIS,CLEARTYPE_QUALITY,DEFAULT_PITCH,L"Segoe UI");
    detailFont=CreateFontW(-px(10),0,0,0,FW_NORMAL,FALSE,FALSE,FALSE,DEFAULT_CHARSET,OUT_DEFAULT_PRECIS,CLIP_DEFAULT_PRECIS,CLEARTYPE_QUALITY,DEFAULT_PITCH,L"Segoe UI");
    chatFont=CreateFontW(-px(12),0,0,0,FW_NORMAL,FALSE,FALSE,FALSE,DEFAULT_CHARSET,OUT_DEFAULT_PRECIS,CLIP_DEFAULT_PRECIS,CLEARTYPE_QUALITY,DEFAULT_PITCH,L"Segoe UI");
    logBrush=CreateSolidBrush(panelColor(design::surface)); inputBrush=CreateSolidBrush(surface);
    WNDCLASSW klass{}; klass.lpfnWndProc=procedure; klass.hInstance=GetModuleHandleW(nullptr); klass.style=CS_DROPSHADOW;
    klass.lpszClassName=L"SanserGroupVoicePanel"; klass.hCursor=LoadCursorW(nullptr,MAKEINTRESOURCEW(32512)); RegisterClassW(&klass);
    RECT area{}; SystemParametersInfoW(SPI_GETWORKAREA,0,&area,0);
    window=CreateWindowExW(WS_EX_TOPMOST|WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE,klass.lpszClassName,stop ? L"Sanser screen sharing and voice" : L"Sanser group voice",WS_POPUP|WS_CLIPCHILDREN,area.right-px(width()+20),area.top+px(12),px(width()),px(52),nullptr,nullptr,klass.hInstance,this);
    if(!window) throw std::runtime_error("Unable to create floating sharing/voice controls");
    INITCOMMONCONTROLSEX controls{sizeof(INITCOMMONCONTROLSEX),ICC_WIN95_CLASSES}; InitCommonControlsEx(&controls);
    tooltip=CreateWindowExW(WS_EX_TOPMOST,TOOLTIPS_CLASSW,nullptr,WS_POPUP|TTS_ALWAYSTIP|TTS_NOPREFIX,CW_USEDEFAULT,CW_USEDEFAULT,CW_USEDEFAULT,CW_USEDEFAULT,window,nullptr,klass.hInstance,nullptr);
    SendMessageW(tooltip,TTM_SETMAXTIPWIDTH,0,px(280));
    const auto button=[&](const wchar_t* title,int id) {
      auto result=CreateWindowW(L"BUTTON",title,WS_CHILD|WS_VISIBLE|BS_OWNERDRAW,0,0,0,0,window,reinterpret_cast<HMENU>(static_cast<INT_PTR>(id)),klass.hInstance,nullptr);
      // We paint the entire button, including its focus/hover treatment. Keep
      // native accessibility and click behavior without themed white borders.
      SetWindowTheme(result,L"",L"");
      SendMessageW(result,WM_UPDATEUISTATE,MAKEWPARAM(UIS_SET,UISF_HIDEFOCUS),0);
      TOOLINFOW info{}; info.cbSize=sizeof(info); info.uFlags=TTF_IDISHWND|TTF_SUBCLASS; info.hwnd=window; info.uId=reinterpret_cast<UINT_PTR>(result); info.lpszText=const_cast<wchar_t*>(title);
      SendMessageW(tooltip,TTM_ADDTOOLW,0,reinterpret_cast<LPARAM>(&info)); return result;
    };
    toggle=button(L"Expand voice controls",10); shortcut=button(L"Unmute microphone",12); chevron=button(L"Expand voice controls",16); stopButton=button(L"Stop screen sharing",15);
    join=button(L"Join voice · listen only",11); microphone=button(L"Unmute microphone",12); deafen=button(L"Deafen voice and mute microphone",13); leave=button(L"Leave voice",14);
    if(chatEnabled) {
      chatLog=CreateWindowExW(0,L"EDIT",L"",WS_CHILD|WS_VSCROLL|ES_MULTILINE|ES_READONLY|ES_AUTOVSCROLL,0,0,0,0,window,reinterpret_cast<HMENU>(static_cast<INT_PTR>(18)),klass.hInstance,nullptr);
      chatInput=CreateWindowExW(0,L"EDIT",L"",WS_CHILD|WS_TABSTOP|ES_AUTOHSCROLL,0,0,0,0,window,reinterpret_cast<HMENU>(static_cast<INT_PTR>(19)),klass.hInstance,nullptr);
      for(auto edit:{chatLog,chatInput}) SendMessageW(edit,WM_SETFONT,reinterpret_cast<WPARAM>(chatFont),TRUE);
      SendMessageW(chatInput,EM_SETCUEBANNER,TRUE,reinterpret_cast<LPARAM>(L"Message everyone…"));
      SendMessageW(chatInput,EM_SETLIMITTEXT,1024,0);
      SetWindowSubclass(chatInput,inputProcedure,1,reinterpret_cast<DWORD_PTR>(this));
      chatButton=button(L"Send message",17);
      showChat();
    }
    SetTimer(window,1,100,nullptr); layout(); refresh(); ShowWindow(window,SW_SHOWNOACTIVATE);
  }
  void label(HWND button,const wchar_t* text) {
    SetWindowTextW(button,text); TOOLINFOW info{}; info.cbSize=sizeof(info); info.hwnd=window; info.uId=reinterpret_cast<UINT_PTR>(button); info.lpszText=const_cast<wchar_t*>(text);
    SendMessageW(tooltip,TTM_UPDATETIPTEXTW,0,reinterpret_cast<LPARAM>(&info));
  }
  void layout() {
    SetWindowPos(window,HWND_TOPMOST,0,0,px(width()),px(height()),SWP_NOMOVE|SWP_NOACTIVATE);
    SetWindowRgn(window,CreateRoundRectRgn(0,0,px(width())+1,px(height())+1,px(design::panelRadius*2),px(design::panelRadius*2)),TRUE);
    const auto place=[&](HWND child,int x,int y,int w,int h) { MoveWindow(child,px(x),px(y),px(w),px(h),TRUE); };
    place(toggle,12,10,164,32); place(shortcut,width()-(stop ? 164 : 132),12,78,28);
    place(chevron,width()-(stop ? 78 : 42),12,28,28); place(stopButton,width()-42,12,28,28);
    const auto actionWidth=(width()-92)/2;
    place(join,16,88,width()-32,36); place(microphone,16,88,actionWidth,36);
    place(deafen,24+actionWidth,88,actionWidth,36); place(leave,width()-52,88,36,36);
    if(chatEnabled) {
      place(chatLog,12,chatTop()+30,width()-24,chatHeight-78);
      place(chatInput,16,chatTop()+chatHeight-40,width()-32-66,26);
      place(chatButton,width()-76,chatTop()+chatHeight-41,60,28);
    }
    showActions();
    ShowWindow(stopButton,stop ? SW_SHOWNOACTIVATE : SW_HIDE);
    ShowWindow(shortcut,voiceEnabled ? SW_SHOWNOACTIVATE : SW_HIDE); ShowWindow(chevron,voiceEnabled || chatEnabled ? SW_SHOWNOACTIVATE : SW_HIDE);
    // Keep the sharing indicator legible, including engines without voice.
    EnableWindow(toggle,voiceEnabled || chatEnabled); InvalidateRect(window,nullptr,FALSE);
  }
  void showActions() const {
    const bool voice=expanded && voiceEnabled;
    ShowWindow(join,voice && !current.state.joined ? SW_SHOWNOACTIVATE : SW_HIDE);
    for(auto child:{microphone,deafen,leave}) ShowWindow(child,voice && current.state.joined ? SW_SHOWNOACTIVATE : SW_HIDE);
    if(chatEnabled) for(auto child:{chatLog,chatInput,chatButton}) ShowWindow(child,expanded ? SW_SHOWNOACTIVATE : SW_HIDE);
  }
  void showChat() {
    std::wstring text;
    if(messages.empty()) text=L"No messages yet. Chat is kept only for this session.";
    for(const auto& line:messages) { if(!text.empty()) text+=L"\r\n"; text+=line; }
    SetWindowTextW(chatLog,text.c_str());
    const auto end=GetWindowTextLengthW(chatLog);
    SendMessageW(chatLog,EM_SETSEL,end,end); SendMessageW(chatLog,EM_SCROLLCARET,0,0);
  }
  bool receiveChat() {
    if(!chatLines) return false;
    bool changed=false;
    for(const auto& raw:chatLines()) {
      const auto line=parsePanelChat(raw); if(!line) continue;
      auto name=line->own ? std::wstring(L"You") : wide(line->name); if(name.empty()) name=L"Viewer";
      auto body=wide(line->text); for(auto& c:body) if(c==L'\n') c=L' ';
      messages.push_back(name+L": "+body); while(messages.size()>50) messages.pop_front();
      if(!line->own && !expanded) ++unread;
      changed=true;
    }
    if(changed) showChat();
    return changed;
  }
  void sendChat() {
    if(!chatInput || !chatSend) return;
    const auto length=GetWindowTextLengthW(chatInput); if(length<=0) return;
    std::wstring value(static_cast<std::size_t>(length)+1,L'\0'); GetWindowTextW(chatInput,value.data(),length+1); value.resize(static_cast<std::size_t>(length));
    const auto first=value.find_first_not_of(L" \t"),last=value.find_last_not_of(L" \t");
    if(first==std::wstring::npos) return;
    const auto text=narrow(value.substr(first,last-first+1));
    if(text.empty() || text.size()>panelChatMaxText) { MessageBeep(MB_ICONWARNING); return; }
    chatSend(text); SetWindowTextW(chatInput,L"");
  }
  void activate(unsigned id) {
    if(id==15) { if(stop) { write({false,false,false}); stop(); } return; }
    if(id==17) { sendChat(); return; }
    if(id==10 || id==16) {
      if(!voiceEnabled && !chatEnabled) return;
      expanded=!expanded; if(expanded) unread=0; layout(); refresh(true); return;
    }
    if(!voiceEnabled) return;
    auto next=read().state;
    if(id==11 && !next.joined) next={true,false,false};
    if(id==12) { if(!next.joined) next={true,false,false}; else { next.microphone=!next.microphone; next.deafened=false; } }
    if(id==13 && next.joined) { next.deafened=!next.deafened; next.microphone=false; }
    if(id==14) next={false,false,false};
    write(next); refresh();
  }
  const wchar_t* subtitle() const {
    if(unread) { subtitleText=unread==1 ? std::wstring(L"1 new message") : std::to_wstring(unread)+L" new messages"; return subtitleText.c_str(); }
    return !voiceEnabled ? (stop ? L"Sharing is active" : L"Messages for this session") : current.capturing ? L"Microphone on" : current.state.microphone ? L"Opening microphone…" : current.state.deafened ? L"Voice muted · mic off" : current.state.joined ? L"Listening · mic off" : L"Voice is off";
  }
  const wchar_t* status() const {
    const auto& state=current.state;
    return current.code=="permission-denied" ? L"Allow microphone in Windows privacy settings." : current.code=="no-device" ? L"No microphone available." : current.code=="unavailable" ? L"Microphone unavailable. Unmute to retry." : current.capturing ? L"Your microphone is on." : state.microphone ? L"Opening your microphone…" : state.deafened ? L"Voice muted. Your microphone is off." : state.joined ? L"You’re listening. Your microphone is off." : L"Join to listen. Your mic stays off.";
  }
  void refresh(bool force=false) {
    // A static sharing bar should not continually dirty captured frames.
    POINT cursor{}; GetCursorPos(&cursor); const auto underCursor=WindowFromPoint(cursor);
    const auto nextHover=GetParent(underCursor)==window ? underCursor : nullptr;
    if(nextHover!=hovered) {
      if(hovered) InvalidateRect(hovered,nullptr,FALSE);
      if(nextHover) InvalidateRect(nextHover,nullptr,FALSE);
      hovered=nextHover;
    }
    if(receiveChat() || unread!=shownUnread) { force=true; shownUnread=unread; }
    const auto value=read(); const auto& state=value.state;
    if(!force && hasSnapshot && current.state.joined==state.joined && current.state.microphone==state.microphone && current.state.deafened==state.deafened && current.capturing==value.capturing && current.code==value.code) return;
    current=value; hasSnapshot=true;
    showActions();
    EnableWindow(shortcut,TRUE); EnableWindow(join,TRUE); EnableWindow(microphone,state.joined); EnableWindow(deafen,state.joined); EnableWindow(leave,state.joined);
    label(shortcut,!state.joined ? L"Join voice · listen only" : state.microphone ? L"Mute microphone" : L"Unmute microphone"); label(microphone,state.microphone ? L"Mute microphone" : L"Unmute microphone");
    label(deafen,state.deafened ? L"Hear voice" : L"Deafen voice and mute microphone");
    label(toggle,stop ? (expanded ? L"Screen sharing active · collapse voice and chat" : L"Screen sharing active · expand voice and chat") : (expanded ? L"Collapse voice and chat" : L"Expand voice and chat"));
    label(chevron,expanded ? L"Collapse voice and chat" : L"Expand voice and chat");
    InvalidateRect(window,nullptr,FALSE); for(auto child:{toggle,shortcut,chevron,stopButton,join,microphone,deafen,leave}) InvalidateRect(child,nullptr,FALSE);
    if(chatButton) InvalidateRect(chatButton,nullptr,FALSE);
  }
  void backdrop(HDC dc,RECT rect,int offsetY=0) const {
    Gdiplus::Graphics graphics(dc);
    Gdiplus::LinearGradientBrush gradient(Gdiplus::Point(0,-offsetY),Gdiplus::Point(0,px(height())-offsetY),
      tint(panelColor(design::surface)),tint(panelColor(design::surface)));
    graphics.FillRectangle(&gradient,pixels(rect));
  }
  void fill(HDC dc,RECT rect,COLORREF color,int diameter=0) const {
    Gdiplus::Graphics graphics(dc); graphics.SetSmoothingMode(Gdiplus::SmoothingModeAntiAlias);
    Gdiplus::SolidBrush brush(tint(color));
    if(diameter) {
      Gdiplus::GraphicsPath path;
      roundedPath(path,static_cast<float>(rect.left),static_cast<float>(rect.top),static_cast<float>(rect.right),static_cast<float>(rect.bottom),static_cast<float>(px(diameter)));
      graphics.FillPath(&brush,&path);
    } else graphics.FillRectangle(&brush,pixels(rect));
  }
  void text(HDC dc,const wchar_t* value,RECT rect,HFONT font,COLORREF color,UINT flags) const {
    const auto old=SelectObject(dc,font); SetBkMode(dc,TRANSPARENT); SetTextColor(dc,color); DrawTextW(dc,value,-1,&rect,flags); SelectObject(dc,old);
  }
  void icon(HDC dc,int x,int y,unsigned id,COLORREF color) const {
    Gdiplus::Graphics graphics(dc); graphics.SetSmoothingMode(Gdiplus::SmoothingModeAntiAlias);
    graphics.TranslateTransform(static_cast<float>(x),static_cast<float>(y)); graphics.ScaleTransform(scale,scale);
    Gdiplus::Pen pen(tint(color),1.65f);
    pen.SetStartCap(Gdiplus::LineCapRound); pen.SetEndCap(Gdiplus::LineCapRound); pen.SetLineJoin(Gdiplus::LineJoinRound);
    const auto line=[&](float ax,float ay,float bx,float by) { graphics.DrawLine(&pen,ax,ay,bx,by); };
    const auto box=[&](float a,float b,float c,float d) {
      Gdiplus::GraphicsPath path; roundedPath(path,a,b,c,d,4); graphics.DrawPath(&pen,&path);
    };
    if(id==15) {
      Gdiplus::GraphicsPath path; roundedPath(path,4,4,16,16,4);
      Gdiplus::SolidBrush brush(tint(color)); graphics.FillPath(&brush,&path);
    } else if(id==17) {
      // Paper plane.
      line(2,9,18,2); line(18,2,11,18); line(11,18,9,11); line(9,11,2,9); line(9,11,18,2);
    } else if(id==16) {
      const float a=expanded ? 12.f : 8.f,b=expanded ? 8.f : 12.f; line(5,a,10,b); line(10,b,15,a);
    } else if(id==12) {
      Gdiplus::GraphicsPath body; roundedPath(body,7,1.5f,13,12.5f,6);
      if(current.state.microphone) { Gdiplus::SolidBrush brush(tint(color)); graphics.FillPath(&brush,&body); }
      else graphics.DrawPath(&pen,&body);
      graphics.DrawArc(&pen,4.f,4.f,12.f,12.f,0.f,180.f);
      line(10,16,10,19); line(6.5f,19,13.5f,19);
      if(!current.state.microphone) line(2,2,18,18);
    } else if(id==13 || id==11) {
      graphics.DrawArc(&pen,3.f,2.f,14.f,16.f,180.f,180.f);
      box(2,10,6,17); box(14,10,18,17);
      if(id==11) { line(8,13,12,13); line(10,11,10,15); }
      if(id==13 && current.state.deafened) line(2,2,18,18);
    } else if(id==14) {
      // Match the Mac phone-down action instead of a desktop exit glyph.
      Gdiplus::GraphicsPath handset;
      handset.AddArc(2.f,7.f,16.f,12.f,205.f,130.f);
      graphics.DrawPath(&pen,&handset); line(2.7f,9.5f,2,13); line(2,13,6,14); line(6,14,7,10);
      line(17.3f,9.5f,18,13); line(18,13,14,14); line(14,14,13,10);
    } else if(stop) { box(2,3,18,14); line(10,14,10,18); line(6,18,14,18); }
    else { line(3,8,3,12); line(7,4,7,16); line(11,1,11,19); line(15,5,15,15); line(19,8,19,12); }
  }
  void drawButton(const DRAWITEMSTRUCT& item) const {
    const unsigned id=item.CtlID; PaintBuffer buffer(item.hDC,item.rcItem); auto dc=buffer.dc; const bool header=id==10;
    const bool enabled=!(item.itemState&ODS_DISABLED);
    const bool quick=item.hwndItem==shortcut;
    COLORREF color=id==15 || id==14 ? red : id==17 || id==11 || (quick && !current.state.joined) || (header && (stop || current.state.joined)) ? accent : id==12 && current.capturing ? accent : id==13 && current.state.deafened ? red : ink;
    if(!enabled && !header) color=RGB(76,85,102);
    POINT cursor{}; GetCursorPos(&cursor); RECT bounds{}; GetWindowRect(item.hwndItem,&bounds);
    const bool hover=enabled && PtInRect(&bounds,cursor);
    const bool active=id==17 || id==11 || (id==12 && current.state.microphone) || (id==13 && current.state.deafened);
    RECT position{}; GetWindowRect(item.hwndItem,&position); MapWindowPoints(nullptr,window,reinterpret_cast<POINT*>(&position),2);
    backdrop(dc,item.rcItem,position.top);
    if(!header && (id!=16 || hover))
      fill(dc,item.rcItem,(item.itemState&ODS_SELECTED) ? RGB(48,62,82) : hover ? RGB(37,49,66) : id==15 ? RGB(46,29,39) : active ? RGB(22,49,46) : surface,16);
    if(header) {
      RECT tile{0,px(2),px(28),px(30)}; fill(dc,tile,RGB(20,49,45),16); icon(dc,px(4),px(6),10,color);
      RECT title{px(36),-px(1),px(164),px(17)},detail{px(36),px(18),px(164),px(32)};
      text(dc,stop ? L"Screen sharing" : voiceEnabled ? L"Voice chat" : L"Chat",title,titleFont,ink,DT_LEFT|DT_SINGLELINE);
      text(dc,subtitle(),detail,detailFont,current.capturing || unread ? accent : muted,DT_LEFT|DT_SINGLELINE);
    } else {
      const wchar_t* caption=id==17 ? L"Send" : quick ? (!current.state.joined ? L"Join" : current.capturing ? L"Mic on" : current.state.microphone ? L"…" : L"Muted") : id==11 ? L"Join voice" : id==12 ? (current.state.microphone ? L"Mute mic" : L"Unmute") : id==13 ? (current.state.deafened ? L"Hear voice" : L"Mute voice") : nullptr;
      const auto symbol=quick && !current.state.joined ? 11u : id;
      const auto top=(item.rcItem.bottom-px(20))/2;
      if(caption) {
        const auto font=SelectObject(dc,controlFont); SIZE size{}; GetTextExtentPoint32W(dc,caption,static_cast<int>(wcslen(caption)),&size); SelectObject(dc,font);
        const auto left=(item.rcItem.right-size.cx-px(28))/2;
        icon(dc,left,top,symbol,color);
        RECT label{left+px(28),0,item.rcItem.right,item.rcItem.bottom}; text(dc,caption,label,controlFont,color,DT_LEFT|DT_VCENTER|DT_SINGLELINE);
      } else icon(dc,(item.rcItem.right-px(20))/2,top,symbol,color);
    }
  }
  void drawSurface(HDC dc,RECT rect) const {
    backdrop(dc,rect);
    {
      Gdiplus::Graphics graphics(dc); graphics.SetSmoothingMode(Gdiplus::SmoothingModeAntiAlias);
      Gdiplus::Pen border(tint(edge),1.f);
      Gdiplus::GraphicsPath shape;
      roundedPath(shape,0.5f,0.5f,rect.right-0.5f,rect.bottom-0.5f,static_cast<float>(px(design::panelRadius*2)));
      graphics.DrawPath(&border,&shape);
      if(expanded) {
        Gdiplus::Pen separator(Gdiplus::Color(20,255,255,255),1.f);
        graphics.DrawLine(&separator,px(14),px(52),px(width()-14),px(52));
        if(chatEnabled && voiceEnabled) graphics.DrawLine(&separator,px(14),px(chatTop()),px(width()-14),px(chatTop()));
      }
    }
    if(expanded && chatEnabled) {
      RECT section{px(16),px(chatTop()+11),px(166),px(chatTop()+27)}; text(dc,L"CHAT",section,detailFont,muted,DT_LEFT|DT_VCENTER|DT_SINGLELINE);
    }
    if(expanded && voiceEnabled) {
      RECT section{px(16),px(63),px(166),px(79)}; text(dc,L"VOICE CHAT",section,detailFont,muted,DT_LEFT|DT_VCENTER|DT_SINGLELINE);
      RECT badge{px(width()-88),px(63),px(width()-16),px(79)}; fill(dc,badge,surface,8);
      text(dc,current.capturing ? L"MIC LIVE" : current.state.microphone ? L"OPENING" : current.state.joined ? L"MIC OFF" : L"NOT JOINED",badge,detailFont,current.capturing ? accent : muted,DT_CENTER|DT_VCENTER|DT_SINGLELINE);
      RECT label{px(16),px(132),px(width()-16),px(156)}; text(dc,status(),label,detailFont,muted,DT_LEFT|DT_WORDBREAK);
    }
  }
  void paint() const {
    PAINTSTRUCT paint{}; auto target=BeginPaint(window,&paint); RECT rect{}; GetClientRect(window,&rect);
    { PaintBuffer buffer(target,rect); drawSurface(buffer.dc,rect); }
    EndPaint(window,&paint);
  }
#ifdef SANSER_CODEC_TEST
  // Render only this fake-controller surface into an image. No desktop, mic,
  // other window or remote session is captured by the CI preview export.
  void preview(const char* name) const {
    const auto directory=std::getenv("SANSER_PANEL_PREVIEW_DIR"); if(!directory) return;
    Gdiplus::Bitmap image(px(width()),px(height()),PixelFormat32bppRGB);
    {
      Gdiplus::Graphics graphics(&image); auto dc=graphics.GetHDC();
      RECT rect{0,0,px(width()),px(height())}; drawSurface(dc,rect);
      for(auto child:{toggle,shortcut,chevron,stopButton,join,microphone,deafen,leave,chatButton}) {
        if(!child || !IsWindowVisible(child)) continue;
        RECT position{}; GetWindowRect(child,&position); MapWindowPoints(nullptr,window,reinterpret_cast<POINT*>(&position),2);
        const auto saved=SaveDC(dc); SetViewportOrgEx(dc,position.left,position.top,nullptr);
        DRAWITEMSTRUCT item{}; item.CtlID=GetDlgCtrlID(child); item.hwndItem=child; item.hDC=dc;
        item.rcItem={0,0,position.right-position.left,position.bottom-position.top}; drawButton(item);
        RestoreDC(dc,saved);
      }
      graphics.ReleaseHDC(dc);
    }
    UINT count=0,bytes=0; Gdiplus::GetImageEncodersSize(&count,&bytes);
    std::vector<BYTE> storage(bytes); auto* encoders=reinterpret_cast<Gdiplus::ImageCodecInfo*>(storage.data());
    if(!bytes || Gdiplus::GetImageEncoders(count,bytes,encoders)!=Gdiplus::Ok) throw std::runtime_error("Cannot read panel preview encoders");
    for(UINT i=0;i<count;++i) if(!wcscmp(encoders[i].MimeType,L"image/png")) {
      if(image.Save((std::filesystem::path(directory)/name).c_str(),&encoders[i].Clsid,nullptr)!=Gdiplus::Ok) throw std::runtime_error("Cannot save panel preview");
      return;
    }
    throw std::runtime_error("PNG panel preview encoder unavailable");
  }
#endif
  ~Impl() {
    if(chatInput) RemoveWindowSubclass(chatInput,inputProcedure,1);
    if(window) { KillTimer(window,1); if(tooltip) DestroyWindow(tooltip); DestroyWindow(window); }
    for(auto font:{titleFont,detailFont,controlFont,chatFont}) if(font) DeleteObject(font);
    for(auto brush:{logBrush,inputBrush}) if(brush) DeleteObject(brush);
  }
};
VoicePanel::VoicePanel(bool enabled,VoiceRuntime& voice,std::function<void()> stopSharing,bool chat) {
  if(enabled || stopSharing || chat) impl_=std::make_unique<Impl>(enabled,voice,std::move(stopSharing),chat);
}
VoicePanel::~VoicePanel()=default;
void VoicePanel::pump() {
  if(!impl_) return;
  if(externalVoiceActive()) {if(IsWindowVisible(impl_->window)) ShowWindow(impl_->window,SW_HIDE);}
  else if(!IsWindowVisible(impl_->window)) ShowWindow(impl_->window,SW_SHOWNOACTIVATE);
  MSG message; for(unsigned count=0;count<64 && PeekMessageW(&message,nullptr,0,0,PM_REMOVE);++count) { TranslateMessage(&message); DispatchMessageW(&message); }
}
#ifdef SANSER_CODEC_TEST
void testVoicePanelControls() {
  VoiceRuntime::Status status; unsigned changes=0,stops=0;
  const auto read=[&] { return status; }; const auto write=[&](voice::State value) { status.state=value; ++changes; };
  VoicePanel::Impl panel(read,write);
  panel.preview("viewer-compact.png");
  if(!panel.window || changes || IsWindowVisible(panel.stopButton) || !(GetWindowLongPtrW(panel.window,GWL_EXSTYLE)&WS_EX_NOACTIVATE)) throw std::runtime_error("Viewer must start voice-only, without capture or stealing activation");
  const auto click=[](VoicePanel::Impl& target,unsigned id) { SendMessageW(target.window,WM_COMMAND,MAKEWPARAM(id,BN_CLICKED),0); };
  click(panel,10); if(!panel.expanded || changes) throw std::runtime_error("Opening voice controls must not join or capture");
  click(panel,11); if(!status.state.joined || status.state.microphone) throw std::runtime_error("Floating Join must listen only");
  click(panel,12); if(!status.state.microphone) throw std::runtime_error("Floating Unmute did not request the microphone");
  status.capturing=true; panel.refresh(); panel.preview("viewer-expanded-mic-on.png");
  panel.expanded=false; panel.layout(); panel.preview("viewer-compact-mic-on.png");
  panel.expanded=true; panel.layout(); status.capturing=false; panel.refresh();
  const auto beforeCollapse=changes; click(panel,10); click(panel,16);
  if(changes!=beforeCollapse || !status.state.microphone) throw std::runtime_error("Collapsing floating controls must not leave the call");
  click(panel,12); if(status.state.microphone) throw std::runtime_error("Compact Mute did not revoke microphone consent");
  click(panel,12); click(panel,13); if(!status.state.deafened || status.state.microphone) throw std::runtime_error("Floating Deafen must mute capture");
  click(panel,14); if(status.state.joined || status.state.microphone) throw std::runtime_error("Floating Leave must revoke microphone consent");
  click(panel,10); click(panel,12);
  if(panel.expanded || !status.state.joined || status.state.microphone) throw std::runtime_error("Compact Join must listen only without opening capture");
  click(panel,16);
  if(IsWindowVisible(panel.join) || !IsWindowVisible(panel.microphone)) throw std::runtime_error("Joined voice must show mic/sound actions instead of Join");
  click(panel,14);
  if(!IsWindowVisible(panel.join) || IsWindowVisible(panel.microphone)) throw std::runtime_error("Leaving voice must restore the Join action");
  VoicePanel::Impl host(read,write,true,[&] { ++stops; });
  host.preview("host-compact.png");
  if(!IsWindowVisible(host.stopButton)) throw std::runtime_error("Sharing host needs one combined screen/voice panel");
  click(host,11); click(host,12); status.capturing=true; host.refresh(); host.preview("host-compact-mic-on.png");
  host.expanded=true; host.layout(); host.preview("host-expanded-mic-on.png");
  click(host,15); status.capturing=false;
  if(stops!=1 || status.state.joined || status.state.microphone) throw std::runtime_error("Stop sharing must also leave voice");
  VoicePanel::Impl screenOnly(read,write,false,[&] { ++stops; });
  if(!IsWindowVisible(screenOnly.stopButton) || IsWindowVisible(screenOnly.shortcut) || IsWindowVisible(screenOnly.chevron)) throw std::runtime_error("Host without voice must retain sharing indicator and stop");
  click(screenOnly,15); if(stops!=2) throw std::runtime_error("Screen-only Stop did not stop sharing");
  std::vector<std::string> incoming,sent;
  VoicePanel::Impl chatting(read,write,true,[&] { ++stops; },[&] { auto lines=std::move(incoming); incoming.clear(); return lines; },[&](std::string text) { sent.push_back(std::move(text)); });
  incoming.push_back("panel-chat\t0\t"+userHex("iPhone")+"\t"+userHex("xin chào"));
  chatting.refresh();
  if(chatting.unread!=1 || chatting.messages.size()!=1 || std::wstring(chatting.subtitle())!=L"1 new message") throw std::runtime_error("Panel chat must count unread messages while collapsed");
  click(chatting,16);
  if(chatting.unread || !IsWindowVisible(chatting.chatInput) || !IsWindowVisible(chatting.chatLog)) throw std::runtime_error("Opening the panel must show chat and clear unread");
  SetWindowTextW(chatting.chatInput,L"  ok thấy rồi  "); click(chatting,17);
  if(sent.size()!=1 || sent[0]!="ok thấy rồi" || GetWindowTextLengthW(chatting.chatInput)) throw std::runtime_error("Send must post trimmed UTF-8 text and clear the box");
  chatting.preview("host-chat.png");
  if(chatting.height()!=52+112+168) throw std::runtime_error("Chat must sit below the voice controls");
}
#endif
}
