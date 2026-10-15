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
#include "broadcast.h"
#include "decode_worker.h"
#include "latest_frame_worker.h"
#include "audio_player.h"
#include "engine.h"
#include "keyboard.h"
#include "window_presenter.h"
#include "desktop_duplication.h"
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
LRESULT CALLBACK sharingNoticeProc(HWND window,UINT message,WPARAM w,LPARAM l) {
  auto* output=reinterpret_cast<HostOutput*>(GetWindowLongPtrW(window,GWLP_USERDATA));
  if(message==WM_NCCREATE) { output=static_cast<HostOutput*>(reinterpret_cast<CREATESTRUCTW*>(l)->lpCreateParams); SetWindowLongPtrW(window,GWLP_USERDATA,reinterpret_cast<LONG_PTR>(output)); }
  if((message==WM_COMMAND && LOWORD(w)==1) || message==WM_CLOSE) { if(output) output->stop(); return 0; }
  return DefWindowProcW(window,message,w,l);
}
struct SharingNotice {
  HWND window=nullptr;
  explicit SharingNotice(HostOutput& output,bool enabled) {
    if(!enabled) return;
    WNDCLASSW klass{}; klass.lpfnWndProc=sharingNoticeProc; klass.hInstance=GetModuleHandleW(nullptr); klass.lpszClassName=L"SanserSharingNotice"; klass.hbrBackground=reinterpret_cast<HBRUSH>(COLOR_WINDOW+1); klass.hCursor=LoadCursorW(nullptr,MAKEINTRESOURCEW(32512)); RegisterClassW(&klass);
    RECT area{}; SystemParametersInfoW(SPI_GETWORKAREA,0,&area,0);
    window=CreateWindowExW(WS_EX_TOPMOST|WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE,klass.lpszClassName,L"Sanser sharing",WS_POPUP|WS_BORDER,area.right-380,area.top+12,360,56,nullptr,nullptr,klass.hInstance,&output);
    if(!window) throw std::runtime_error("Unable to show the screen-sharing indicator");
    CreateWindowW(L"STATIC",L"Sanser · Screen sharing active",WS_CHILD|WS_VISIBLE,12,18,235,24,window,nullptr,klass.hInstance,nullptr);
    CreateWindowW(L"BUTTON",L"Stop",WS_CHILD|WS_VISIBLE|BS_PUSHBUTTON,258,10,88,34,window,reinterpret_cast<HMENU>(1),klass.hInstance,nullptr);
    ShowWindow(window,SW_SHOWNOACTIVATE);
  }
  ~SharingNotice() { if(window) DestroyWindow(window); }
  void pump() { if(!window) return; MSG message; while(PeekMessageW(&message,nullptr,0,0,PM_REMOVE)) { TranslateMessage(&message); DispatchMessageW(&message); } }
};
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
    SendInput(1,&input,sizeof(input));
  }
private:
  void reset() {
    for(auto code:keys_) { const auto scan=windowsKey(code); INPUT input{}; input.type=INPUT_KEYBOARD; input.ki.wScan=scan&255; input.ki.dwFlags=KEYEVENTF_SCANCODE|KEYEVENTF_KEYUP|(scan&256?KEYEVENTF_EXTENDEDKEY:0); SendInput(1,&input,sizeof(input)); } keys_.clear();
    static constexpr DWORD up[]={MOUSEEVENTF_LEFTUP,MOUSEEVENTF_RIGHTUP,MOUSEEVENTF_MIDDLEUP};
    for(auto code:buttons_) { INPUT input{}; input.type=INPUT_MOUSE; input.mi.dwFlags=up[code]; SendInput(1,&input,sizeof(input)); } buttons_.clear();
  }
  long left_,top_; std::uint32_t width_,height_; std::set<std::uint16_t> keys_,buttons_;
};
struct Image { Bytes pixels; std::uint32_t width=0,height=0; };
struct Nv12Image { Bytes pixels; std::uint32_t width=0,height=0,stride=0; };
Image convertNv12(const Nv12Image& source) {
  const auto width=source.width,height=source.height,stride=source.stride;
  Image image{Bytes(static_cast<std::size_t>(width)*height*4),width,height};
  const auto* data=source.pixels.data();
  auto clamp=[](int n) { return static_cast<std::uint8_t>(std::clamp(n,0,255)); };
  for(std::uint32_t y=0;y<height;++y) for(std::uint32_t x=0;x<width;++x) {
    const int luminance=static_cast<int>(data[static_cast<std::size_t>(y)*stride+x])-16;
    const auto uv=static_cast<std::size_t>(stride)*height+static_cast<std::size_t>(y/2)*stride+(x&~1U);
    const int u=static_cast<int>(data[uv])-128,v=static_cast<int>(data[uv+1])-128;
    const auto offset=(static_cast<std::size_t>(y)*width+x)*4;
    image.pixels[offset]=clamp((298*luminance+516*u+128)>>8); image.pixels[offset+1]=clamp((298*luminance-100*u-208*v+128)>>8); image.pixels[offset+2]=clamp((298*luminance+409*v+128)>>8); image.pixels[offset+3]=255;
  }
  return image;
}
class Decoder {
public:
  Decoder(std::uint32_t width, std::uint32_t height, std::function<void(Nv12Image)> output) : output_(std::move(output)) {
    check(CoCreateInstance(CLSID_CMSH264DecoderMFT,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&decoder_)),"Create H.264 decoder");
    ComPtr<ICodecAPI> codec;
    if(SUCCEEDED(decoder_.As(&codec))) { VARIANT low{}; low.vt=VT_UI4; low.ulVal=1; (void)codec->SetValue(&CODECAPI_AVLowLatencyMode,&low); }
    ComPtr<IMFMediaType> input; check(MFCreateMediaType(&input),"Create video type");
    check(input->SetGUID(MF_MT_MAJOR_TYPE,MFMediaType_Video),"Set video type"); check(input->SetGUID(MF_MT_SUBTYPE,MFVideoFormat_H264),"Set H.264 type");
    check(MFSetAttributeSize(input.Get(), MF_MT_FRAME_SIZE, width, height), "Set H.264 frame size");
    check(decoder_->SetInputType(0,input.Get(),0),"Configure H.264 decoder");
    configureOutput();
    decoder_->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,0); decoder_->ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM,0);
  }
  void decode(const Frame& frame) {
    ComPtr<IMFSample> sample; ComPtr<IMFMediaBuffer> buffer;
    check(MFCreateSample(&sample),"Create sample"); check(MFCreateMemoryBuffer(static_cast<DWORD>(frame.data.size()),&buffer),"Allocate sample");
    BYTE* target=nullptr; check(buffer->Lock(&target,nullptr,nullptr),"Lock sample"); std::copy(frame.data.begin(),frame.data.end(),target); buffer->Unlock();
    buffer->SetCurrentLength(static_cast<DWORD>(frame.data.size())); sample->AddBuffer(buffer.Get()); sample->SetSampleTime(time_); sample->SetSampleDuration(166667); time_+=166667;
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
      BYTE* data=nullptr; DWORD size=0; check(buffer->Lock(&data,nullptr,&size),"Lock decoded frame");
      if(static_cast<std::size_t>(size)<static_cast<std::size_t>(stride)*height*3/2) { buffer->Unlock(); throw std::runtime_error("Truncated decoded image"); }
      Nv12Image image{Bytes(data,data+static_cast<std::size_t>(stride)*height*3/2),width,height,static_cast<std::uint32_t>(stride)};
      buffer->Unlock(); output_(std::move(image)); if(!reported_) { reported_=true; std::cout<<"SNV2_FIRST_FRAME decoded=1"<<std::endl; }
    }
  }
  ComPtr<IMFTransform> decoder_; ComPtr<IMFMediaType> type_; std::function<void(Nv12Image)> output_; LONGLONG time_=0; bool reported_=false;
};
struct WindowState {
  bool viewOnly=false;
  std::function<void(Input)> sendInput; std::mutex mutex; std::shared_ptr<const Image> image;
  bool released=false; RECT video{};
  WindowPresenter presenter;
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
  if(state->viewOnly && ((message>=WM_KEYFIRST && message<=WM_KEYLAST) || (message>=WM_MOUSEFIRST && message<=WM_MOUSELAST))) return DefWindowProcW(window,message,w,l);
  if(message==WM_CLOSE) { DestroyWindow(window); return 0; }
  if(message==WM_DESTROY) { PostQuitMessage(0); return 0; }
  if(message==WM_ERASEBKGND) return 1;
  if(message==WM_PAINT) {
    PAINTSTRUCT paint{}; HDC dc=BeginPaint(window,&paint); RECT area{}; GetClientRect(window,&area);
    std::shared_ptr<const Image> snapshot;
    { std::lock_guard lock(state->mutex); snapshot=state->image; }
    const auto* pixels=snapshot && !snapshot->pixels.empty() ? snapshot->pixels.data() : nullptr;
    if(!state->presenter.paint(dc,area.right,area.bottom,pixels,snapshot ? snapshot->width : 0,snapshot ? snapshot->height : 0,state->video))
      std::cerr<<"Unable to paint the remote desktop frame"<<std::endl;
    EndPaint(window,&paint); return 0;
  }
  // Windows runs its own modal loop while a window is dragged or resized.
  // Its loop cannot wait on frameReady, so keep repainting the latest frame
  // with a short-lived timer until normal event processing resumes.
  if(message==WM_ENTERSIZEMOVE) { SetTimer(window,1,16,nullptr); return 0; }
  if(message==WM_EXITSIZEMOVE) { KillTimer(window,1); InvalidateRect(window,nullptr,FALSE); return 0; }
  if(message==WM_TIMER && w==1) { if(WaitForSingleObject(state->frameReady,0)==WAIT_OBJECT_0) InvalidateRect(window,nullptr,FALSE); return 0; }
  if(message==WM_SIZE) { InvalidateRect(window,nullptr,FALSE); return 0; }
  if(message==WM_KILLFOCUS) { state->sendInput(Input{}); state->released=true; if(GetCapture()==window) ReleaseCapture(); return 0; }
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
  require(windowsHid(VK_RETURN,(0x1cLL<<16)|(1LL<<24))==88,"Keypad Enter lost its extended scan code");
  SendMessageW(window,WM_KILLFOCUS,0,0);
  require(state.released && events.back().kind==Input::Reset,"Focus loss did not release remote keys/buttons");
  state.viewOnly=true; const auto before=events.size();
  SendMessageW(window,WM_KEYDOWN,'A',0); SendMessageW(window,WM_LBUTTONDOWN,0,MAKELPARAM(100,50));
  require(events.size()==before,"View-only window forwarded input");
  std::cout<<"Windows input: resume click, keyboard fallback, keypad, focus reset and view-only enforcement passed\n";
}
void testWindowsMacFixture(const char* path) {
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
  DecodeWorker worker([&](Frame frame) {
    if(!runtime) runtime=std::make_unique<Runtime>();
    if(!decoder) decoder=std::make_unique<Decoder>(width,height,[&](Nv12Image image) { conversion.submit(std::move(image)); ++decoded; changed.notify_all(); });
    decoder->decode(frame);
  },[&] { decoder.reset(); runtime.reset(); },[&] { ++keyframes; changed.notify_all(); });
  for(unsigned i=0;i<count;++i) {
    const auto size=read(); const auto key=file.get();
    if(size==0 || size>4*1024*1024 || (key!=0 && key!=1)) throw std::runtime_error("Invalid Mac access unit");
    Frame frame{Bytes(size),width,height,key==1}; file.read(reinterpret_cast<char*>(frame.data.data()),size);
    if(!file) throw std::runtime_error("Truncated Mac access unit");
    worker.submit(std::move(frame));
    std::unique_lock lock(mutex);
    if(!changed.wait_for(lock,std::chrono::seconds(3),[&] { return decoded>i || worker.failures()>0; }) || worker.failures()>0)
      throw std::runtime_error("Windows stalled decoding Mac frame "+std::to_string(i)+": "+worker.error());
    if(i==0 && !changed.wait_for(lock,std::chrono::seconds(3),[&] { return latest!=nullptr || conversionError; }))
      throw std::runtime_error("Windows did not convert the initial Mac picture");
  }
  {
    std::unique_lock lock(mutex);
    const auto blue=[&] { return latest && latest->width==width && latest->height==height && latest->pixels[0]>150 && latest->pixels[1]<80 && latest->pixels[2]<80; };
    if(!changed.wait_for(lock,std::chrono::seconds(3),[&] { return blue() || conversionError; }) || !blue()) throw std::runtime_error("Windows picture did not advance to the final blue Mac frame");
  }
  worker.stop(); conversion.stop();
  if(decoded!=45 || converted<2 || keyframes!=0) throw std::runtime_error("Mac stream lost decoder progress or reset references under conversion load");
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
    SharingNotice notice(peer,options.sharing);
    peer.onInput=[&](Input event) { injector.apply(event); }; HostOutputStop shutdown{peer}; peer.start();
    FrameBgra cached; bool hasCached=false, force=false;
    auto generation=display.generation();
    const auto mediaStart=Clock::now();
    auto bitrate=options.bitrate;
    while(peer.running()) {
      const auto began=Clock::now();
      peer.pumpControl();
      notice.pump(); if(!peer.running()) break;
      const auto target=peer.bitrate();
      if(target!=bitrate && encoder->setBitrate(target)) bitrate=target;
      force=peer.takeKeyframeRequest() || force;
      if(peer.ready()) {
        FrameBgra frame;
        const bool captured=display.captureFrame(frame, options.ultraLowLatency ? 2 : 16);
        if(display.recovering()) { std::this_thread::sleep_for(std::chrono::milliseconds(10)); continue; }
        if(generation!=display.generation()) {
          if(encoding.gpuDevice && encoding.gpuDevice!=display.gpuDevice()) {
            encoding.gpuDevice=display.gpuDevice();
            encoder=std::make_unique<MfVideoPacketEncoder>(options.width,options.height,encoding);
            bitrate=options.bitrate;
            display.setGpuCapture(encoder->usingGpuInput());
          }
          generation=display.generation(); hasCached=false; cached=FrameBgra{}; force=true;
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
  AudioPlayer audio(options.audio);
  if(options.audio) peer.onAudio=[&](AudioSamples samples) { audio.submit(std::move(samples)); };
  // Decoder COM objects belong to the decode worker, not the socket/UI threads.
  std::atomic<bool> conversionFailed{false};
  std::atomic<std::uint64_t> decodedFrames{0},convertedFrames{0};
  LatestFrameWorker<Nv12Image> conversion([&](Nv12Image pixels) {
    try {
      auto snapshot=std::make_shared<const Image>(convertNv12(pixels));
      { std::lock_guard lock(state.mutex); state.image=std::move(snapshot); }
      ++convertedFrames;
      SetEvent(state.frameReady);
    } catch(const std::exception& error) { std::cerr<<"SNV2_COLOR_CONVERSION failed: "<<error.what()<<std::endl; conversionFailed=true; }
  });
  std::unique_ptr<Runtime> runtime; std::unique_ptr<Decoder> decoder;
  DecodeWorker decode([&](Frame frame) {
    if(!runtime) runtime=std::make_unique<Runtime>();
    if(!decoder) decoder=std::make_unique<Decoder>(frame.width,frame.height,[&](Nv12Image image) { ++decodedFrames; conversion.submit(std::move(image)); });
    decoder->decode(frame);
  },[&] { decoder.reset(); runtime.reset(); conversion.clear(); },[&] { peer.requestKeyframe(); });
  peer.onFrame=[&](Frame frame) { decode.submit(std::move(frame)); };
  PeerStop shutdown{peer};
  const auto instance=GetModuleHandleW(nullptr); WNDCLASSW klass{}; klass.lpfnWndProc=windowProc; klass.hInstance=instance; klass.lpszClassName=L"SanserRemoteDesktop"; klass.hCursor=LoadCursor(nullptr,IDC_ARROW); RegisterClassW(&klass);
  HWND window=CreateWindowExW(0,klass.lpszClassName,options.input ? L"Sanser · Remote desktop · Ctrl+Alt+Escape releases input" : L"Sanser · View only",WS_OVERLAPPEDWINDOW,CW_USEDEFAULT,CW_USEDEFAULT,1280,760,nullptr,nullptr,instance,&state);
  if(!window) throw std::runtime_error("Unable to create remote desktop window"); ShowWindow(window,SW_SHOW); SetForegroundWindow(window); SetFocus(window); peer.start();
  MSG message{}; bool open=true;
  auto lastProgress=Clock::now();
  while(open && peer.running() && decode.failures()<10 && !conversionFailed) {
    if(std::getenv("SANSER_TELEMETRY") && Clock::now()-lastProgress>=std::chrono::seconds(1)) {
      std::cout<<"SNV2_VIDEO_HEALTH decoded="<<decodedFrames.load()<<" converted="<<convertedFrames.load()
        <<" encodedQueueDrops="<<decode.dropped()<<" decodeErrors="<<decode.failures()<<std::endl;
      lastProgress=Clock::now();
    }
    const auto wake=MsgWaitForMultipleObjectsEx(1,&state.frameReady,100,QS_ALLINPUT,MWMO_INPUTAVAILABLE);
    if(wake==WAIT_FAILED) { peer.stop(); DestroyWindow(window); throw std::runtime_error("Remote window event wait failed"); }
    if(wake==WAIT_OBJECT_0) InvalidateRect(window,nullptr,FALSE);
    while(PeekMessageW(&message,nullptr,0,0,PM_REMOVE)) {
      if(message.message==WM_QUIT) { open=false; break; }
      TranslateMessage(&message); DispatchMessageW(&message);
    }
  }
  peer.stop(); decode.stop(); conversion.stop(); if(IsWindow(window)) DestroyWindow(window);
  if(decode.failures()>=10) throw std::runtime_error("Unable to decode the remote video: "+decode.error());
  if(conversionFailed) throw std::runtime_error("Unable to convert the remote video frame");
  return 0;
}
}
