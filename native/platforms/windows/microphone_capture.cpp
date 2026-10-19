#include <windows.h>
#include <audioclient.h>
#include <mmdeviceapi.h>
#include <ks.h>
#include <ksmedia.h>
#include <wrl/client.h>
#include "microphone_capture.h"
#include "audio_capture_pcm.h"
#include "opus_audio.h"
#include <atomic>
#include <condition_variable>
#include <mutex>
#include <stdexcept>
#include <thread>

using Microsoft::WRL::ComPtr;
namespace sanser::desktop {
namespace {
using Clock=std::chrono::steady_clock;
struct Failure : std::runtime_error {
  HRESULT code;
  Failure(HRESULT value,const char* operation):std::runtime_error(operation),code(value) {}
};
void check(HRESULT hr,const char* operation) { if(FAILED(hr)) throw Failure(hr,operation); }
struct Com {
  Com() { check(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"Microphone COM"); }
  ~Com() { CoUninitialize(); }
};
struct Event { HANDLE value=CreateEventW(nullptr,FALSE,FALSE,nullptr); ~Event() { if(value) CloseHandle(value); } };
struct Format { WAVEFORMATEX* value=nullptr; ~Format() { CoTaskMemFree(value); } };
struct Stop { IAudioClient* client; ~Stop() { client->Stop(); } };
std::wstring identity(IMMDevice* device) {
  LPWSTR raw=nullptr; check(device->GetId(&raw),"Microphone endpoint ID");
  std::wstring result(raw); CoTaskMemFree(raw); return result;
}
}
struct MicrophoneCapture::State {
  std::function<void(const AudioSamples&)> sink;
  std::function<void(std::string)> status;
  std::atomic<bool> running{true};
  std::mutex mutex;
  std::condition_variable wake;
  std::thread worker;
  State(std::function<void(const AudioSamples&)> output,std::function<void(std::string)> report)
    :sink(std::move(output)),status(std::move(report)),worker([this] { run(); }) {}
  ~State() { running=false; wake.notify_all(); if(worker.joinable()) worker.join(); }
  void report(const char* value) noexcept { if(status) try { status(value); } catch(...) { } }
  void capture() {
    Com com;
    ComPtr<IMMDeviceEnumerator> enumerator;
    check(CoCreateInstance(__uuidof(MMDeviceEnumerator),nullptr,CLSCTX_ALL,IID_PPV_ARGS(&enumerator)),"Microphone enumerator");
    ComPtr<IMMDevice> device;
    const auto deviceResult=enumerator->GetDefaultAudioEndpoint(eCapture,eCommunications,&device);
    if(deviceResult==E_NOTFOUND) { report("no-device"); return; }
    check(deviceResult,"Default microphone endpoint");
    const auto selected=identity(device.Get());
    ComPtr<IAudioClient> client;
    check(device->Activate(__uuidof(IAudioClient),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(client.GetAddressOf())),"Activate microphone");
    Format format;
    check(client->GetMixFormat(&format.value),"Microphone mix format");
    const auto& wave=*format.value;
    bool floating=wave.wFormatTag==WAVE_FORMAT_IEEE_FLOAT,pcm=wave.wFormatTag==WAVE_FORMAT_PCM;
    if(wave.wFormatTag==WAVE_FORMAT_EXTENSIBLE && wave.cbSize>=sizeof(WAVEFORMATEXTENSIBLE)-sizeof(WAVEFORMATEX)) {
      const auto* extended=reinterpret_cast<const WAVEFORMATEXTENSIBLE*>(format.value);
      floating=IsEqualGUID(extended->SubFormat,KSDATAFORMAT_SUBTYPE_IEEE_FLOAT);
      pcm=IsEqualGUID(extended->SubFormat,KSDATAFORMAT_SUBTYPE_PCM);
    }
    if((!floating && !pcm) || wave.nChannels<1 || wave.nChannels>8 || wave.nSamplesPerSec<8000 || wave.nSamplesPerSec>192000 ||
       (floating && wave.wBitsPerSample!=32) || (wave.wBitsPerSample!=16 && wave.wBitsPerSample!=24 && wave.wBitsPerSample!=32) ||
       wave.nBlockAlign!=wave.nChannels*(wave.wBitsPerSample/8)) throw std::runtime_error("Unsupported microphone mix format");
    // Capture endpoint, never AUDCLNT_STREAMFLAGS_LOOPBACK: this stream must
    // contain only the selected microphone, not remote voice/system playback.
    check(client->Initialize(AUDCLNT_SHAREMODE_SHARED,AUDCLNT_STREAMFLAGS_EVENTCALLBACK,200000,0,format.value,nullptr),"Initialize microphone");
    Event event;
    if(!event.value) throw std::runtime_error("Microphone event");
    check(client->SetEventHandle(event.value),"Microphone event handle");
    ComPtr<IAudioCaptureClient> capture;
    check(client->GetService(IID_PPV_ARGS(&capture)),"Microphone capture service");
    sanser::audio::Packetizer packetizer(sink);
    std::vector<float> stereo;
    if(!running) return;
    check(client->Start(),"Start microphone"); Stop stop{client.Get()};
    report("running");
    auto deviceCheck=Clock::now();
    LARGE_INTEGER frequency{};
    if(!QueryPerformanceFrequency(&frequency) || frequency.QuadPart<=0) throw std::runtime_error("Microphone clock");
    while(running) {
      if(WaitForSingleObject(event.value,20)==WAIT_FAILED) throw std::runtime_error("Microphone wait");
      if(!running) break;
      UINT32 available=0; check(capture->GetNextPacketSize(&available),"Microphone packet size");
      while(available && running) {
        BYTE* data=nullptr; UINT32 frames=0; DWORD flags=0; UINT64 capturedAt=0;
        check(capture->GetBuffer(&data,&frames,&flags,nullptr,&capturedAt),"Microphone buffer");
        LARGE_INTEGER current{};
        QueryPerformanceCounter(&current);
        // WASAPI reports QPC in 100 ns units. If scheduling delayed capture,
        // discard old microphone PCM instead of replaying a speech backlog.
        const auto now=static_cast<UINT64>(static_cast<long double>(current.QuadPart)*10000000.0L/frequency.QuadPart);
        const bool stale=!(flags&AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR) && capturedAt && now>capturedAt && now-capturedAt>400000;
        if(stale) {
          check(capture->ReleaseBuffer(frames),"Release stale microphone buffer");
          packetizer.reset();
          check(capture->GetNextPacketSize(&available),"Next microphone packet");
          continue;
        }
        const bool silent=(flags&AUDCLNT_BUFFERFLAGS_SILENT)!=0;
        const bool valid=sanser::audio::captureStereo(
          data ? std::span<const std::uint8_t>(data,static_cast<std::size_t>(frames)*wave.nBlockAlign) : std::span<const std::uint8_t>{},
          frames,wave.nChannels,wave.wBitsPerSample,floating,silent,stereo);
        check(capture->ReleaseBuffer(frames),"Release microphone buffer");
        if(!valid) throw std::runtime_error("Invalid microphone buffer");
        if(flags&AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY) packetizer.reset();
        if(!packetizer.push(stereo,wave.nSamplesPerSec)) throw std::runtime_error("Microphone resampling failed");
        check(capture->GetNextPacketSize(&available),"Next microphone packet");
      }
      if(Clock::now()-deviceCheck>std::chrono::seconds(2)) {
        ComPtr<IMMDevice> current;
        check(enumerator->GetDefaultAudioEndpoint(eCapture,eCommunications,&current),"Current microphone endpoint");
        if(identity(current.Get())!=selected) { report("device-changed"); break; }
        deviceCheck=Clock::now();
      }
    }
  }
  void run() noexcept {
    report("starting");
    while(running) {
      try { capture(); }
      catch(const Failure& error) {
        if(error.code==E_ACCESSDENIED) { report("permission-denied"); return; }
        report("unavailable");
      } catch(...) { report("unavailable"); }
      std::unique_lock lock(mutex);
      wake.wait_for(lock,std::chrono::seconds(1),[this] { return !running; });
    }
    report("stopped");
  }
};
MicrophoneCapture::MicrophoneCapture(bool enabled,std::function<void(const AudioSamples&)> sink,
                                   std::function<void(std::string)> status) {
  if(enabled) state_=std::make_unique<State>(std::move(sink),std::move(status));
}
MicrophoneCapture::~MicrophoneCapture()=default;
}
