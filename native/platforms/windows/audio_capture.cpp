#include <windows.h>
#include <audioclient.h>
#include <audioclientactivationparams.h>
#include <mmdeviceapi.h>
#include <ks.h>
#include <ksmedia.h>
#include <wrl/client.h>
#include <wrl/implements.h>
#include "audio_capture.h"
#include "audio_capture_pcm.h"
#include "opus_audio.h"
#include <iostream>
#include <condition_variable>
#include <memory>
#include <mutex>
#include <stdexcept>
using Microsoft::WRL::ComPtr;
namespace sanser::desktop {
using Clock=std::chrono::steady_clock;
namespace {
void check(HRESULT hr,const char* operation) { if(FAILED(hr)) throw std::runtime_error(operation); }
struct Com { Com() { check(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"Audio capture COM"); } ~Com() { CoUninitialize(); } };
struct Event { HANDLE value=CreateEventW(nullptr,FALSE,FALSE,nullptr); ~Event() { if(value) CloseHandle(value); } };
struct Format { WAVEFORMATEX* value=nullptr; ~Format() { CoTaskMemFree(value); } };
std::wstring identity(IMMDevice* device) {
  LPWSTR raw=nullptr; check(device->GetId(&raw),"Audio capture endpoint ID");
  std::wstring id(raw); CoTaskMemFree(raw); return id;
}
// The operation may finish after a bounded wait was cancelled. Its agile COM
// completion owns every byte/result it accesses and never points into the
// AudioCapture, session, sink or stack frame. Windows retains this callback
// until activation completes; the caller must not manually delete it.
class ProcessActivation final : public Microsoft::WRL::RuntimeClass<
    Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,
    Microsoft::WRL::FtmBase,IActivateAudioInterfaceCompletionHandler> {
public:
  ProcessActivation() {
    parameters.ActivationType=AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
    parameters.ProcessLoopbackParams.TargetProcessId=GetCurrentProcessId();
    parameters.ProcessLoopbackParams.ProcessLoopbackMode=PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE;
    variant.vt=VT_BLOB;
    variant.blob.cbSize=sizeof(parameters);
    variant.blob.pBlobData=reinterpret_cast<BYTE*>(&parameters);
  }
  STDMETHOD(ActivateCompleted)(IActivateAudioInterfaceAsyncOperation* operation) override {
    HRESULT activated=E_UNEXPECTED;
    ComPtr<IUnknown> object;
    auto error=operation ? operation->GetActivateResult(&activated,&object) : E_POINTER;
    ComPtr<IAudioClient> result;
    if(SUCCEEDED(error)) error=activated;
    if(SUCCEEDED(error)) error=object ? object.As(&result) : E_NOINTERFACE;
    {
      std::lock_guard lock(mutex_);
      if(!abandoned_) { result_=error; client_=std::move(result); done_=true; }
    }
    wake_.notify_all();
    return S_OK;
  }
  ComPtr<IAudioClient> wait(const std::atomic<bool>& running) {
    std::unique_lock lock(mutex_);
    const auto deadline=Clock::now()+std::chrono::milliseconds(1500);
    while(running && !done_ && Clock::now()<deadline)
      wake_.wait_for(lock,std::chrono::milliseconds(20));
    if(!running || !done_ || FAILED(result_)) {
      abandoned_=true;
      client_.Reset();
      return {};
    }
    return client_;
  }
  AUDIOCLIENT_ACTIVATION_PARAMS parameters{};
  PROPVARIANT variant{};
private:
  std::mutex mutex_;
  std::condition_variable wake_;
  bool done_=false,abandoned_=false;
  HRESULT result_=E_PENDING;
  ComPtr<IAudioClient> client_;
};
ComPtr<IAudioClient> activateProcessLoopback(const std::atomic<bool>& running) {
  // Keep the system module loaded until process exit: an activation cancelled
  // by our bounded wait may still complete inside Windows later. No additional
  // import-library requirement or new unconditional startup dependency.
  static HMODULE module=LoadLibraryExW(L"Mmdevapi.dll",nullptr,LOAD_LIBRARY_SEARCH_SYSTEM32);
  using Activate=HRESULT(WINAPI*)(LPCWSTR,REFIID,PROPVARIANT*,IActivateAudioInterfaceCompletionHandler*,IActivateAudioInterfaceAsyncOperation**);
  static auto activate=module ? reinterpret_cast<Activate>(GetProcAddress(module,"ActivateAudioInterfaceAsync")) : nullptr;
  if(!activate || !running) return {};
  auto completion=Microsoft::WRL::Make<ProcessActivation>();
  if(!completion) return {};
  ComPtr<IActivateAudioInterfaceAsyncOperation> operation;
  const auto result=activate(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,__uuidof(IAudioClient),&completion->variant,completion.Get(),&operation);
  if(FAILED(result)) return {};
  return completion->wait(running);
}
struct Stream {
  ComPtr<IAudioClient> client;
  ComPtr<IAudioCaptureClient> capture;
  ComPtr<IMMDeviceEnumerator> enumerator;
  Format format;
  WAVEFORMATEX fixed{};
  Event event;
  std::wstring selected;
  bool excluded=false;
  ~Stream() { if(client) client->Stop(); }
  const WAVEFORMATEX& wave() const { return format.value ? *format.value : fixed; }
  void initialize(DWORD flags,REFERENCE_TIME duration) {
    if(!event.value) throw std::runtime_error("Audio capture event");
    check(client->Initialize(AUDCLNT_SHAREMODE_SHARED,flags,duration,0,&wave(),nullptr),"Initialize loopback capture");
    check(client->SetEventHandle(event.value),"Audio capture event handle");
    check(client->GetService(IID_PPV_ARGS(&capture)),"Audio capture service");
  }
};
std::unique_ptr<Stream> processStream(const std::atomic<bool>& running) {
  auto stream=std::make_unique<Stream>();
  stream->client=activateProcessLoopback(running);
  if(!stream->client) return {};
  // Virtual process-loopback GetMixFormat is unsupported on some Windows
  // builds. The documented shared-mode process API accepts a PCM format with
  // AUTOCONVERTPCM; use Sanser's exact wire rate rather than querying a device.
  stream->fixed.wFormatTag=WAVE_FORMAT_PCM;
  stream->fixed.nChannels=audioChannels;
  stream->fixed.nSamplesPerSec=audioRate;
  stream->fixed.wBitsPerSample=16;
  stream->fixed.nBlockAlign=audioChannels*sizeof(std::int16_t);
  stream->fixed.nAvgBytesPerSec=audioRate*stream->fixed.nBlockAlign;
  stream->initialize(AUDCLNT_STREAMFLAGS_LOOPBACK|AUDCLNT_STREAMFLAGS_EVENTCALLBACK|AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,0);
  stream->excluded=true;
  return stream;
}
std::unique_ptr<Stream> endpointStream() {
  auto stream=std::make_unique<Stream>();
  check(CoCreateInstance(__uuidof(MMDeviceEnumerator),nullptr,CLSCTX_ALL,IID_PPV_ARGS(&stream->enumerator)),"Audio capture enumerator");
  ComPtr<IMMDevice> device;
  check(stream->enumerator->GetDefaultAudioEndpoint(eRender,eMultimedia,&device),"Default audio capture endpoint");
  stream->selected=identity(device.Get());
  check(device->Activate(__uuidof(IAudioClient),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(stream->client.GetAddressOf())),"Activate audio capture");
  check(stream->client->GetMixFormat(&stream->format.value),"Audio capture mix format");
  stream->initialize(AUDCLNT_STREAMFLAGS_LOOPBACK|AUDCLNT_STREAMFLAGS_EVENTCALLBACK,200000);
  return stream;
}
}
AudioCapture::AudioCapture(bool enabled,std::function<void(const AudioSamples&)> sink,
                           bool excludeSelf,std::function<void(bool)> exclusionStatus)
  :sink_(std::move(sink)),excludeSelf_(excludeSelf),exclusionStatus_(std::move(exclusionStatus)) {
  if(enabled) worker_=std::thread([this] { run(); });
}
AudioCapture::~AudioCapture() { running_=false; if(worker_.joinable()) worker_.join(); }
void AudioCapture::reportExclusion(bool excluded) noexcept {
  if(exclusionStatus_) try { exclusionStatus_(excluded); } catch(...) { }
}
void AudioCapture::run() {
  reportExclusion(false);
  bool tryExclusion=excludeSelf_;
  while(running_) {
    try {
      Com com;
      std::unique_ptr<Stream> stream;
      if(tryExclusion) {
        try { stream=processStream(running_); } catch(...) { stream.reset(); }
        if(!stream) {
          // At most one timed-out activation is left pending. Do not pile up
          // abandoned COM operations on an unsupported/unresponsive OS.
          tryExclusion=false;
          reportExclusion(false);
          std::cerr<<"SNV2_AUDIO_CAPTURE processExclusion=0 fallback=endpoint-loopback"<<std::endl;
        }
      }
      if(!running_) break;
      if(!stream) stream=endpointStream();
      const auto& wave=stream->wave();
      bool floating=wave.wFormatTag==WAVE_FORMAT_IEEE_FLOAT,pcm=wave.wFormatTag==WAVE_FORMAT_PCM;
      if(wave.wFormatTag==WAVE_FORMAT_EXTENSIBLE && wave.cbSize>=sizeof(WAVEFORMATEXTENSIBLE)-sizeof(WAVEFORMATEX)) {
        const auto* extended=reinterpret_cast<const WAVEFORMATEXTENSIBLE*>(&wave);
        floating=IsEqualGUID(extended->SubFormat,KSDATAFORMAT_SUBTYPE_IEEE_FLOAT);
        pcm=IsEqualGUID(extended->SubFormat,KSDATAFORMAT_SUBTYPE_PCM);
      }
      if((!floating && !pcm) || wave.nChannels<1 || wave.nChannels>8 || wave.nSamplesPerSec<8000 || wave.nSamplesPerSec>192000 ||
         (floating && wave.wBitsPerSample!=32) || (wave.wBitsPerSample!=16 && wave.wBitsPerSample!=24 && wave.wBitsPerSample!=32) ||
         wave.nBlockAlign!=wave.nChannels*(wave.wBitsPerSample/8)) throw std::runtime_error("Unsupported audio capture mix format");
      sanser::audio::Packetizer packetizer(sink_);
      std::vector<float> stereo;
      if(!running_) break;
      auto start=stream->client->Start();
      if(FAILED(start) && stream->excluded) {
        // Initialization may succeed while Start reports an unsupported
        // virtual path. Fall back safely on the next loop rather than retrying
        // a path that never actually captured process-excluded audio.
        tryExclusion=false;
      }
      check(start,"Start audio capture");
      reportExclusion(stream->excluded);
      std::cout<<"SNV2_AUDIO_CAPTURE started=1 sourceRate="<<wave.nSamplesPerSec<<" channels="<<wave.nChannels
               <<" processExclusion="<<(stream->excluded ? 1 : 0)<<std::endl;
      auto deviceCheck=Clock::now();
      while(running_) {
        if(WaitForSingleObject(stream->event.value,20)==WAIT_FAILED) throw std::runtime_error("Audio capture wait");
        UINT32 available=0; check(stream->capture->GetNextPacketSize(&available),"Audio capture packet size");
        while(available && running_) {
          BYTE* data=nullptr; UINT32 frames=0; DWORD flags=0;
          check(stream->capture->GetBuffer(&data,&frames,&flags,nullptr,nullptr),"Audio capture buffer");
          const bool silent=(flags&AUDCLNT_BUFFERFLAGS_SILENT)!=0;
          const bool valid=sanser::audio::captureStereo(
            data ? std::span<const std::uint8_t>(data,static_cast<std::size_t>(frames)*wave.nBlockAlign) : std::span<const std::uint8_t>{},
            frames,wave.nChannels,wave.wBitsPerSample,floating,silent,stereo);
          check(stream->capture->ReleaseBuffer(frames),"Release audio capture buffer");
          if(!valid) throw std::runtime_error("Invalid audio capture buffer");
          if(flags&AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY) packetizer.reset();
          if(!packetizer.push(stereo,wave.nSamplesPerSec)) throw std::runtime_error("Audio capture resampling failed");
          check(stream->capture->GetNextPacketSize(&available),"Next audio capture packet");
        }
        if(!stream->excluded && Clock::now()-deviceCheck>std::chrono::seconds(2)) {
          ComPtr<IMMDevice> current;
          check(stream->enumerator->GetDefaultAudioEndpoint(eRender,eMultimedia,&current),"Current audio capture endpoint");
          if(identity(current.Get())!=stream->selected) break;
          deviceCheck=Clock::now();
        }
      }
    } catch(const std::exception& error) {
      reportExclusion(false);
      std::cerr<<"SNV2_AUDIO_CAPTURE unavailable: "<<error.what()<<"; retrying without stopping video"<<std::endl;
      for(unsigned i=0;i<50 && running_;++i) std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
  }
  reportExclusion(false);
}
#if defined(SANSER_CODEC_TEST)
// Exercise COM completion and cancellation lifetime without activating an
// audio interface, opening a device, playing audio, or recording anything.
bool testWindowsAudioCaptureExclusion() {
  class FailedOperation final : public Microsoft::WRL::RuntimeClass<
      Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,
      IActivateAudioInterfaceAsyncOperation> {
  public:
    STDMETHOD(GetActivateResult)(HRESULT* result,IUnknown** object) override {
      if(!result || !object) return E_POINTER;
      *result=E_NOTIMPL; *object=nullptr;
      return S_OK;
    }
  };
  Com com;
  auto completion=Microsoft::WRL::Make<ProcessActivation>();
  auto operation=Microsoft::WRL::Make<FailedOperation>();
  if(!completion || !operation || completion->parameters.ActivationType!=AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK ||
     completion->parameters.ProcessLoopbackParams.TargetProcessId!=GetCurrentProcessId() ||
     completion->parameters.ProcessLoopbackParams.ProcessLoopbackMode!=PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE ||
     completion->variant.blob.pBlobData!=reinterpret_cast<BYTE*>(&completion->parameters)) return false;
  std::atomic<bool> running{true};
  completion->ActivateCompleted(operation.Get());
  if(completion->wait(running)) return false;
  auto pending=Microsoft::WRL::Make<ProcessActivation>();
  if(!pending) return false;
  running=false;
  const auto before=Clock::now();
  if(pending->wait(running) || Clock::now()-before>std::chrono::milliseconds(100)) return false;
  // Simulate Windows retaining the completion after the capture caller has
  // gone away, then completing late. The callback may access only its state.
  auto late=pending;
  pending.Reset();
  if(FAILED(late->ActivateCompleted(operation.Get()))) return false;
  return !late->wait(running);
}
#endif
}
