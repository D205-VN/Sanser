#include "client.h"
#include "transport.h"
#include "gamepad_capture.h"
#include "voice.h"
#include "voice_capture_policy.h"
#include <algorithm>
#include <deque>
#include <memory>
#include <thread>
namespace {
using sanser::desktop::Clock;
// Opus encode/decode and the jitter mixer run on one 5 ms worker, never on the
// UDP reader or the platform's realtime audio thread. The platform supplies
// echo-cancelled microphone PCM and plays the mixed voice output.
class MobileVoice {
public:
  MobileVoice(sanser::desktop::Peer& peer,SanserMobileCallbacks callbacks):peer_(peer),callbacks_(callbacks),worker_([this]{run();}) {}
  ~MobileVoice(){stop();}
  void stop(){running_=false;if(worker_.joinable())worker_.join();}
  void state(bool joined,bool microphone,bool deafened) {
    if(!joined) microphone=deafened=false;
    if(deafened) microphone=false;
    // Revoke transport permission before anything else, then drop buffered PCM.
    peer_.setVoiceState({joined,microphone,deafened});
    joined_=joined;microphone_=microphone;deafened_=deafened;
    std::lock_guard lock(mutex_);++epoch_;captured_.clear();if(!joined || deafened)incoming_.clear();
  }
  bool capture(const std::int16_t* mono,std::size_t frames) {
    if(!running_ || !joined_ || !microphone_ || deafened_) return false;
    std::lock_guard capture(captureMutex_);
    const auto epoch=epoch_.load();
    if(epoch!=captureEpoch_){captureEpoch_=epoch;used_=0;}
    for(std::size_t i=0;i<frames;++i) {
      pending_[used_*2]=pending_[used_*2+1]=mono[i];
      if(++used_<sanser::desktop::audioFrames) continue;
      used_=0;
      std::lock_guard lock(mutex_);
      if(epoch_!=epoch) return false;
      // iOS voice processing delivers ~100 ms per tap (20 blocks); 8 dropped
      // most of every callback. The worker drains each 5 ms; stale ones are
      // still skipped by staleCapture.
      if(captured_.size()>=48) captured_.pop_front();
      captured_.push_back({pending_,Clock::now()});
    }
    return true;
  }
  void receive(sanser::voice::Packet packet) {
    if(!joined_ || deafened_) return;
    std::lock_guard lock(mutex_);
    if(incoming_.size()>=24) incoming_.pop_front();
    incoming_.push_back({std::move(packet),Clock::now()});
  }
private:
  void run() noexcept {
    std::unique_ptr<sanser::voice::Encoder> encoder;
    std::unique_ptr<sanser::voice::Mixer> mixer;
    std::uint64_t seen=0;
    while(running_) {
      const auto began=Clock::now();
      try {
        std::deque<std::pair<sanser::voice::Packet,Clock::time_point>> received;
        std::deque<std::pair<sanser::desktop::AudioSamples,Clock::time_point>> pcm;
        std::uint64_t epoch=0;
        { std::lock_guard lock(mutex_); received.swap(incoming_); pcm.swap(captured_); epoch=epoch_; }
        if(epoch!=seen) {
          seen=epoch;encoder.reset();mixer.reset();
          if(joined_ && !deafened_) mixer=std::make_unique<sanser::voice::Mixer>();
          if(joined_ && microphone_ && !deafened_) encoder=std::make_unique<sanser::voice::Encoder>();
        }
        if(mixer) {
          for(auto& [packet,at]:received) mixer->receive(std::move(packet),at);
          for(auto& block:mixer->drain(began)) if(callbacks_.voice) callbacks_.voice(callbacks_.context,block.data(),block.size());
        }
        if(encoder && !pcm.empty()) {
          const auto newest=pcm.back().second;bool skipped=false;
          for(auto& [block,at]:pcm) {
            if(sanser::voice::staleCapture(at,newest,Clock::now())){skipped=true;continue;}
            if(skipped){encoder->reset();skipped=false;}
            if(auto packet=encoder->push(block,at)) peer_.voicePacket(std::move(*packet));
          }
        }
      } catch(...) {seen=0;encoder.reset();mixer.reset();}
      std::this_thread::sleep_until(began+std::chrono::milliseconds(5));
    }
  }
  sanser::desktop::Peer& peer_;
  SanserMobileCallbacks callbacks_;
  std::atomic<bool> running_{true},joined_{false},microphone_{false},deafened_{false};
  std::atomic<std::uint64_t> epoch_{1};
  std::mutex mutex_,captureMutex_;
  std::deque<std::pair<sanser::voice::Packet,Clock::time_point>> incoming_;
  std::deque<std::pair<sanser::desktop::AudioSamples,Clock::time_point>> captured_;
  sanser::desktop::AudioSamples pending_{};std::size_t used_=0;std::uint64_t captureEpoch_=0;
  std::thread worker_;
};
}
struct SanserMobileClient {
  std::unique_ptr<sanser::desktop::Peer> peer;
  // Declared after peer: destroyed first, but only after peer->stop() below.
  std::unique_ptr<MobileVoice> voice;
  std::unique_ptr<sanser::mobile::GamepadCapture> gamepad;
  SanserMobileCallbacks callbacks;
  std::mutex fileMutex;
  std::deque<std::pair<uint32_t,sanser::desktop::Bytes>> files;
  std::atomic<bool> fileOverflow{false};
  ~SanserMobileClient() {
    // Peer::loop invokes onStopped while stop() joins its worker. Do this while
    // peer is still owned: unique_ptr destruction can clear its pointer before
    // invoking Peer::~Peer, so reading self->peer in that callback would crash.
    gamepad.reset();if(voice) voice->stop();if(peer) peer->stop();
  }
};
extern "C" SanserMobileClient *sanser_mobile_create(const char *endpoint,uint16_t port,const char *token,int control,int sharing,const char *viewer,SanserMobileCallbacks callbacks) {
  if(!endpoint || !token || !callbacks.video) return nullptr;
  try {
    auto result=std::make_unique<SanserMobileClient>();
    sanser::desktop::Options o;
    o.peer=endpoint; o.port=port; o.token=token; o.input=control!=0;
    o.sharing=sharing!=0; o.viewerId=viewer?viewer:""; o.audio=true;
    o.groupVoice=o.sharing && !o.input;
    o.collab=true;
    o.width=1920; o.height=1080; o.fps=60; o.bitrate=12000000;
    result->callbacks=callbacks;
    result->peer=std::make_unique<sanser::desktop::Peer>(o);
    result->peer->receiveVideoLimit(6000000);
    auto *self=result.get();
    result->gamepad=std::make_unique<sanser::mobile::GamepadCapture>(
      [self]{return self->peer->gamepadAvailable() && self->peer->inputAvailable() ? self->peer->remoteGeneration():0U;},
      [self](auto state,auto epoch){return self->peer->gamepad(state,epoch);});
    result->peer->onFile=[self](sanser::desktop::Bytes body) {
      std::lock_guard lock(self->fileMutex);
      if(self->files.size()>=16){self->files.clear();self->fileOverflow=true;return;}
      self->files.emplace_back(self->peer->fileGeneration(),std::move(body));
    };
    result->peer->onFrame=[self](sanser::desktop::Frame frame) {
      self->callbacks.video(self->callbacks.context,frame.data.data(),frame.data.size(),frame.width,frame.height,frame.keyframe?1:0);
    };
    result->peer->onAudio=[self](sanser::desktop::AudioSamples samples) {
      if(self->callbacks.audio) self->callbacks.audio(self->callbacks.context,samples.data(),samples.size());
    };
    if(o.groupVoice) {
      result->voice=std::make_unique<MobileVoice>(*result->peer,callbacks);
      result->peer->onVoice=[self](sanser::voice::Packet packet){self->voice->receive(std::move(packet));};
    }
    result->peer->onCollab=[self](sanser::desktop::Bytes payload) {
      if(self->callbacks.collab) self->callbacks.collab(self->callbacks.context,payload.data(),payload.size());
    };
    result->peer->onStopped=[self] {if(self->callbacks.stopped) self->callbacks.stopped(self->callbacks.context,self->peer->stopReason());};
    return result.release();
  } catch(...) {return nullptr;}
}
extern "C" void sanser_mobile_video_limit(SanserMobileClient *c,uint32_t bitrate,double baseline){if(c)c->peer->receiveVideoLimit(bitrate,baseline);}
extern "C" int sanser_mobile_start(SanserMobileClient *c) {if(!c)return 0;try {c->peer->start();return 1;}catch(...){return 0;}}
extern "C" void sanser_mobile_destroy(SanserMobileClient *c) {delete c;}
extern "C" int sanser_mobile_running(SanserMobileClient *c) {return c && c->peer->running();}
extern "C" int sanser_mobile_ready(SanserMobileClient *c) {return c && c->peer->ready();}
extern "C" double sanser_mobile_rtt(SanserMobileClient *c) {return c?c->peer->controlRoundTripMs().value_or(-1.0):-1.0;}
extern "C" void sanser_mobile_stats(SanserMobileClient *c,SanserMobileStats *stats) {
  if(!stats)return;
  if(!c){*stats={};return;}
  const auto s=c->peer->receiveHealth();
  *stats={s.videoPackets,s.videoFrames,s.nackRequests,s.lostFragments,s.referenceDrops,s.audioDecoded,s.audioConcealed,s.audioLate,s.sampleTimeMs};
}
extern "C" void sanser_mobile_input(SanserMobileClient *c,unsigned kind,unsigned x,unsigned y,unsigned code,int delta,int down) {
  if(!c || kind>sanser::desktop::Input::Reset || x>65535 || y>65535 || code>65535 || delta<-32768 || delta>32767)return;
  if(kind==sanser::desktop::Input::Reset)c->gamepad->release();
  sanser::desktop::Input input;input.kind=static_cast<sanser::desktop::Input::Kind>(kind);input.x=static_cast<uint16_t>(x);input.y=static_cast<uint16_t>(y);input.code=static_cast<uint16_t>(code);input.delta=static_cast<int16_t>(delta);input.down=down!=0;c->peer->input(input);
}
extern "C" int sanser_mobile_gamepad_available(SanserMobileClient *c){return c && c->peer->gamepadAvailable() && c->peer->inputAvailable();}
extern "C" int sanser_mobile_gamepad_capture(SanserMobileClient *c,uint64_t id,int enabled){try{return c && c->gamepad->capture(id,enabled!=0);}catch(...){return 0;}}
extern "C" int sanser_mobile_gamepad_sample(SanserMobileClient *c,uint64_t id,const SanserMobileGamepad *s){try{return c && s && c->gamepad->sample(id,*s);}catch(...){return 0;}}
extern "C" unsigned sanser_mobile_gamepad_status(SanserMobileClient *c,uint64_t id){try{return c ? c->gamepad->status(id):0;}catch(...){return 0;}}
extern "C" void sanser_mobile_mute(SanserMobileClient *c,int muted) {if(c)c->peer->setAudioMuted(muted!=0);}
extern "C" int sanser_mobile_text_available(SanserMobileClient *c) {return c && c->peer->textAvailable();}
extern "C" int sanser_mobile_remote_control_off(SanserMobileClient *c) {return c && c->peer->remoteControlOff();}
extern "C" int sanser_mobile_clipboard_available(SanserMobileClient *c) {return c && c->peer->clipboardAvailable();}
extern "C" int sanser_mobile_files_enable(SanserMobileClient *c,int enabled){return c && c->peer->configureFiles(enabled!=0);}
extern "C" int sanser_mobile_files_available(SanserMobileClient *c){return c && c->peer->filesAvailable();}
extern "C" uint32_t sanser_mobile_file_generation(SanserMobileClient *c){return c?c->peer->fileGeneration():0;}
extern "C" int sanser_mobile_file_overflow(SanserMobileClient *c){return c && c->fileOverflow.exchange(false);}
extern "C" int sanser_mobile_file_send(SanserMobileClient *c,uint32_t epoch,const uint8_t* data,size_t count){
  if(!c || !epoch || !data || count<21 || count>8192)return 0;
  try{return c->peer->fileMessage(sanser::desktop::Bytes(data,data+count),epoch);}catch(...){return 0;}
}
extern "C" size_t sanser_mobile_file_receive(SanserMobileClient *c,uint32_t epoch,uint8_t* out,size_t capacity){
  if(!c || !out || capacity<8192)return 0;
  std::lock_guard lock(c->fileMutex);
  while(!c->files.empty()) {auto message=std::move(c->files.front());c->files.pop_front();
    if(epoch!=c->peer->fileGeneration() || message.first!=epoch)continue;
    std::copy(message.second.begin(),message.second.end(),out);return message.second.size();}
  return 0;
}
extern "C" int sanser_mobile_clipboard(SanserMobileClient *c,int write,const uint8_t *utf8,size_t bytes,void *context,SanserMobileClipboardReply reply) {
  if(!c || !reply || (!utf8 && bytes) || bytes>8192)return 0;
  try {
    const std::string text=bytes ? std::string(reinterpret_cast<const char*>(utf8),bytes):std::string{};
    return c->peer->clipboard(write!=0,text,[context,reply](unsigned status,std::string result){reply(context,status,reinterpret_cast<const uint8_t*>(result.data()),result.size());});
  }catch(...){return 0;}
}
extern "C" int sanser_mobile_text(SanserMobileClient *c,const uint8_t *utf8,size_t bytes) {
  if(!c || !utf8 || !bytes || bytes>1024) return 0;
  try { return c->peer->text(std::string(reinterpret_cast<const char*>(utf8),bytes)); }
  catch(...) { return 0; } // Never unwind a C++ exception through Swift/JNI.
}
extern "C" int sanser_mobile_voice_available(SanserMobileClient *c){return c && c->voice;}
extern "C" int sanser_mobile_voice_state(SanserMobileClient *c,int joined,int microphone,int deafened){
  if(!c || !c->voice)return 0;
  try{c->voice->state(joined!=0,microphone!=0,deafened!=0);return 1;}catch(...){return 0;}
}
extern "C" int sanser_mobile_voice_capture(SanserMobileClient *c,const int16_t *mono,size_t frames){
  if(!c || !c->voice || !mono || !frames || frames>48000)return 0;
  try{return c->voice->capture(mono,frames);}catch(...){return 0;}
}
extern "C" int sanser_mobile_collab_available(SanserMobileClient *c){return c && c->peer->collabAvailable();}
extern "C" int sanser_mobile_collab_send(SanserMobileClient *c,const uint8_t *payload,size_t bytes){
  if(!c || !payload || bytes==0 || bytes>sanser::desktop::collabMaxPayload) return 0;
  try{return c->peer->collab(sanser::desktop::Bytes(payload,payload+bytes));}catch(...){return 0;}
}
extern "C" void sanser_mobile_video_hidden(SanserMobileClient *c,int hidden){if(c)try{c->peer->setVideoHidden(hidden!=0);}catch(...){}}
extern "C" void sanser_mobile_keyframe(SanserMobileClient *c) {if(c)c->peer->requestKeyframe();}
