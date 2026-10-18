#include <windows.h>
#include <audioclient.h>
#include <mmdeviceapi.h>
#include <ks.h>
#include <ksmedia.h>
#include <wrl/client.h>
#include "audio_capture.h"
#include "audio_capture_pcm.h"
#include "opus_audio.h"
#include <iostream>
#include <stdexcept>
using Microsoft::WRL::ComPtr;
namespace sanser::desktop {
using Clock=std::chrono::steady_clock;
namespace {
void check(HRESULT hr,const char* operation) { if(FAILED(hr)) throw std::runtime_error(operation); }
struct Com { Com() { check(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"Audio capture COM"); } ~Com() { CoUninitialize(); } };
struct Event { HANDLE value=CreateEventW(nullptr,FALSE,FALSE,nullptr); ~Event() { if(value) CloseHandle(value); } };
struct Format { WAVEFORMATEX* value=nullptr; ~Format() { CoTaskMemFree(value); } };
struct Stop { IAudioClient* client; ~Stop() { client->Stop(); } };
std::wstring identity(IMMDevice* device) {
  LPWSTR raw=nullptr; check(device->GetId(&raw),"Audio capture endpoint ID");
  std::wstring id(raw); CoTaskMemFree(raw); return id;
}
}
AudioCapture::AudioCapture(bool enabled,std::function<void(const AudioSamples&)> sink):sink_(std::move(sink)) {
  if(enabled) worker_=std::thread([this] { run(); });
}
AudioCapture::~AudioCapture() { running_=false; if(worker_.joinable()) worker_.join(); }
void AudioCapture::run() {
  while(running_) {
    try {
      Com com;
      ComPtr<IMMDeviceEnumerator> enumerator;
      check(CoCreateInstance(__uuidof(MMDeviceEnumerator),nullptr,CLSCTX_ALL,IID_PPV_ARGS(&enumerator)),"Audio capture enumerator");
      ComPtr<IMMDevice> device;
      check(enumerator->GetDefaultAudioEndpoint(eRender,eMultimedia,&device),"Default audio capture endpoint");
      const auto selected=identity(device.Get());
      ComPtr<IAudioClient> client;
      check(device->Activate(__uuidof(IAudioClient),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(client.GetAddressOf())),"Activate audio capture");
      Format format;
      check(client->GetMixFormat(&format.value),"Audio capture mix format");
      const auto& wave=*format.value;
      bool floating=wave.wFormatTag==WAVE_FORMAT_IEEE_FLOAT,pcm=wave.wFormatTag==WAVE_FORMAT_PCM;
      if(wave.wFormatTag==WAVE_FORMAT_EXTENSIBLE && wave.cbSize>=sizeof(WAVEFORMATEXTENSIBLE)-sizeof(WAVEFORMATEX)) {
        const auto* extended=reinterpret_cast<const WAVEFORMATEXTENSIBLE*>(format.value);
        floating=IsEqualGUID(extended->SubFormat,KSDATAFORMAT_SUBTYPE_IEEE_FLOAT);
        pcm=IsEqualGUID(extended->SubFormat,KSDATAFORMAT_SUBTYPE_PCM);
      }
      if((!floating && !pcm) || wave.nChannels<1 || wave.nChannels>8 || wave.nSamplesPerSec<8000 || wave.nSamplesPerSec>192000 ||
         (floating && wave.wBitsPerSample!=32) || (wave.wBitsPerSample!=16 && wave.wBitsPerSample!=24 && wave.wBitsPerSample!=32) ||
         wave.nBlockAlign!=wave.nChannels*(wave.wBitsPerSample/8)) throw std::runtime_error("Unsupported audio capture mix format");
      check(client->Initialize(AUDCLNT_SHAREMODE_SHARED,AUDCLNT_STREAMFLAGS_LOOPBACK|AUDCLNT_STREAMFLAGS_EVENTCALLBACK,200000,0,format.value,nullptr),"Initialize loopback capture");
      Event event; if(!event.value) throw std::runtime_error("Audio capture event");
      check(client->SetEventHandle(event.value),"Audio capture event handle");
      ComPtr<IAudioCaptureClient> capture;
      check(client->GetService(IID_PPV_ARGS(&capture)),"Audio capture service");
      sanser::audio::Packetizer packetizer(sink_);
      std::vector<float> stereo;
      check(client->Start(),"Start audio capture"); Stop stop{client.Get()};
      std::cout<<"SNV2_AUDIO_CAPTURE started=1 sourceRate="<<wave.nSamplesPerSec<<" channels="<<wave.nChannels<<std::endl;
      auto deviceCheck=Clock::now();
      while(running_) {
        if(WaitForSingleObject(event.value,20)==WAIT_FAILED) throw std::runtime_error("Audio capture wait");
        UINT32 available=0; check(capture->GetNextPacketSize(&available),"Audio capture packet size");
        while(available && running_) {
          BYTE* data=nullptr; UINT32 frames=0; DWORD flags=0;
          check(capture->GetBuffer(&data,&frames,&flags,nullptr,nullptr),"Audio capture buffer");
          const bool silent=(flags&AUDCLNT_BUFFERFLAGS_SILENT)!=0;
          const bool valid=sanser::audio::captureStereo(
            data ? std::span<const std::uint8_t>(data,static_cast<std::size_t>(frames)*wave.nBlockAlign) : std::span<const std::uint8_t>{},
            frames,wave.nChannels,wave.wBitsPerSample,floating,silent,stereo);
          check(capture->ReleaseBuffer(frames),"Release audio capture buffer");
          if(!valid) throw std::runtime_error("Invalid audio capture buffer");
          if(flags&AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY) packetizer.reset();
          if(!packetizer.push(stereo,wave.nSamplesPerSec)) throw std::runtime_error("Audio capture resampling failed");
          check(capture->GetNextPacketSize(&available),"Next audio capture packet");
        }
        if(Clock::now()-deviceCheck>std::chrono::seconds(2)) {
          ComPtr<IMMDevice> current;
          check(enumerator->GetDefaultAudioEndpoint(eRender,eMultimedia,&current),"Current audio capture endpoint");
          if(identity(current.Get())!=selected) break;
          deviceCheck=Clock::now();
        }
      }
    } catch(const std::exception& error) {
      std::cerr<<"SNV2_AUDIO_CAPTURE unavailable: "<<error.what()<<"; retrying without stopping video"<<std::endl;
      for(unsigned i=0;i<50 && running_;++i) std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
  }
}
}
