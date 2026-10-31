#include "client.h"
#include "transport.h"
#include <algorithm>
#include <memory>
struct SanserMobileClient {
  std::unique_ptr<sanser::desktop::Peer> peer;
  SanserMobileCallbacks callbacks;
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
    auto *self=result.get();
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
extern "C" int sanser_mobile_start(SanserMobileClient *c) {if(!c)return 0;try {c->peer->start();return 1;}catch(...){return 0;}}
extern "C" void sanser_mobile_destroy(SanserMobileClient *c) {delete c;}
extern "C" int sanser_mobile_running(SanserMobileClient *c) {return c && c->peer->running();}
extern "C" int sanser_mobile_ready(SanserMobileClient *c) {return c && c->peer->ready();}
extern "C" double sanser_mobile_rtt(SanserMobileClient *c) {return c?c->peer->controlRoundTripMs().value_or(-1.0):-1.0;}
extern "C" void sanser_mobile_input(SanserMobileClient *c,unsigned kind,unsigned x,unsigned y,unsigned code,int delta,int down) {
  if(!c || kind>sanser::desktop::Input::Reset || x>65535 || y>65535 || code>65535 || delta<-32768 || delta>32767)return;
  sanser::desktop::Input input;input.kind=static_cast<sanser::desktop::Input::Kind>(kind);input.x=static_cast<uint16_t>(x);input.y=static_cast<uint16_t>(y);input.code=static_cast<uint16_t>(code);input.delta=static_cast<int16_t>(delta);input.down=down!=0;c->peer->input(input);
}
extern "C" void sanser_mobile_mute(SanserMobileClient *c,int muted) {if(c)c->peer->setAudioMuted(muted!=0);}
extern "C" void sanser_mobile_keyframe(SanserMobileClient *c) {if(c)c->peer->requestKeyframe();}
