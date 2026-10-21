#include <windows.h>
#include <audioclient.h>
#include <mmdeviceapi.h>
#include <wrl/client.h>
#include "audio_player.h"
#include "audio_playout.h"
#include "voice_echo.h"
#include <cstring>
#include <iostream>
#include <stdexcept>
#include <string>
using Microsoft::WRL::ComPtr;
namespace sanser::desktop {
namespace {
void checkAudio(HRESULT value,const char* operation) {
  if(FAILED(value)) throw std::runtime_error(std::string(operation)+" HRESULT="+std::to_string(static_cast<unsigned long>(value)));
}
struct ComRuntime {
  ComRuntime() { checkAudio(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"Audio COM startup"); }
  ~ComRuntime() { CoUninitialize(); }
};
struct AudioStop { IAudioClient* client; ~AudioStop() { client->Stop(); } };
struct AudioEvent { HANDLE value=CreateEventW(nullptr,FALSE,FALSE,nullptr); ~AudioEvent() { if(value) CloseHandle(value); } };
std::wstring endpointId(IMMDevice* device) {
  LPWSTR value=nullptr; checkAudio(device->GetId(&value),"Audio endpoint ID");
  std::wstring result(value); CoTaskMemFree(value); return result;
}
}
AudioPlayer::AudioPlayer(bool enabled) { if(enabled) worker_=std::thread([this] { run(); }); }
AudioPlayer::~AudioPlayer() { running_=false; if(worker_.joinable()) worker_.join(); }
void AudioPlayer::run() {
  // Endpoint activation, rendering and COM teardown all stay on this worker.
  // USB/Bluetooth removal must not stop video or the authenticated input peer.
  while(running_) {
    try {
      ComRuntime com;
      ComPtr<IMMDeviceEnumerator> enumerator;
      checkAudio(CoCreateInstance(__uuidof(MMDeviceEnumerator),nullptr,CLSCTX_ALL,IID_PPV_ARGS(&enumerator)),"Audio device enumerator");
      ComPtr<IMMDevice> device;
      checkAudio(enumerator->GetDefaultAudioEndpoint(eRender,eMultimedia,&device),"Default audio output");
      const auto selected=endpointId(device.Get());
      ComPtr<IAudioClient> client;
      checkAudio(device->Activate(__uuidof(IAudioClient),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(client.GetAddressOf())),"Activate audio output");
      WAVEFORMATEX format{}; format.wFormatTag=WAVE_FORMAT_PCM; format.nChannels=audioChannels;
      format.nSamplesPerSec=audioRate; format.wBitsPerSample=16; format.nBlockAlign=audioChannels*2;
      format.nAvgBytesPerSec=audioRate*format.nBlockAlign;
      checkAudio(client->Initialize(AUDCLNT_SHAREMODE_SHARED,AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM|AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY|AUDCLNT_STREAMFLAGS_EVENTCALLBACK,200000,0,&format,nullptr),"Initialize audio output");
      AudioEvent event;
      if(!event.value) throw std::runtime_error("Create audio output event failed");
      checkAudio(client->SetEventHandle(event.value),"Set audio output event");
      UINT32 capacity=0; checkAudio(client->GetBufferSize(&capacity),"Audio buffer size");
      if(capacity<audioFrames) throw std::runtime_error("Audio endpoint buffer is smaller than one block");
      ComPtr<IAudioRenderClient> render;
      checkAudio(client->GetService(IID_PPV_ARGS(&render)),"Audio renderer");
      queue_.clear();
      const auto targetFrames=std::min<UINT32>(capacity,audioFrames*4);
      BYTE* prime=nullptr; checkAudio(render->GetBuffer(targetFrames,&prime),"Prime audio output");
      checkAudio(render->ReleaseBuffer(targetFrames,AUDCLNT_BUFFERFLAGS_SILENT),"Submit priming silence");
      checkAudio(client->Start(),"Start audio output"); AudioStop stop{client.Get()};
      sanser::AudioContinuity continuity; bool wasSilent=true;
      voice::PlaybackTap reference;
      std::cout<<"SNV2_AUDIO_OUTPUT started=1 rate=48000 channels=2 format=pcm16"<<std::endl;
      auto checkDevice=std::chrono::steady_clock::now();
      while(running_) {
        UINT32 padding=0; checkAudio(client->GetCurrentPadding(&padding),"Audio output padding");
        // Device events replace sleep_for(2ms), which can oversleep a whole
        // Windows timer tick. Refill all available blocks up to 20ms, so a
        // normal 10ms device period cannot underrun a one-block 5ms queue.
        while(padding+audioFrames<=targetFrames) {
          auto samples=queue_.popBlock(); BYTE* target=nullptr;
          AudioSamples output=samples ? std::move(samples->samples) : AudioSamples{};
          continuity.process(output.data(),audioFrames,audioChannels,audioRate,!samples || wasSilent || samples->discontinuity);
          wasSilent=!samples;
          checkAudio(render->GetBuffer(audioFrames,&target),"Audio output buffer");
          std::memcpy(target,output.data(),output.size()*sizeof(std::int16_t));
          checkAudio(render->ReleaseBuffer(audioFrames,0),"Submit audio output");
          reference.publish(output,voice::EchoClock::now()+std::chrono::microseconds(static_cast<std::uint64_t>(padding)*1000000/audioRate));
          padding+=audioFrames;
        }
        if(std::chrono::steady_clock::now()-checkDevice>std::chrono::seconds(2)) {
          ComPtr<IMMDevice> current;
          checkAudio(enumerator->GetDefaultAudioEndpoint(eRender,eMultimedia,&current),"Current audio output");
          if(endpointId(current.Get())!=selected) break;
          checkDevice=std::chrono::steady_clock::now();
        }
        if(WaitForSingleObject(event.value,20)==WAIT_FAILED) throw std::runtime_error("Audio output event wait failed");
      }
    } catch(const std::exception& error) {
      std::cerr<<"SNV2_AUDIO_OUTPUT unavailable: "<<error.what()<<"; retrying without stopping video"<<std::endl;
      for(unsigned wait=0;wait<50 && running_;++wait) std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
  }
}
}
