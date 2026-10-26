#include "voice_runtime.h"
#include "transport.h"
#include "broadcast.h"
#include "microphone_capture.h"
#include "audio_player.h"
#include "voice_echo.h"
#include <algorithm>
#include <atomic>
#include <deque>
#include <iostream>
#include <mutex>
#include <sstream>
#include <cstdio>
#include <cmath>

namespace sanser::desktop {
struct VoiceRuntime::Impl {
  std::function<void(voice::Packet)> send;
  std::function<void(voice::State)> update;
  std::atomic<bool> running{true},joined{false},microphone{false},deafened{false};
  std::atomic<unsigned> captureToken{0};
  std::atomic<std::uint64_t> captureDrops{0},incomingDrops{0};
  std::mutex mutex;
  struct Incoming { voice::Packet packet; Clock::time_point at; };
  struct Captured { AudioSamples pcm; Clock::time_point at; };
  std::deque<Incoming> incoming;
  std::deque<Captured> captured;
  std::string pendingStatus;
  std::deque<std::string> commands;
  VoiceRuntime::Status observed;
  std::thread worker;
  Impl(std::function<void(voice::Packet)> output,std::function<void(voice::State)> status)
    :send(std::move(output)),update(std::move(status)),worker([this] { run(); }) {}
  ~Impl() { running=false; ++captureToken; if(worker.joinable()) worker.join(); }
  void status(const std::string& code,bool capturing=false) {
    { std::lock_guard lock(mutex); observed={{joined.load(),microphone.load(),deafened.load()},capturing,code}; }
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
    std::unique_ptr<voice::EchoCanceller> echo;
    std::string code="idle";
    bool capturing=false;
    auto lastStatus=Clock::time_point{};
    auto previousWake=Clock::time_point{};
    std::uint64_t capturedFrames=0,rawClipped=0,cleanClipped=0,staleCapture=0,sentPackets=0;
    unsigned rawPeak=0,cleanPeak=0; double wakeGapMax=0;
    const auto signalLevel=[](const AudioSamples& samples,unsigned& peak,std::uint64_t& clipped) {
      for(unsigned i=0;i<audioFrames;++i) {
        const auto level=static_cast<unsigned>(std::max(std::abs(static_cast<int>(samples[i*2])),std::abs(static_cast<int>(samples[i*2+1]))));
        peak=std::max(peak,level); if(level>=32760) ++clipped;
      }
    };
    try {
      status(code);
      while(running && !nativeStopRequested()) {
        const auto began=Clock::now();
        if(joined && previousWake!=Clock::time_point{}) wakeGapMax=std::max(wakeGapMax,std::chrono::duration<double,std::milli>(began-previousWake).count());
        previousWake=began;
        auto changes=takeVoiceCommands();
        { std::lock_guard lock(mutex); for(auto& line:commands) changes.push_back(std::move(line)); commands.clear(); }
        for(const auto& line:changes) {
          if(const auto config=voice::command(line)) {
            joined=config->joined; microphone=config->microphone; deafened=config->deafened;
            // The consent gates change before closing a device or clearing
            // buffered PCM; callbacks cannot enqueue newly muted speech.
            ++captureToken;
            state(); // Revoke transport permission before potentially waiting for a device to close.
            mic.reset(); capturing=false;
            echo.reset(); // Revoking mic consent destroys its adaptation history.
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
              echo=std::make_unique<voice::EchoCanceller>();
              const auto token=captureToken.load();
              mic=std::make_unique<MicrophoneCapture>(true,[this,token](const AudioSamples& pcm,Clock::time_point at) {
                if(!running || !joined || !microphone || deafened || token!=captureToken) return;
                std::lock_guard lock(mutex);
                if(captured.size()>=8) { captured.pop_front(); ++captureDrops; }
                captured.push_back({pcm,at});
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
          for(auto& samples:mixer->drain(began)) player->submit(std::move(samples));
        }
        if(joined && microphone && !deafened && encoder) {
          for(auto& item:pcm) {
            if(Clock::now()-item.at>std::chrono::milliseconds(40)) { ++staleCapture; encoder->reset(); continue; }
            capturedFrames+=audioFrames; signalLevel(item.pcm,rawPeak,rawClipped);
            if(echo) echo->process(item.pcm,item.at);
            signalLevel(item.pcm,cleanPeak,cleanClipped);
            if(auto packet=encoder->push(item.pcm,item.at)) if(send) { send(std::move(*packet)); ++sentPackets; }
          }
        }
        if(began-lastStatus>=std::chrono::seconds(1)) {
          status(code,capturing); lastStatus=began;
          if(joined) {
            const auto aec=echo ? echo->stats() : voice::EchoCanceller::Stats{};
            const auto mix=mixer ? mixer->stats() : voice::Stats{};
            const auto output=player ? player->stats() : AudioQueue::Stats{};
            std::ostringstream line;
            line<<"SNVOICE_HEALTH voiceCapturedFrames="<<capturedFrames<<" voiceRawPeak="<<rawPeak<<" voiceCleanPeak="<<cleanPeak
              <<" voiceRawClippedFrames="<<rawClipped<<" voiceCleanClippedFrames="<<cleanClipped
              <<" voiceCaptureQueueDrops="<<captureDrops.load()<<" voiceIncomingQueueDrops="<<incomingDrops.load()
              <<" voiceStaleCaptureBlocks="<<staleCapture<<" voicePacketsSent="<<sentPackets<<" voiceWorkerGapMaxMs="<<wakeGapMax
              <<" voiceDecoded="<<mix.decoded<<" voicePlc="<<mix.concealed<<" voiceLate="<<mix.late
              <<" voiceDropped="<<mix.dropped<<" voiceInvalid="<<mix.invalid<<" voiceLimitedFrames="<<mix.limitedFrames
              <<" voicePcmUnderflows="<<output.underflows<<" voicePcmCapacityDrops="<<output.capacityDrops
              <<" voicePcmStaleDrops="<<output.staleDrops<<" voicePcmReprimes="<<output.reprimes
              <<" voiceAecBlocks="<<aec.blocks<<" voiceAecMissingReference="<<aec.missingReference
              <<" voiceAecDryBlocks="<<aec.dryBlocks<<" voiceAecResets="<<aec.resets<<'\n';
            const auto text=line.str(); std::fwrite(text.data(),1,text.size(),stdout); std::fflush(stdout);
          }
        }
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
void VoiceRuntime::configure(voice::State state) {
  if(!impl_) return;
  std::ostringstream line;
  line<<"voice-config\t"<<state.joined<<'\t'<<state.microphone<<'\t'<<state.deafened;
  std::lock_guard lock(impl_->mutex);
  if(impl_->commands.size()>=16) impl_->commands.pop_front();
  impl_->commands.push_back(line.str());
}
VoiceRuntime::Status VoiceRuntime::snapshot() const {
  if(!impl_) return {};
  std::lock_guard lock(impl_->mutex); return impl_->observed;
}
void VoiceRuntime::receive(voice::Packet packet) {
  if(!impl_ || !impl_->joined || impl_->deafened) return;
  std::lock_guard lock(impl_->mutex);
  if(impl_->incoming.size()>=24) { impl_->incoming.pop_front(); ++impl_->incomingDrops; }
  impl_->incoming.push_back({std::move(packet),Clock::now()});
}
bool VoiceRuntime::listening() const { return impl_ && impl_->joined && !impl_->deafened; }
}
