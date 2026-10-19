#include "voice_runtime.h"
#include "transport.h"
#include "broadcast.h"
#include "microphone_capture.h"
#include "audio_player.h"
#include <atomic>
#include <deque>
#include <iostream>
#include <mutex>
#include <sstream>
#include <cstdio>

namespace sanser::desktop {
struct VoiceRuntime::Impl {
  std::function<void(voice::Packet)> send;
  std::function<void(voice::State)> update;
  std::atomic<bool> running{true},joined{false},microphone{false},deafened{false};
  std::atomic<unsigned> captureToken{0};
  std::mutex mutex;
  struct Incoming { voice::Packet packet; Clock::time_point at; };
  struct Captured { AudioSamples pcm; Clock::time_point at; };
  std::deque<Incoming> incoming;
  std::deque<Captured> captured;
  std::string pendingStatus;
  std::thread worker;
  Impl(std::function<void(voice::Packet)> output,std::function<void(voice::State)> status)
    :send(std::move(output)),update(std::move(status)),worker([this] { run(); }) {}
  ~Impl() { running=false; ++captureToken; if(worker.joinable()) worker.join(); }
  void status(const std::string& code,bool capturing=false) {
    std::ostringstream line;
    line<<"SNVOICE_STATUS joined="<<joined.load()<<" mic="<<microphone.load()
      <<" deafen="<<deafened.load()<<" capture="<<capturing<<" code="<<code;
    const auto text=line.str()+"\n";
    std::fwrite(text.data(),1,text.size(),stdout); std::fflush(stdout);
  }
  void state() { if(update) update({joined.load(),microphone.load(),deafened.load()}); }
  void run() noexcept {
    std::unique_ptr<AudioPlayer> player;
    std::unique_ptr<MicrophoneCapture> mic;
    std::unique_ptr<voice::Encoder> encoder;
    std::unique_ptr<voice::Mixer> mixer;
    std::string code="idle";
    bool capturing=false;
    auto lastStatus=Clock::time_point{};
    try {
      status(code);
      while(running && !nativeStopRequested()) {
        const auto began=Clock::now();
        for(const auto& line:takeVoiceCommands()) {
          if(const auto config=voice::command(line)) {
            joined=config->joined; microphone=config->microphone; deafened=config->deafened;
            // The consent gates change before closing a device or clearing
            // buffered PCM; callbacks cannot enqueue newly muted speech.
            ++captureToken;
            state(); // Revoke transport permission before potentially waiting for a device to close.
            mic.reset(); capturing=false;
            { std::lock_guard lock(mutex); captured.clear(); pendingStatus.clear(); if(!joined || deafened) incoming.clear(); }
            if(encoder) encoder->reset();
            if(!joined || deafened) { player.reset(); if(mixer) mixer->clear(); }
            else {
              if(!mixer) mixer=std::make_unique<voice::Mixer>();
              if(!player) player=std::make_unique<AudioPlayer>(true);
            }
            code=joined ? deafened ? "deafened" : microphone ? "starting" : "listening" : "idle";
            if(joined && microphone && !deafened) {
              if(!encoder) encoder=std::make_unique<voice::Encoder>();
              const auto token=captureToken.load();
              mic=std::make_unique<MicrophoneCapture>(true,[this,token](const AudioSamples& pcm) {
                if(!running || !joined || !microphone || deafened || token!=captureToken) return;
                std::lock_guard lock(mutex);
                if(captured.size()>=8) captured.pop_front();
                captured.push_back({pcm,Clock::now()});
              },[this,token](std::string value) {
                if(token!=captureToken) return;
                std::lock_guard lock(mutex); pendingStatus=std::move(value);
              });
            }
            status(code,capturing);
          } else if(line.size()==14 && line.starts_with("voice-mute\t") && line[11]>='0' && line[11]<'4'
                    && line[12]=='\t' && (line[13]=='0' || line[13]=='1')) {
            if(mixer) mixer->mute(static_cast<unsigned>(line[11]-'0'),line[13]=='1');
          }
        }
        std::deque<Incoming> received; std::deque<Captured> pcm; std::string report;
        {
          std::lock_guard lock(mutex); received.swap(incoming); pcm.swap(captured); report.swap(pendingStatus);
        }
        if(!report.empty()) {
          code=report; capturing=report=="running";
          if(report=="permission-denied" || report=="no-device" || report=="unavailable" || report=="stopped") {
            microphone=false; ++captureToken; mic.reset(); state();
          }
          status(code,capturing);
        }
        if(joined && !deafened && mixer && player) {
          for(auto& item:received) mixer->receive(std::move(item.packet),item.at);
          if(auto samples=mixer->poll(began)) player->submit(std::move(*samples));
        }
        if(joined && microphone && !deafened && encoder) {
          for(const auto& item:pcm) {
            if(Clock::now()-item.at>std::chrono::milliseconds(40)) { encoder->reset(); continue; }
            if(auto packet=encoder->push(item.pcm,item.at)) if(send) send(std::move(*packet));
          }
        }
        if(began-lastStatus>=std::chrono::seconds(1)) { status(code,capturing); lastStatus=began; }
        std::this_thread::sleep_until(began+std::chrono::milliseconds(5));
      }
    } catch(...) { code="unavailable"; }
    joined=microphone=deafened=false; ++captureToken; mic.reset(); player.reset();
    try { state(); status(code=="unavailable" ? code : "idle"); } catch(...) {}
  }
};
VoiceRuntime::VoiceRuntime(bool enabled,std::function<void(voice::Packet)> send,std::function<void(voice::State)> state) {
  if(enabled) impl_=std::make_unique<Impl>(std::move(send),std::move(state));
}
VoiceRuntime::~VoiceRuntime()=default;
void VoiceRuntime::receive(voice::Packet packet) {
  if(!impl_ || !impl_->joined || impl_->deafened) return;
  std::lock_guard lock(impl_->mutex);
  if(impl_->incoming.size()>=24) impl_->incoming.pop_front();
  impl_->incoming.push_back({std::move(packet),Clock::now()});
}
bool VoiceRuntime::listening() const { return impl_ && impl_->joined && !impl_->deafened; }
}
