#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include <mfapi.h>
#include <mfidl.h>
#include <mftransform.h>
#include <mferror.h>
#include <wmcodecdsp.h>
#include <codecapi.h>
#include <wrl/client.h>
#include "transport.h"
#include "telemetry.h"
#include "broadcast.h"
#include "voice_runtime.h"
#include "voice_panel.h"
#include "decode_worker.h"
#include "latest_frame_worker.h"
#include "audio_player.h"
#include "audio_capture.h"
#include "engine.h"
#include "keyboard.h"
#include "window_presenter.h"
#include "desktop_duplication.h"
#include "../../windows-uac/client.h"
#include "mf_video_packet_encoder.h"
#include <algorithm>
#include <memory>
#include <iostream>
#include <set>
#include <stdexcept>
#include <fstream>
#include <cstdlib>
using Microsoft::WRL::ComPtr;
namespace sanser::desktop {
namespace {
void check(HRESULT hr,const char* operation) { if(FAILED(hr)) throw std::runtime_error(std::string(operation)+" failed ("+std::to_string(static_cast<unsigned long>(hr))+")"); }
struct Runtime {
  Runtime() { check(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"COM startup"); check(MFStartup(MF_VERSION),"Media Foundation startup"); }
  ~Runtime() { MFShutdown(); CoUninitialize(); }
};
class Injector {
public:
  explicit Injector(const DesktopDuplicator& display) : left_(display.left()),top_(display.top()),width_(display.width()),height_(display.height()) {}
  ~Injector() { reset(); }
  void apply(const Input& event) {
    if(event.kind==Input::Reset) { reset(); return; }
    INPUT input{};
    if(event.kind==Input::Key) {
      const auto scan=windowsKey(event.code); if(!scan) return;
      input.type=INPUT_KEYBOARD; input.ki.wScan=scan&255; input.ki.dwFlags=KEYEVENTF_SCANCODE|(scan&256?KEYEVENTF_EXTENDEDKEY:0)|(event.down?0:KEYEVENTF_KEYUP);
      if(event.down) keys_.insert(event.code); else keys_.erase(event.code);
    } else {
      input.type=INPUT_MOUSE;
      const auto vx=GetSystemMetrics(SM_XVIRTUALSCREEN),vy=GetSystemMetrics(SM_YVIRTUALSCREEN);
      const auto vw=std::max(1,GetSystemMetrics(SM_CXVIRTUALSCREEN)-1),vh=std::max(1,GetSystemMetrics(SM_CYVIRTUALSCREEN)-1);
      input.mi.dx=static_cast<LONG>((left_-vx+event.x/65535.0*std::max(1U,width_-1))*65535/vw);
      input.mi.dy=static_cast<LONG>((top_-vy+event.y/65535.0*std::max(1U,height_-1))*65535/vh);
      input.mi.dwFlags=MOUSEEVENTF_MOVE|MOUSEEVENTF_ABSOLUTE|MOUSEEVENTF_VIRTUALDESK;
      if(event.kind==Input::Button) {
        static constexpr DWORD down[]={MOUSEEVENTF_LEFTDOWN,MOUSEEVENTF_RIGHTDOWN,MOUSEEVENTF_MIDDLEDOWN};
        static constexpr DWORD up[]={MOUSEEVENTF_LEFTUP,MOUSEEVENTF_RIGHTUP,MOUSEEVENTF_MIDDLEUP};
        input.mi.dwFlags|=event.down?down[event.code]:up[event.code];
        if(event.down) buttons_.insert(event.code); else buttons_.erase(event.code);
      }
      if(event.kind==Input::Scroll) { input.mi.dwFlags=MOUSEEVENTF_WHEEL; input.mi.mouseData=static_cast<DWORD>(static_cast<int>(event.delta)*WHEEL_DELTA); }
    }
    sanser::uac::sendInput(1,&input,sizeof(input));
  }
private:
  void reset() {
    for(auto it=keys_.begin();it!=keys_.end();) {
      const auto scan=windowsKey(*it); INPUT input{}; input.type=INPUT_KEYBOARD; input.ki.wScan=scan&255;
      input.ki.dwFlags=KEYEVENTF_SCANCODE|KEYEVENTF_KEYUP|(scan&256?KEYEVENTF_EXTENDEDKEY:0);
      if(sanser::uac::sendInput(1,&input,sizeof(input))==1) it=keys_.erase(it); else ++it;
    }
    static constexpr DWORD up[]={MOUSEEVENTF_LEFTUP,MOUSEEVENTF_RIGHTUP,MOUSEEVENTF_MIDDLEUP};
    for(auto it=buttons_.begin();it!=buttons_.end();) {
      INPUT input{}; input.type=INPUT_MOUSE; input.mi.dwFlags=up[*it];
      if(sanser::uac::sendInput(1,&input,sizeof(input))==1) it=buttons_.erase(it); else ++it;
    }
  }
  long left_,top_; std::uint32_t width_,height_; std::set<std::uint16_t> keys_,buttons_;
};
struct Image { Bytes pixels; std::uint32_t width=0,height=0; Clock::time_point decodedAt=Clock::now(); };
struct Nv12Image { Bytes pixels; std::uint32_t width=0,height=0,stride=0,rows=0,left=0,top=0; Clock::time_point decodedAt=Clock::now(); };
Image convertNv12(const Nv12Image& source) {
  const auto width=source.width,height=source.height,stride=source.stride;
  if(!width || !height || source.left+width>stride || source.top+height>source.rows || source.rows%2
      || source.pixels.size()<static_cast<std::size_t>(stride)*source.rows*3/2)
    throw std::runtime_error("Invalid NV12 display aperture");
  Image image{Bytes(static_cast<std::size_t>(width)*height*4),width,height,source.decodedAt};
  const auto* data=source.pixels.data();
  auto clamp=[](int n) { return static_cast<std::uint8_t>(std::clamp(n,0,255)); };
  for(std::uint32_t y=0;y<height;++y) for(std::uint32_t x=0;x<width;++x) {
    const auto px=x+source.left,py=y+source.top;
    const int luminance=static_cast<int>(data[static_cast<std::size_t>(py)*stride+px])-16;
    const auto uv=static_cast<std::size_t>(stride)*source.rows+static_cast<std::size_t>(py/2)*stride+(px&~1U);
    const int u=static_cast<int>(data[uv])-128,v=static_cast<int>(data[uv+1])-128;
    const auto offset=(static_cast<std::size_t>(y)*width+x)*4;
    image.pixels[offset]=clamp((298*luminance+516*u+128)>>8); image.pixels[offset+1]=clamp((298*luminance-100*u-208*v+128)>>8); image.pixels[offset+2]=clamp((298*luminance+409*v+128)>>8); image.pixels[offset+3]=255;
  }
  return image;
}
void setDisplayAperture(IMFMediaType* type,Nv12Image& image,std::uint32_t codedWidth,std::uint32_t codedHeight) {
  image.width=codedWidth; image.height=codedHeight; image.rows=codedHeight;
  // MF_MT_FRAME_SIZE includes H.264 macroblock padding (e.g. 1846 -> 1856).
  // UV still starts after all coded rows, even when only a smaller aperture
  // is displayed. Keep its storage geometry separate from visible dimensions.
  for(const auto& key:{MF_MT_MINIMUM_DISPLAY_APERTURE,MF_MT_GEOMETRIC_APERTURE}) {
    MFVideoArea area{}; UINT32 size=0;
    if(FAILED(type->GetBlob(key,reinterpret_cast<UINT8*>(&area),sizeof(area),&size)) || size!=sizeof(area)) continue;
    if(area.OffsetX.value<0 || area.OffsetY.value<0 || area.Area.cx<=0 || area.Area.cy<=0
        || static_cast<std::uint32_t>(area.OffsetX.value)+static_cast<std::uint32_t>(area.Area.cx)>codedWidth
        || static_cast<std::uint32_t>(area.OffsetY.value)+static_cast<std::uint32_t>(area.Area.cy)>codedHeight)
      throw std::runtime_error("Invalid decoder display aperture");
    image.left=static_cast<std::uint32_t>(area.OffsetX.value); image.top=static_cast<std::uint32_t>(area.OffsetY.value);
    image.width=static_cast<std::uint32_t>(area.Area.cx); image.height=static_cast<std::uint32_t>(area.Area.cy); return;
  }
}
class Decoder {
public:
  Decoder(std::uint32_t width, std::uint32_t height, std::function<void(Nv12Image)> output) : output_(std::move(output)) {
    check(CoCreateInstance(CLSID_CMSH264DecoderMFT,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&decoder_)),"Create H.264 decoder");
    ComPtr<IMFMediaType> input; check(MFCreateMediaType(&input),"Create video type");
    check(input->SetGUID(MF_MT_MAJOR_TYPE,MFMediaType_Video),"Set video type"); check(input->SetGUID(MF_MT_SUBTYPE,MFVideoFormat_H264),"Set H.264 type");
    check(MFSetAttributeSize(input.Get(), MF_MT_FRAME_SIZE, width, height), "Set H.264 frame size");
    check(decoder_->SetInputType(0,input.Get(),0),"Configure H.264 decoder");
    configureOutput();
    ComPtr<ICodecAPI> codec; bool lowLatency=false;
    if(SUCCEEDED(decoder_.As(&codec))) { VARIANT low{}; low.vt=VT_UI4; low.ulVal=1; lowLatency=SUCCEEDED(codec->SetValue(&CODECAPI_AVLowLatencyMode,&low)); }
    ComPtr<IMFAttributes> attributes;
    if(SUCCEEDED(decoder_->GetAttributes(&attributes))) (void)attributes->SetUINT32(MF_LOW_LATENCY,TRUE);
    std::cout<<"SNV2_DECODER lowLatency="<<(lowLatency ? 1 : 0)<<std::endl;
    decoder_->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,0); decoder_->ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM,0);
  }
  void decode(const Frame& frame) {
    ComPtr<IMFSample> sample; ComPtr<IMFMediaBuffer> buffer;
    check(MFCreateSample(&sample),"Create sample"); check(MFCreateMemoryBuffer(static_cast<DWORD>(frame.data.size()),&buffer),"Allocate sample");
    BYTE* target=nullptr; check(buffer->Lock(&target,nullptr,nullptr),"Lock sample"); std::copy(frame.data.begin(),frame.data.end(),target); buffer->Unlock();
    buffer->SetCurrentLength(static_cast<DWORD>(frame.data.size())); sample->AddBuffer(buffer.Get()); sample->SetSampleTime(time_); sample->SetSampleDuration(166667); time_+=166667;
    sample->SetUINT32(MFSampleExtension_CleanPoint,frame.keyframe ? TRUE : FALSE);
    if(firstInput_) { sample->SetUINT32(MFSampleExtension_Discontinuity,TRUE); firstInput_=false; }
    auto status=decoder_->ProcessInput(0,sample.Get(),0);
    if(status==MF_E_NOTACCEPTING) { drain(); status=decoder_->ProcessInput(0,sample.Get(),0); }
    check(status,"Submit H.264 frame"); drain();
  }
private:
  void configureOutput() {
    for(DWORD index=0;;++index) { ComPtr<IMFMediaType> type; const auto status=decoder_->GetOutputAvailableType(0,index,&type); if(status==MF_E_NO_MORE_TYPES) break; check(status,"Get decoder output type"); GUID subtype{}; type->GetGUID(MF_MT_SUBTYPE,&subtype); if(subtype!=MFVideoFormat_NV12) continue; if(SUCCEEDED(decoder_->SetOutputType(0,type.Get(),0))) { type_=type; return; } }
    throw std::runtime_error("H.264 decoder does not provide NV12 output");
  }
  void drain() {
    for(unsigned iteration=0;iteration<16;++iteration) {
      MFT_OUTPUT_STREAM_INFO info{}; check(decoder_->GetOutputStreamInfo(0,&info),"Get decoder buffer size");
      MFT_OUTPUT_DATA_BUFFER output{}; output.dwStreamID=0;
      ComPtr<IMFSample> sample; ComPtr<IMFMediaBuffer> buffer;
      if(!(info.dwFlags&MFT_OUTPUT_STREAM_PROVIDES_SAMPLES)) { check(MFCreateSample(&sample),"Create output sample"); check(MFCreateMemoryBuffer(info.cbSize,&buffer),"Allocate output sample"); sample->AddBuffer(buffer.Get()); output.pSample=sample.Get(); }
      DWORD flags=0; auto status=decoder_->ProcessOutput(0,1,&output,&flags); if(output.pEvents) output.pEvents->Release();
      ComPtr<IMFSample> owned;
      if(!sample && output.pSample) owned.Attach(output.pSample);
      if(status==MF_E_TRANSFORM_STREAM_CHANGE) { configureOutput(); continue; }
      if(status==MF_E_TRANSFORM_NEED_MORE_INPUT) return;
      check(status,"Decode H.264 frame");
      if(!output.pSample) return;
      check(output.pSample->ConvertToContiguousBuffer(&buffer),"Read decoded frame");
      UINT32 width=0,height=0; check(MFGetAttributeSize(type_.Get(),MF_MT_FRAME_SIZE,&width,&height),"Get decoded size");
      if(width<64||height<64||width>3840||height>2160||width%2||height%2) throw std::runtime_error("Invalid decoded dimensions");
      UINT32 strideValue=width; type_->GetUINT32(MF_MT_DEFAULT_STRIDE,&strideValue); const auto stride=static_cast<LONG>(strideValue);
      if(stride<static_cast<LONG>(width)||stride>16384) throw std::runtime_error("Invalid decoder stride");
      Nv12Image image; image.stride=static_cast<std::uint32_t>(stride); setDisplayAperture(type_.Get(),image,width,height);
      BYTE* data=nullptr; DWORD size=0; check(buffer->Lock(&data,nullptr,&size),"Lock decoded frame");
      if(static_cast<std::size_t>(size)<static_cast<std::size_t>(stride)*height*3/2) { buffer->Unlock(); throw std::runtime_error("Truncated decoded image"); }
      image.pixels.assign(data,data+static_cast<std::size_t>(stride)*height*3/2);
      buffer->Unlock(); output_(std::move(image)); if(!reported_) { reported_=true; std::cout<<"SNV2_FIRST_FRAME decoded=1"<<std::endl; }
    }
  }
  ComPtr<IMFTransform> decoder_; ComPtr<IMFMediaType> type_; std::function<void(Nv12Image)> output_; LONGLONG time_=0; bool reported_=false,firstInput_=true;
};
struct WindowState {
  bool viewOnly=false;
  bool userClosed=false;
  bool inputPaused=false;
  std::function<void(Input)> sendInput; std::mutex mutex; std::shared_ptr<const Image> image;
  bool released=false; RECT video{};
  WindowPresenter presenter;
  std::weak_ptr<const Image> lastPresented;
  std::uint64_t painted=0,paintUs=0,decodedToPaintUs=0;
  // Auto-reset event coalesces decoded frames instead of queuing a UI message
  // for every frame. Painting retains a snapshot without blocking decoding.
  HANDLE frameReady=CreateEventW(nullptr,FALSE,FALSE,nullptr);
  WindowState() { if(!frameReady) throw std::runtime_error("Unable to create frame notification"); }
  ~WindowState() { CloseHandle(frameReady); }
};
std::uint16_t windowsHid(WPARAM key,LPARAM detail) {
  auto scan=static_cast<std::uint16_t>((detail>>16)&0xff);
  bool extended=(detail&(1LL<<24))!=0;
  // Some IMEs and injected Windows keyboard messages omit lParam's scan code.
  // Resolve their virtual key rather than silently dropping ordinary typing.
  if(!scan) {
    const auto mapped=MapVirtualKeyW(static_cast<UINT>(key),MAPVK_VK_TO_VSC_EX);
    scan=static_cast<std::uint16_t>(mapped&255); extended=(mapped&0xff00)==0xe000;
  }
  return hidFromWindows(static_cast<std::uint16_t>(scan|(extended ? 0x100 : 0)));
}
LRESULT CALLBACK windowProc(HWND window,UINT message,WPARAM w,LPARAM l) {
  auto* state=reinterpret_cast<WindowState*>(GetWindowLongPtrW(window,GWLP_USERDATA));
  if(message==WM_NCCREATE) { state=static_cast<WindowState*>(reinterpret_cast<CREATESTRUCTW*>(l)->lpCreateParams); SetWindowLongPtrW(window,GWLP_USERDATA,reinterpret_cast<LONG_PTR>(state)); }
  if(!state) return DefWindowProcW(window,message,w,l);
  if((state->viewOnly || state->inputPaused) && ((message>=WM_KEYFIRST && message<=WM_KEYLAST) || (message>=WM_MOUSEFIRST && message<=WM_MOUSELAST))) return DefWindowProcW(window,message,w,l);
  if(message==WM_CLOSE) { state->userClosed=true; DestroyWindow(window); return 0; }
  if(message==WM_DESTROY) { PostQuitMessage(0); return 0; }
  if(message==WM_ERASEBKGND) return 1;
  if(message==WM_PAINT) {
    const auto began=Clock::now();
    PAINTSTRUCT paint{}; HDC dc=BeginPaint(window,&paint); RECT area{}; GetClientRect(window,&area);
    std::shared_ptr<const Image> snapshot;
    { std::lock_guard lock(state->mutex); snapshot=state->image; }
    const auto* pixels=snapshot && !snapshot->pixels.empty() ? snapshot->pixels.data() : nullptr;
    const bool presented=state->presenter.paint(dc,area.right,area.bottom,pixels,snapshot ? snapshot->width : 0,snapshot ? snapshot->height : 0,state->video);
    if(!presented)
      std::cerr<<"Unable to paint the remote desktop frame"<<std::endl;
    // Do not call repeated paints of an unchanged idle picture frame latency.
    if(presented && pixels && area.right>0 && area.bottom>0 && state->lastPresented.lock()!=snapshot) {
      ++state->painted;
      state->paintUs+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(Clock::now()-began).count());
      state->decodedToPaintUs+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(Clock::now()-snapshot->decodedAt).count());
      state->lastPresented=snapshot;
    }
    EndPaint(window,&paint); return 0;
  }
  // Windows runs its own modal loop while a window is dragged or resized.
  // Its loop cannot wait on frameReady, so keep repainting the latest frame
  // with a short-lived timer until normal event processing resumes.
  if(message==WM_ENTERSIZEMOVE) { SetTimer(window,1,16,nullptr); return 0; }
  if(message==WM_EXITSIZEMOVE) { KillTimer(window,1); InvalidateRect(window,nullptr,FALSE); return 0; }
  if(message==WM_TIMER && w==1) { if(WaitForSingleObject(state->frameReady,0)==WAIT_OBJECT_0) InvalidateRect(window,nullptr,FALSE); return 0; }
  if(message==WM_SIZE) { InvalidateRect(window,nullptr,FALSE); return 0; }
  if(message==WM_KILLFOCUS) { if(!state->viewOnly) state->sendInput(Input{}); state->released=true; if(GetCapture()==window) ReleaseCapture(); return 0; }
  if(message==WM_KEYDOWN || message==WM_SYSKEYDOWN || message==WM_KEYUP || message==WM_SYSKEYUP) {
    const bool down=message==WM_KEYDOWN||message==WM_SYSKEYDOWN;
    if(down && w==VK_ESCAPE && (GetKeyState(VK_CONTROL)&0x8000) && (GetKeyState(VK_MENU)&0x8000)) { state->sendInput(Input{}); state->released=true; ReleaseCapture(); SetWindowTextW(window,L"Sanser · Input released · Click to resume"); return 0; }
    if(!state->released) { const auto hid=windowsHid(w,l); if(hid) state->sendInput(Input{Input::Key,0,0,hid,0,down}); }
    return 0;
  }
  if(message==WM_MOUSEMOVE || message==WM_LBUTTONDOWN || message==WM_LBUTTONUP || message==WM_RBUTTONDOWN || message==WM_RBUTTONUP || message==WM_MBUTTONDOWN || message==WM_MBUTTONUP || message==WM_MOUSEWHEEL) {
    if(message==WM_LBUTTONDOWN && state->released) { SetFocus(window); state->released=false; SetWindowTextW(window,L"Sanser · Remote desktop · Ctrl+Alt+Escape releases input"); }
    if(state->released) return 0;
    POINT point{static_cast<short>(LOWORD(l)),static_cast<short>(HIWORD(l))}; if(message==WM_MOUSEWHEEL) ScreenToClient(window,&point);
    const auto& r=state->video; if(r.right<=r.left || r.bottom<=r.top) return 0;
    Input event; event.kind=Input::Move; event.x=static_cast<std::uint16_t>(std::clamp((point.x-r.left)/static_cast<double>(r.right-r.left),0.0,1.0)*65535); event.y=static_cast<std::uint16_t>(std::clamp((point.y-r.top)/static_cast<double>(r.bottom-r.top),0.0,1.0)*65535);
    if(message==WM_MOUSEWHEEL) { event.kind=Input::Scroll; event.delta=static_cast<short>(HIWORD(w))/WHEEL_DELTA; }
    else if(message!=WM_MOUSEMOVE) { event.kind=Input::Button; event.down=message==WM_LBUTTONDOWN||message==WM_RBUTTONDOWN||message==WM_MBUTTONDOWN; event.code=(message==WM_RBUTTONDOWN||message==WM_RBUTTONUP)?1:(message==WM_MBUTTONDOWN||message==WM_MBUTTONUP)?2:0; if(event.down) SetCapture(window); else ReleaseCapture(); }
    state->sendInput(event); return 0;
  }
  return DefWindowProcW(window,message,w,l);
}
bool pumpWindowMessages(MSG& message) {
  // A busy input queue must yield to frameReady. WM_PAINT is normally below
  // mouse/key messages in priority, so pumping until empty can starve video.
  const auto deadline=Clock::now()+std::chrono::milliseconds(2);
  for(unsigned count=0;count<64 && PeekMessageW(&message,nullptr,0,0,PM_REMOVE);++count) {
    if(message.message==WM_QUIT) return false;
    TranslateMessage(&message); DispatchMessageW(&message);
    if(Clock::now()>=deadline) break;
  }
  return true;
}
void presentReadyFrame(HWND window) {
  // Send WM_PAINT now, including when mouse traffic keeps the queue nonempty.
  RedrawWindow(window,nullptr,nullptr,RDW_INVALIDATE|RDW_UPDATENOW|RDW_NOERASE);
}
}
#ifdef SANSER_CODEC_TEST
void testWindowsInput() {
  const auto require=[](bool value,const char* message) { if(!value) throw std::runtime_error(message); };
  WindowState state; std::vector<Input> events; state.sendInput=[&](Input input) { events.push_back(input); };
  WNDCLASSW klass{}; klass.lpfnWndProc=windowProc; klass.hInstance=GetModuleHandleW(nullptr); klass.lpszClassName=L"SanserInputRegression"; RegisterClassW(&klass);
  HWND window=CreateWindowW(klass.lpszClassName,L"Input test",WS_OVERLAPPEDWINDOW,0,0,640,480,nullptr,nullptr,klass.hInstance,&state);
  require(window!=nullptr,"Create native input test window");
  struct Close { HWND window; ~Close() { if(IsWindow(window)) DestroyWindow(window); } } close{window};
  // A real click is permitted only after a picture exists. Focus changes may
  // synchronously paint the window; a waiting/empty picture clears video bounds.
  state.image=std::make_shared<const Image>(Image{Bytes(64*48*4,127),64,48});
  state.video={0,0,640,480}; state.released=true; events.clear();
  SendMessageW(window,WM_LBUTTONDOWN,MK_LBUTTON,MAKELPARAM(100,50));
  SendMessageW(window,WM_LBUTTONUP,0,MAKELPARAM(100,50));
  std::vector<Input> clicks;
  for(const auto& event:events) if(event.kind==Input::Button) clicks.push_back(event);
  if(state.released || clicks.size()!=2) {
    std::cerr<<"Resume input released="<<state.released<<" events="<<events.size()<<" clickEvents="<<clicks.size()<<" video="<<state.video.right<<'x'<<state.video.bottom<<'\n';
    for(const auto& event:events) std::cerr<<"Input kind="<<static_cast<int>(event.kind)<<" down="<<event.down<<'\n';
  }
  require(!state.released && clicks.size()==2 && clicks[0].down && !clicks[1].down,"Resume click must forward both down and up");
  events.clear();
  SendMessageW(window,WM_KEYDOWN,'A',0); SendMessageW(window,WM_KEYUP,'A',0);
  require(events.size()==2 && events[0].kind==Input::Key && events[0].code==4 && events[0].down && !events[1].down,"Windows virtual-key fallback did not forward ordinary typing");
  events.clear();
  for(const auto scan:{0x2a,0x36}) {
    SendMessageW(window,WM_KEYDOWN,VK_SHIFT,static_cast<LPARAM>(scan)<<16);
    SendMessageW(window,WM_KEYDOWN,'A',0x1eLL<<16);
    SendMessageW(window,WM_KEYUP,'A',(0x1eLL<<16)|(1LL<<30)|(1LL<<31));
    SendMessageW(window,WM_KEYUP,VK_SHIFT,(static_cast<LPARAM>(scan)<<16)|(1LL<<30)|(1LL<<31));
  }
  require(events.size()==8 && events[0].code==225 && events[0].down && events[1].code==4 && events[1].down && !events[2].down && events[3].code==225 && !events[3].down
    && events[4].code==229 && events[4].down && events[5].code==4 && events[5].down && !events[6].down && events[7].code==229 && !events[7].down,
    "Windows must forward both Shift+A chords in press/release order");
  require(windowsHid(VK_LSHIFT,0)==225 && windowsHid(VK_RSHIFT,0)==229,"Scan-less Shift messages lost their side");
  require(windowsHid(VK_RETURN,(0x1cLL<<16)|(1LL<<24))==88,"Keypad Enter lost its extended scan code");
  SendMessageW(window,WM_KILLFOCUS,0,0);
  require(state.released && events.back().kind==Input::Reset,"Focus loss did not release remote keys/buttons");
  state.viewOnly=true; const auto before=events.size();
  SendMessageW(window,WM_KEYDOWN,'A',0); SendMessageW(window,WM_LBUTTONDOWN,0,MAKELPARAM(100,50));
  SendMessageW(window,WM_KILLFOCUS,0,0);
  require(events.size()==before,"View-only window forwarded input");
  ShowWindow(window,SW_SHOWNOACTIVATE);
  // Posted app messages are not coalesced like physical WM_MOUSEMOVE events.
  // Keep the queue provably nonempty while testing scheduling fairness.
  constexpr UINT floodMessage=WM_APP+17;
  for(unsigned i=0;i<256;++i) require(PostMessageW(window,floodMessage,0,0),"Unable to queue message flood");
  MSG message{};
  require(pumpWindowMessages(message),"Input pump unexpectedly quit");
  require(PeekMessageW(&message,window,floodMessage,floodMessage,PM_NOREMOVE),"Input pump must yield while messages remain queued");
  state.video={}; presentReadyFrame(window);
  require(state.video.right>state.video.left,"A ready picture was not painted during queued input");
  while(PeekMessageW(&message,window,floodMessage,floodMessage,PM_REMOVE)) {}
  SendMessageW(window,WM_CLOSE,0,0);
  require(state.userClosed && !IsWindow(window),"Viewer X must emit a deliberate close, independently of main-shell tray behavior");
  std::cout<<"Windows input: resume click, keyboard fallback, keypad, focus reset and view-only enforcement passed\n";
}
void testWindowsMacFixture(const char* path,bool lowDelay) {
  // A padded/cropped decoder surface must use coded height for the UV plane,
  // not the smaller display height, and must not paint macroblock padding.
  Nv12Image padded{Bytes(8*6*3/2,128),2,2,8,6,2,2};
  std::fill(padded.pixels.begin(),padded.pixels.begin()+8*6,16);
  for(unsigned y=2;y<4;++y) for(unsigned x=2;x<4;++x) padded.pixels[y*8+x]=41;
  padded.pixels[8*6+8+2]=240; padded.pixels[8*6+8+3]=110;
  const auto cropped=convertNv12(padded);
  for(std::size_t pixel=0;pixel<cropped.pixels.size();pixel+=4)
    if(cropped.pixels[pixel]<240 || cropped.pixels[pixel+1]>10 || cropped.pixels[pixel+2]>10)
      throw std::runtime_error("Padded NV12 conversion used the wrong image/chroma aperture");
  std::ifstream file(path,std::ios::binary);
  const auto read=[&]() { std::uint32_t value=0; for(unsigned i=0;i<4;++i) { const auto c=file.get(); if(c==EOF) throw std::runtime_error("Truncated Mac fixture"); value=(value<<8)|static_cast<unsigned char>(c); } return value; };
  char magic[4]{}; file.read(magic,4); if(std::string(magic,4)!="SVC1") throw std::runtime_error("Invalid Mac fixture");
  const auto width=read(),height=read(),count=read();
  if(width>3840 || height>2160 || count!=45) throw std::runtime_error("Invalid Mac fixture dimensions/count");
  std::mutex mutex; std::condition_variable changed; std::shared_ptr<const Image> latest;
  std::atomic<unsigned> decoded{0},converted{0},keyframes{0}; std::atomic<bool> conversionError{false};
  LatestFrameWorker<Nv12Image> conversion([&](Nv12Image frame) {
    try {
      auto image=std::make_shared<const Image>(convertNv12(frame));
      { std::lock_guard lock(mutex); latest=std::move(image); }
      ++converted; changed.notify_all();
    } catch(...) { conversionError=true; changed.notify_all(); }
  });
  std::unique_ptr<Runtime> runtime; std::unique_ptr<Decoder> decoder;
  unsigned decodedBase=0;
  DecodeWorker worker([&](Frame frame) {
    if(!runtime) runtime=std::make_unique<Runtime>();
    if(!decoder) decoder=std::make_unique<Decoder>(width,height,[&](Nv12Image image) { conversion.submit(std::move(image)); ++decoded; changed.notify_all(); });
    decoder->decode(frame);
  },[&] { decoder.reset(); runtime.reset(); decodedBase=decoded.load(); },[&] { ++keyframes; changed.notify_all(); },true,false,
    [] { return Clock::now(); },[&] { return decoded.load()>decodedBase; });
  for(unsigned i=0;i<count;++i) {
    const auto size=read(); const auto key=file.get();
    if(size==0 || size>4*1024*1024 || (key!=0 && key!=1)) throw std::runtime_error("Invalid Mac access unit");
    Frame frame{Bytes(size),width,height,key==1}; file.read(reinterpret_cast<char*>(frame.data.data()),size);
    if(!file) throw std::runtime_error("Truncated Mac access unit");
    worker.submit(std::move(frame));
    if(lowDelay) {
      // Submit exactly one access unit, then stop feeding. An idle remote
      // desktop must not wait for another mouse movement to expose this frame.
      std::unique_lock lock(mutex);
      const auto expected=i+1;
      if(!changed.wait_for(lock,std::chrono::seconds(2),[&] { return converted>=expected || conversionError; }) || conversionError)
        throw std::runtime_error("Low-delay Windows decoder held frame "+std::to_string(i)+" waiting for future input");
      if(decoded!=expected || converted!=expected || !latest)
        throw std::runtime_error("Low-delay stream did not produce one picture for one input");
      const auto* pixel=latest->pixels.data();
      const auto channel=i<15 ? 2 : i<30 ? 1 : 0;
      if(latest->width!=width || latest->height!=height || pixel[channel]<150 || pixel[(channel+1)%3]>80 || pixel[(channel+2)%3]>80)
        throw std::runtime_error("Paused Mac stream displayed a stale or incorrect picture");
    }
    // Feed a live stream, not one picture followed by silence: a compliant MF
    // decoder may retain a small startup/reference delay even in low-latency
    // mode. Require continuous output and the final changed picture below.
    std::this_thread::sleep_for(std::chrono::milliseconds(20));
    if(worker.failures()>0)
      throw std::runtime_error("Windows stalled decoding Mac frame "+std::to_string(i)+": "+worker.error());
  }
  {
    std::unique_lock lock(mutex);
    const auto blue=[&] { return latest && latest->width==width && latest->height==height && latest->pixels[0]>150 && latest->pixels[1]<80 && latest->pixels[2]<80; };
    if(!changed.wait_for(lock,std::chrono::seconds(3),[&] { return blue() || conversionError; }) || !blue()) {
      const auto timing=worker.takeTiming();
      std::cerr<<"Mac fixture progress decoded="<<decoded<<" converted="<<converted<<" keyframeRequests="<<keyframes
        <<" latest="<<(latest ? latest->width : 0)<<'x'<<(latest ? latest->height : 0)
        <<" inputs="<<timing.samples<<" dropped="<<worker.dropped()<<" expired="<<worker.expired()
        <<" workUs="<<timing.workUs<<" queueMaxUs="<<timing.queueMaxUs<<" lastError="<<worker.error()<<'\n';
      throw std::runtime_error("Windows picture did not advance to the final blue Mac frame");
    }
  }
  worker.stop(); conversion.stop();
  if(decoded<40 || converted<2 || keyframes!=0) throw std::runtime_error("Mac stream lost decoder progress or reset references under conversion load");
  std::cout<<"Mac VideoToolbox -> Windows MF: decoded="<<decoded<<" converted="<<converted<<" dimensions="<<width<<'x'<<height<<" final changing picture verified\n";
}
#endif
int runWindows(bool host,int argc,char** argv) {
  SetProcessDPIAware(); const auto options=parseOptions(host,argc,argv);
  if(host) {
    DesktopDuplicator display; display.initialize(); HostOutput peer(options); Injector injector(display);
    VideoPacketEncodeOptions encoding; encoding.fps=options.fps; encoding.bitrate=options.bitrate; encoding.codec=VideoCodec::H264;
    encoding.live=options.sharing || options.ultraLowLatency;
    encoding.gpuDevice=encoding.live ? display.gpuDevice() : nullptr;
    auto encoder=std::make_unique<MfVideoPacketEncoder>(options.width,options.height,encoding);
    display.setGpuCapture(encoder->usingGpuInput());
    if(options.ultraLowLatency && !encoder->usingGpuInput())
      throw std::runtime_error("Ultra Low Latency requires GPU capture and hardware encoding on Windows. Choose Balanced if the GPU path is unavailable.");
    VoiceRuntime voice(options.groupVoice,[&](voice::Packet packet) { peer.voicePacket(std::move(packet)); },[&](voice::State state) { peer.setVoiceState(state); });
    VoicePanel voicePanel(options.groupVoice,voice,options.sharing ? std::function<void()>([&] { peer.stop(); }) : std::function<void()>{});
    peer.onVoice=[&](voice::Packet packet) { voice.receive(std::move(packet)); };
    peer.onInput=[&](Input event) { injector.apply(event); }; HostOutputStop shutdown{peer}; peer.start();
    std::atomic<bool> loopbackExcludesVoice{!options.groupVoice};
    AudioCapture audio(options.audio,[&](const AudioSamples& samples) {
      // Older Windows releases cannot exclude the voice playback process.
      // Never feed that playback back into the shared screen sound.
      if(loopbackExcludesVoice || !voice.listening()) peer.audio(samples);
    },options.groupVoice,[&](bool excluded) { loopbackExcludesVoice=excluded; });
    FrameBgra cached; bool hasCached=false, force=false,capturePaused=false;
    auto generation=display.generation();
    const auto mediaStart=Clock::now();
    auto bitrate=options.bitrate;
    while(peer.running()) {
      const auto began=Clock::now();
      peer.pumpControl();
      voicePanel.pump(); if(!peer.running()) break;
      const auto target=peer.bitrate();
      if(target!=bitrate && encoder->setBitrate(target)) bitrate=target;
      force=peer.takeKeyframeRequest() || force;
      if(peer.ready()) {
        FrameBgra frame;
        const bool captured=display.captureFrame(frame, options.ultraLowLatency ? 2 : 16);
        if(display.recovering()) {
          if(!capturePaused) {
            capturePaused=true; peer.setCapturePaused(true); cached=FrameBgra{}; hasCached=false;
            std::cerr<<"SNV2_CAPTURE_PAUSED reason=desktop-unavailable possibleSecureDesktop=yes control=alive"<<std::endl;
          }
          std::this_thread::sleep_for(std::chrono::milliseconds(10)); continue;
        }
        if(generation!=display.generation()) {
          if(encoding.gpuDevice && encoding.gpuDevice!=display.gpuDevice()) {
            encoding.gpuDevice=display.gpuDevice();
            encoder=std::make_unique<MfVideoPacketEncoder>(options.width,options.height,encoding);
            bitrate=options.bitrate;
            display.setGpuCapture(encoder->usingGpuInput());
          }
          generation=display.generation(); hasCached=false; cached=FrameBgra{}; force=true;
        }
        if(capturePaused) {
          capturePaused=false; peer.setCapturePaused(false); force=true;
          std::cerr<<"SNV2_CAPTURE_RESUMED generation="<<generation<<" keyframeRequested=1"<<std::endl;
        }
        if(captured && options.ultraLowLatency && !frame.texture)
          throw std::runtime_error("Ultra Low Latency requires GPU capture. The display is using CPU capture; choose Balanced.");
        if(captured) { cached=frame; hasCached=true; }
        // Late viewers receive a fresh IDR even if the desktop is unchanged.
        const bool refresh=force && hasCached;
        if(refresh) { encoder->requestKeyframe(); force=false; }
        const auto timestamp=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(Clock::now()-mediaStart).count())+1;
        auto packets=captured || refresh ? encoder->encodeFrame(captured ? frame : cached,timestamp) : encoder->drain();
        for(auto& packet:packets) peer.video(Frame{std::move(packet.payload),options.width,options.height,packet.keyframe});
      }
      std::this_thread::sleep_until(began+std::chrono::microseconds(1000000/options.fps));
    }
    peer.stop(); return 0;
  }
  Peer peer(options); WindowState state; state.sendInput=[&](Input input) { peer.input(input); }; state.viewOnly=!options.input;
  VoiceRuntime voice(options.groupVoice,[&](voice::Packet packet) { peer.voicePacket(std::move(packet)); },[&](voice::State value) { peer.setVoiceState(value); });
  VoicePanel voicePanel(options.groupVoice,voice);
  peer.onVoice=[&](voice::Packet packet) { voice.receive(std::move(packet)); };
  AudioPlayer audio(options.audio);
  if(options.audio) peer.onAudio=[&](AudioSamples samples) { audio.submit(std::move(samples)); };
  if(options.audio) peer.onAudioReset=[&] { audio.reset(); };
  // Decoder COM objects belong to the decode worker, not the socket/UI threads.
  std::atomic<bool> conversionFailed{false};
  std::atomic<std::uint64_t> decodedFrames{0},convertedFrames{0};
  std::atomic<std::uint64_t> conversionUs{0};
  LatestFrameWorker<Nv12Image> conversion([&](Nv12Image pixels) {
    try {
      const auto began=Clock::now();
      auto snapshot=std::make_shared<const Image>(convertNv12(pixels));
      conversionUs+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(Clock::now()-began).count());
      { std::lock_guard lock(state.mutex); state.image=std::move(snapshot); }
      ++convertedFrames;
      SetEvent(state.frameReady);
    } catch(const std::exception& error) { std::cerr<<"SNV2_COLOR_CONVERSION failed: "<<error.what()<<std::endl; conversionFailed=true; }
  });
  std::unique_ptr<Runtime> runtime; std::unique_ptr<Decoder> decoder;
  std::uint64_t decodedBase=0;
  DecodeWorker decode([&](Frame frame) {
    if(!runtime) runtime=std::make_unique<Runtime>();
    if(!decoder) decoder=std::make_unique<Decoder>(frame.width,frame.height,[&](Nv12Image image) { ++decodedFrames; conversion.submit(std::move(image)); });
    decoder->decode(frame);
  },[&] { decoder.reset(); runtime.reset(); conversion.clear(); decodedBase=decodedFrames.load(); },[&] { peer.requestKeyframe(); },true,options.ultraLowLatency,
    [] { return Clock::now(); },[&] { return decodedFrames.load()>decodedBase; });
  peer.onFrame=[&](Frame frame) { decode.submit(std::move(frame)); };
  PeerStop shutdown{peer};
  const auto instance=GetModuleHandleW(nullptr); WNDCLASSW klass{}; klass.lpfnWndProc=windowProc; klass.hInstance=instance; klass.lpszClassName=L"SanserRemoteDesktop"; klass.hCursor=LoadCursor(nullptr,IDC_ARROW); RegisterClassW(&klass);
  HWND window=CreateWindowExW(0,klass.lpszClassName,options.input ? L"Sanser · Remote desktop · Ctrl+Alt+Escape releases input" : L"Sanser · View only",WS_OVERLAPPEDWINDOW,CW_USEDEFAULT,CW_USEDEFAULT,1280,760,nullptr,nullptr,instance,&state);
  if(!window) throw std::runtime_error("Unable to create remote desktop window"); ShowWindow(window,SW_SHOW); SetForegroundWindow(window); SetFocus(window); peer.start();
  MSG message{}; bool open=true;
  auto lastProgress=Clock::now();
  while(open && peer.running() && decode.failures()<10 && !conversionFailed) {
    if(options.input && state.inputPaused==peer.inputAvailable()) {
      state.inputPaused=!peer.inputAvailable(); state.released=true;
      if(GetCapture()==window) ReleaseCapture();
      SetWindowTextW(window,state.inputPaused ? peer.capturePaused() ? L"Sanser · Host capture temporarily unavailable · Check UAC on the host" : L"Sanser · Input paused · Waiting for authenticated reset; reconnect if it persists" : L"Sanser · Input restored · Click to resume");
    }
    if(std::getenv("SANSER_TELEMETRY") && Clock::now()-lastProgress>=std::chrono::seconds(1)) {
      std::ostringstream line; line<<"SNV2_VIDEO_HEALTH decoded="<<decodedFrames.load()<<" converted="<<convertedFrames.load()
        <<" encodedQueueDrops="<<decode.dropped()<<" decodeQueueExpired="<<decode.expired()<<" decodeErrors="<<decode.failures()<<" rendered="<<state.painted;
      const auto timing=decode.takeTiming(); const auto converted=convertedFrames.load();
      if(timing.samples) line<<" decodeWorkAvgMs="<<timing.workUs/1000.0/timing.samples
        <<" decodeQueueAvgMs="<<timing.queueUs/1000.0/timing.samples<<" decodeQueueMaxMs="<<timing.queueMaxUs/1000.0;
      if(converted) line<<" conversionAvgMs="<<conversionUs.load()/1000.0/converted;
      if(state.painted) line<<" paintAvgMs="<<state.paintUs/1000.0/state.painted<<" decodedToPaintAvgMs="<<state.decodedToPaintUs/1000.0/state.painted;
      writeTelemetry(line);
      if(options.audio) writeAudioOutputHealth(audio.stats());
      lastProgress=Clock::now();
    }
    const auto wake=MsgWaitForMultipleObjectsEx(1,&state.frameReady,100,QS_ALLINPUT,MWMO_INPUTAVAILABLE);
    if(wake==WAIT_FAILED) { peer.stop(); DestroyWindow(window); throw std::runtime_error("Remote window event wait failed"); }
    if(wake==WAIT_OBJECT_0) presentReadyFrame(window);
    open=pumpWindowMessages(message);
  }
  if(state.userClosed) std::cerr<<"SNV2_VIEWER_CLOSED"<<std::endl;
  peer.stop(); decode.stop(); conversion.stop(); if(IsWindow(window)) DestroyWindow(window);
  if(decode.failures()>=10) throw std::runtime_error("Unable to decode the remote video: "+decode.error());
  if(conversionFailed) throw std::runtime_error("Unable to convert the remote video frame");
  return 0;
}
}
