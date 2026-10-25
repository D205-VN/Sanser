#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <commctrl.h>
#include <objidl.h>
#include <gdiplus.h>
#include <uxtheme.h>
#include "voice_panel.h"
#include <algorithm>
#include <cmath>
#include <cwchar>
#include <stdexcept>
#include <vector>
#ifdef SANSER_CODEC_TEST
#include <cstdlib>
#include <filesystem>
#endif
namespace sanser::desktop {
namespace {
constexpr COLORREF surface=RGB(28,36,49),ink=RGB(225,232,247),muted=RGB(141,156,181),accent=RGB(102,219,184),red=RGB(255,125,138),edge=RGB(48,59,77);
Gdiplus::Color tint(COLORREF value) { return Gdiplus::Color(255,GetRValue(value),GetGValue(value),GetBValue(value)); }
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
  VoiceRuntime::Status current;
  HWND window=nullptr,toggle=nullptr,shortcut=nullptr,chevron=nullptr,stopButton=nullptr,join=nullptr,microphone=nullptr,deafen=nullptr,leave=nullptr,tooltip=nullptr,hovered=nullptr;
  HFONT titleFont=nullptr,detailFont=nullptr,controlFont=nullptr;
  bool expanded=false,voiceEnabled=true,hasSnapshot=false;
  float scale=1;
  int px(double value) const { return static_cast<int>(std::lround(value*scale)); }
  int width() const { return stop ? 352 : 312; }
  int height() const { return expanded ? 164 : 52; }
  static LRESULT CALLBACK procedure(HWND window,UINT message,WPARAM w,LPARAM l) {
    auto* self=reinterpret_cast<Impl*>(GetWindowLongPtrW(window,GWLP_USERDATA));
    if(message==WM_NCCREATE) { self=static_cast<Impl*>(reinterpret_cast<CREATESTRUCTW*>(l)->lpCreateParams); SetWindowLongPtrW(window,GWLP_USERDATA,reinterpret_cast<LONG_PTR>(self)); }
    if(self && message==WM_TIMER) { self->refresh(); return 0; }
    if(self && message==WM_CLOSE) { self->write({false,false,false}); self->expanded=false; self->layout(); self->refresh(true); return 0; }
    if(self && message==WM_COMMAND && HIWORD(w)==BN_CLICKED) { self->activate(LOWORD(w)); return 0; }
    if(self && message==WM_DRAWITEM) { self->drawButton(*reinterpret_cast<DRAWITEMSTRUCT*>(l)); return TRUE; }
    if(self && message==WM_PAINT) { self->paint(); return 0; }
    if(message==WM_ERASEBKGND) return 1;
    if(message==WM_MOUSEACTIVATE) return MA_NOACTIVATE;
    // Empty space is a drag handle. Native child buttons keep their own hit tests.
    if(self && message==WM_NCHITTEST) {
      const auto hit=DefWindowProcW(window,message,w,l); return hit==HTCLIENT ? HTCAPTION : hit;
    }
    return DefWindowProcW(window,message,w,l);
  }
  Impl(bool enabled,VoiceRuntime& value,std::function<void()> stopSharing):Impl([&value] { return value.snapshot(); },[&value](voice::State state) { value.configure(state); },enabled,std::move(stopSharing)) {}
  Impl(std::function<VoiceRuntime::Status()> readStatus,std::function<void(voice::State)> configure,bool enabled=true,std::function<void()> stopSharing={}):read(std::move(readStatus)),write(std::move(configure)),stop(std::move(stopSharing)),voiceEnabled(enabled) {
    const HDC dc=GetDC(nullptr); scale=GetDeviceCaps(dc,LOGPIXELSX)/96.f; ReleaseDC(nullptr,dc);
    titleFont=CreateFontW(-px(12),0,0,0,FW_SEMIBOLD,FALSE,FALSE,FALSE,DEFAULT_CHARSET,OUT_DEFAULT_PRECIS,CLIP_DEFAULT_PRECIS,CLEARTYPE_QUALITY,DEFAULT_PITCH,L"Segoe UI");
    controlFont=CreateFontW(-px(11),0,0,0,FW_SEMIBOLD,FALSE,FALSE,FALSE,DEFAULT_CHARSET,OUT_DEFAULT_PRECIS,CLIP_DEFAULT_PRECIS,CLEARTYPE_QUALITY,DEFAULT_PITCH,L"Segoe UI");
    detailFont=CreateFontW(-px(10),0,0,0,FW_NORMAL,FALSE,FALSE,FALSE,DEFAULT_CHARSET,OUT_DEFAULT_PRECIS,CLIP_DEFAULT_PRECIS,CLEARTYPE_QUALITY,DEFAULT_PITCH,L"Segoe UI");
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
    SetTimer(window,1,100,nullptr); layout(); refresh(); ShowWindow(window,SW_SHOWNOACTIVATE);
  }
  void label(HWND button,const wchar_t* text) {
    SetWindowTextW(button,text); TOOLINFOW info{}; info.cbSize=sizeof(info); info.hwnd=window; info.uId=reinterpret_cast<UINT_PTR>(button); info.lpszText=const_cast<wchar_t*>(text);
    SendMessageW(tooltip,TTM_UPDATETIPTEXTW,0,reinterpret_cast<LPARAM>(&info));
  }
  void layout() {
    SetWindowPos(window,HWND_TOPMOST,0,0,px(width()),px(height()),SWP_NOMOVE|SWP_NOACTIVATE);
    SetWindowRgn(window,CreateRoundRectRgn(0,0,px(width())+1,px(height())+1,px(30),px(30)),TRUE);
    const auto place=[&](HWND child,int x,int y,int w,int h) { MoveWindow(child,px(x),px(y),px(w),px(h),TRUE); };
    place(toggle,12,10,164,32); place(shortcut,width()-(stop ? 164 : 132),12,78,28);
    place(chevron,width()-(stop ? 78 : 42),12,28,28); place(stopButton,width()-42,12,28,28);
    const auto actionWidth=(width()-92)/2;
    place(join,16,88,width()-32,36); place(microphone,16,88,actionWidth,36);
    place(deafen,24+actionWidth,88,actionWidth,36); place(leave,width()-52,88,36,36);
    showActions();
    ShowWindow(stopButton,stop ? SW_SHOWNOACTIVATE : SW_HIDE);
    ShowWindow(shortcut,voiceEnabled ? SW_SHOWNOACTIVATE : SW_HIDE); ShowWindow(chevron,voiceEnabled ? SW_SHOWNOACTIVATE : SW_HIDE);
    // Keep the sharing indicator legible, including engines without voice.
    EnableWindow(toggle,voiceEnabled); InvalidateRect(window,nullptr,FALSE);
  }
  void showActions() const {
    ShowWindow(join,expanded && !current.state.joined ? SW_SHOWNOACTIVATE : SW_HIDE);
    for(auto child:{microphone,deafen,leave}) ShowWindow(child,expanded && current.state.joined ? SW_SHOWNOACTIVATE : SW_HIDE);
  }
  void activate(unsigned id) {
    if(id==15) { if(stop) { write({false,false,false}); stop(); } return; }
    if(!voiceEnabled) return;
    if(id==10 || id==16) { expanded=!expanded; layout(); refresh(true); return; }
    auto next=read().state;
    if(id==11 && !next.joined) next={true,false,false};
    if(id==12) { if(!next.joined) next={true,false,false}; else { next.microphone=!next.microphone; next.deafened=false; } }
    if(id==13 && next.joined) { next.deafened=!next.deafened; next.microphone=false; }
    if(id==14) next={false,false,false};
    write(next); refresh();
  }
  const wchar_t* subtitle() const {
    return !voiceEnabled ? L"Sharing is active" : current.state.joined ? L"Connected to voice" : L"Voice is off";
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
    const auto value=read(); const auto& state=value.state;
    if(!force && hasSnapshot && current.state.joined==state.joined && current.state.microphone==state.microphone && current.state.deafened==state.deafened && current.capturing==value.capturing && current.code==value.code) return;
    current=value; hasSnapshot=true;
    showActions();
    EnableWindow(shortcut,TRUE); EnableWindow(join,TRUE); EnableWindow(microphone,state.joined); EnableWindow(deafen,state.joined); EnableWindow(leave,state.joined);
    label(shortcut,!state.joined ? L"Join voice · listen only" : state.microphone ? L"Mute microphone" : L"Unmute microphone"); label(microphone,state.microphone ? L"Mute microphone" : L"Unmute microphone");
    label(deafen,state.deafened ? L"Hear voice" : L"Deafen voice and mute microphone");
    label(toggle,stop ? (expanded ? L"Screen sharing active · collapse voice controls" : L"Screen sharing active · expand voice controls") : (expanded ? L"Collapse voice controls" : L"Expand voice controls"));
    label(chevron,expanded ? L"Collapse voice controls" : L"Expand voice controls");
    InvalidateRect(window,nullptr,FALSE); for(auto child:{toggle,shortcut,chevron,stopButton,join,microphone,deafen,leave}) InvalidateRect(child,nullptr,FALSE);
  }
  void backdrop(HDC dc,RECT rect,int offsetY=0) const {
    Gdiplus::Graphics graphics(dc);
    Gdiplus::LinearGradientBrush gradient(Gdiplus::Point(0,-offsetY),Gdiplus::Point(0,px(height())-offsetY),
      Gdiplus::Color(255,14,18,27),Gdiplus::Color(255,22,28,38));
    graphics.FillRectangle(&gradient,rect.left,rect.top,rect.right-rect.left,rect.bottom-rect.top);
  }
  void fill(HDC dc,RECT rect,COLORREF color,int diameter=0) const {
    Gdiplus::Graphics graphics(dc); graphics.SetSmoothingMode(Gdiplus::SmoothingModeAntiAlias);
    Gdiplus::SolidBrush brush(tint(color));
    if(diameter) {
      Gdiplus::GraphicsPath path;
      roundedPath(path,static_cast<float>(rect.left),static_cast<float>(rect.top),static_cast<float>(rect.right),static_cast<float>(rect.bottom),static_cast<float>(px(diameter)));
      graphics.FillPath(&brush,&path);
    } else graphics.FillRectangle(&brush,rect.left,rect.top,rect.right-rect.left,rect.bottom-rect.top);
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
    COLORREF color=id==15 || id==14 ? red : id==11 || (quick && !current.state.joined) || (header && (stop || current.state.joined)) ? accent : id==12 && current.capturing ? accent : id==13 && current.state.deafened ? red : ink;
    if(!enabled && !header) color=RGB(76,85,102);
    POINT cursor{}; GetCursorPos(&cursor); RECT bounds{}; GetWindowRect(item.hwndItem,&bounds);
    const bool hover=enabled && PtInRect(&bounds,cursor);
    const bool active=id==11 || (id==12 && current.state.microphone) || (id==13 && current.state.deafened);
    RECT position{}; GetWindowRect(item.hwndItem,&position); MapWindowPoints(nullptr,window,reinterpret_cast<POINT*>(&position),2);
    backdrop(dc,item.rcItem,position.top);
    if(!header && (id!=16 || hover))
      fill(dc,item.rcItem,(item.itemState&ODS_SELECTED) ? RGB(48,62,82) : hover ? RGB(37,49,66) : id==15 ? RGB(46,29,39) : active ? RGB(22,49,46) : surface,16);
    if(header) {
      RECT tile{0,px(2),px(28),px(30)}; fill(dc,tile,RGB(20,49,45),16); icon(dc,px(4),px(6),10,color);
      RECT title{px(36),-px(1),px(164),px(17)},detail{px(36),px(18),px(164),px(32)};
      text(dc,stop ? L"Screen sharing" : L"Voice chat",title,titleFont,ink,DT_LEFT|DT_SINGLELINE);
      text(dc,subtitle(),detail,detailFont,current.state.joined ? accent : muted,DT_LEFT|DT_SINGLELINE);
    } else {
      const wchar_t* caption=quick ? (!current.state.joined ? L"Join" : current.capturing ? L"Mic on" : current.state.microphone ? L"…" : L"Muted") : id==11 ? L"Join voice" : id==12 ? (current.state.microphone ? L"Mute mic" : L"Unmute") : id==13 ? (current.state.deafened ? L"Hear voice" : L"Mute voice") : nullptr;
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
      roundedPath(shape,0.5f,0.5f,rect.right-0.5f,rect.bottom-0.5f,static_cast<float>(px(30)));
      graphics.DrawPath(&border,&shape);
      if(expanded) {
        Gdiplus::Pen separator(Gdiplus::Color(20,255,255,255),1.f);
        graphics.DrawLine(&separator,px(14),px(52),px(width()-14),px(52));
      }
    }
    if(expanded) {
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
      for(auto child:{toggle,shortcut,chevron,stopButton,join,microphone,deafen,leave}) {
        if(!IsWindowVisible(child)) continue;
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
  ~Impl() { if(window) { KillTimer(window,1); if(tooltip) DestroyWindow(tooltip); DestroyWindow(window); } if(titleFont) DeleteObject(titleFont); if(detailFont) DeleteObject(detailFont); if(controlFont) DeleteObject(controlFont); }
};
VoicePanel::VoicePanel(bool enabled,VoiceRuntime& voice,std::function<void()> stopSharing) {
  if(enabled || stopSharing) impl_=std::make_unique<Impl>(enabled,voice,std::move(stopSharing));
}
VoicePanel::~VoicePanel()=default;
void VoicePanel::pump() {
  if(!impl_) return;
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
}
#endif
}
