#include "client.h"
#include "transport.h"
#include "gamepad_capture.h"
#include <algorithm>
#include <memory>
struct SanserMobileClient {
  std::unique_ptr<sanser::desktop::Peer> peer;
  std::unique_ptr<sanser::mobile::GamepadCapture> gamepad;
  SanserMobileCallbacks callbacks;
  std::mutex fileMutex;
  std::deque<std::pair<uint32_t,sanser::desktop::Bytes>> files;
  std::atomic<bool> fileOverflow{false};
  ~SanserMobileClient() {
    // Peer::loop invokes onStopped while stop() joins its worker. Do this while
    // peer is still owned: unique_ptr destruction can clear its pointer before
    // invoking Peer::~Peer, so reading self->peer in that callback would crash.
    gamepad.reset();if(peer) peer->stop();
  }
};
extern "C" SanserMobileClient *sanser_mobile_create(const char *endpoint,uint16_t port,const char *token,int control,int sharing,const char *viewer,SanserMobileCallbacks callbacks) {
  if(!endpoint || !token || !callbacks.video) return nullptr;
  try {
    auto result=std::make_unique<SanserMobileClient>();
    sanser::desktop::Options o;
    o.peer=endpoint; o.port=port; o.token=token; o.input=control!=0;
    o.sharing=sharing!=0; o.viewerId=viewer?viewer:""; o.audio=true;
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
  *stats={s.videoPackets,s.videoFrames,s.nackRequests,s.lostFragments,s.referenceDrops,s.audioDecoded,s.audioConcealed,s.audioLate};
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
extern "C" void sanser_mobile_keyframe(SanserMobileClient *c) {if(c)c->peer->requestKeyframe();}
