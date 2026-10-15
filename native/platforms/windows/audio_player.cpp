#include <windows.h>
#include <audioclient.h>
#include <mmdeviceapi.h>
#include <wrl/client.h>
#include "audio_player.h"
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
      checkAudio(client->Initialize(AUDCLNT_SHAREMODE_SHARED,AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM|AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,200000,0,&format,nullptr),"Initialize audio output");
      UINT32 capacity=0; checkAudio(client->GetBufferSize(&capacity),"Audio buffer size");
      if(capacity<audioFrames) throw std::runtime_error("Audio endpoint buffer is smaller than one block");
      ComPtr<IAudioRenderClient> render;
      checkAudio(client->GetService(IID_PPV_ARGS(&render)),"Audio renderer");
      checkAudio(client->Start(),"Start audio output"); AudioStop stop{client.Get()};
      queue_.clear();
      std::cout<<"SNV2_AUDIO_OUTPUT started=1 rate=48000 channels=2 format=pcm16"<<std::endl;
      auto checkDevice=std::chrono::steady_clock::now();
      while(running_) {
        UINT32 padding=0; checkAudio(client->GetCurrentPadding(&padding),"Audio output padding");
        // Keep at most 10 ms in the device, even if Windows allocated a larger
        // buffer. Silence on loss, never wait for a video frame or retransmit.
        if(padding<=audioFrames && padding+audioFrames<=capacity) {
          auto samples=queue_.pop(); BYTE* target=nullptr;
          checkAudio(render->GetBuffer(audioFrames,&target),"Audio output buffer");
          if(samples) std::memcpy(target,samples->data(),samples->size()*sizeof(std::int16_t));
          checkAudio(render->ReleaseBuffer(audioFrames,samples ? 0 : AUDCLNT_BUFFERFLAGS_SILENT),"Submit audio output");
        }
        if(std::chrono::steady_clock::now()-checkDevice>std::chrono::seconds(2)) {
          ComPtr<IMMDevice> current;
          checkAudio(enumerator->GetDefaultAudioEndpoint(eRender,eMultimedia,&current),"Current audio output");
          if(endpointId(current.Get())!=selected) break;
          checkDevice=std::chrono::steady_clock::now();
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(2));
      }
    } catch(const std::exception& error) {
      std::cerr<<"SNV2_AUDIO_OUTPUT unavailable: "<<error.what()<<"; retrying without stopping video"<<std::endl;
      for(unsigned wait=0;wait<50 && running_;++wait) std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
  }
}
}
