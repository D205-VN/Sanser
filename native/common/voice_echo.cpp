#include "voice_echo.h"
#include <speex/speex_echo.h>
#include <speex/speex_preprocess.h>
#include <algorithm>
#include <array>
#include <cmath>
#include <limits>
#include <mutex>
#include <stdexcept>

namespace sanser::voice {
namespace {
constexpr unsigned rate=48000,frames=240,history=rate;
std::int64_t sampleTime(EchoClock::time_point at) {
  return std::chrono::duration_cast<std::chrono::microseconds>(at.time_since_epoch()).count()*48/1000;
}
std::int16_t pcm(std::int32_t value) { return static_cast<std::int16_t>(std::clamp(value,-32768,32767)); }
}
struct PlaybackReference::Impl {
  struct Slot {
    std::int64_t stamp=std::numeric_limits<std::int64_t>::min();
    std::int32_t app=0,external=0,full=0; bool endpoint=false;
  };
  std::mutex mutex;
  std::array<Slot,history> samples{};
};
PlaybackReference::PlaybackReference():impl_(std::make_unique<Impl>()) {}
PlaybackReference::~PlaybackReference()=default;
PlaybackReference& PlaybackReference::shared() { static PlaybackReference reference; return reference; }
void PlaybackReference::publish(std::span<const std::int16_t> stereo,EchoClock::time_point at,RenderSource source) {
  if(stereo.size()%2 || stereo.size()>rate*2) return;
  // An output callback never waits for AEC/encoding. Contention loses only
  // this reference; the microphone path bypasses AEC if coverage is missing.
  std::unique_lock lock(impl_->mutex,std::try_to_lock); if(!lock) return;
  auto stamp=sampleTime(at);
  for(std::size_t i=0;i<stereo.size();i+=2,++stamp) {
    if(stamp<0) continue;
    auto& slot=impl_->samples[static_cast<std::size_t>(stamp)%history];
    if(slot.stamp!=stamp) { slot={}; slot.stamp=stamp; }
    const auto value=(static_cast<std::int32_t>(stereo[i])+stereo[i+1])/2;
    if(source==RenderSource::FullEndpoint) { slot.full=value; slot.endpoint=true; }
    else if(source==RenderSource::External) slot.external+=value;
    else slot.app+=value;
  }
}
bool PlaybackReference::read(std::span<std::int16_t> mono,EchoClock::time_point at) {
  auto stamp=sampleTime(at); unsigned present=0;
  std::lock_guard lock(impl_->mutex);
  for(auto& value:mono) {
    const auto& slot=impl_->samples[static_cast<std::size_t>(std::max<std::int64_t>(0,stamp))%history];
    if(slot.stamp==stamp) { value=pcm(slot.endpoint ? slot.full : slot.app+slot.external); ++present; }
    else value=0;
    ++stamp;
  }
  return present*10>=mono.size()*9;
}
void PlaybackTap::publish(std::span<const std::int16_t> stereo,EchoClock::time_point at) {
  if(next_==EchoClock::time_point{} || at-next_>std::chrono::milliseconds(20) || next_-at>std::chrono::milliseconds(20)) next_=at;
  reference_.publish(stereo,next_,source_);
  next_+=std::chrono::microseconds(stereo.size()*1000000ULL/(2*rate));
}
struct EchoCanceller::Impl {
  PlaybackReference& reference;
  SpeexEchoState* echo=nullptr;
  SpeexPreprocessState* preprocess=nullptr;
  EchoClock::time_point next{},lastFarSignal{};
  std::array<spx_int16_t,frames> delayedNear{};
  unsigned missingFrames=0;
  double wet=0;
  explicit Impl(PlaybackReference& input):reference(input) {
    echo=speex_echo_state_init(frames,rate/5); // 200ms acoustic/device tail.
    preprocess=speex_preprocess_state_init(frames,rate);
    if(!echo || !preprocess) {
      if(preprocess) speex_preprocess_state_destroy(preprocess);
      if(echo) speex_echo_state_destroy(echo);
      throw std::runtime_error("Unable to create voice echo canceller");
    }
    int sampleRate=rate,off=0,suppress=-30,active=-10;
    speex_echo_ctl(echo,SPEEX_ECHO_SET_SAMPLING_RATE,&sampleRate);
    speex_preprocess_ctl(preprocess,SPEEX_PREPROCESS_SET_ECHO_STATE,echo);
    speex_preprocess_ctl(preprocess,SPEEX_PREPROCESS_SET_AGC,&off);
    speex_preprocess_ctl(preprocess,SPEEX_PREPROCESS_SET_DENOISE,&off);
    speex_preprocess_ctl(preprocess,SPEEX_PREPROCESS_SET_ECHO_SUPPRESS,&suppress);
    speex_preprocess_ctl(preprocess,SPEEX_PREPROCESS_SET_ECHO_SUPPRESS_ACTIVE,&active);
  }
  ~Impl() { if(preprocess) speex_preprocess_state_destroy(preprocess); if(echo) speex_echo_state_destroy(echo); }
};
EchoCanceller::EchoCanceller(PlaybackReference& reference):impl_(std::make_unique<Impl>(reference)) {}
EchoCanceller::~EchoCanceller()=default;
void EchoCanceller::reset() {
  auto fresh=std::make_unique<Impl>(impl_->reference); impl_=std::move(fresh); ++stats_.resets;
}
void EchoCanceller::process(std::span<std::int16_t> stereo,EchoClock::time_point captured) {
  if(stereo.size()!=frames*2) return;
  ++stats_.blocks;
  if(impl_->next!=EchoClock::time_point{} && (captured-impl_->next>std::chrono::milliseconds(40) || impl_->next-captured>std::chrono::milliseconds(40))) reset();
  if(impl_->next==EchoClock::time_point{}) impl_->next=captured;
  auto& self=*impl_;
  std::array<spx_int16_t,frames> reference{},near{},clean{};
  // Capture timestamps describe each block's first sample. A causal filter
  // must receive reference at that point: shifting it 40ms into the past
  // cannot cancel a short (e.g. 10ms) acoustic path.
  const bool available=self.reference.read(reference,self.next);
  const auto at=self.next;
  self.next+=std::chrono::milliseconds(5);
  if(!available) {
    ++stats_.missingReference; reference.fill(0);
    // Advance the processing timeline through brief reference holes. A long
    // gap invalidates acoustic adaptation, but must not reset the WOLA output
    // history or repeatedly insert a fresh silent block into speech.
    if(self.missingFrames<rate/25) {
      self.missingFrames+=frames;
      if(self.missingFrames==rate/25) { speex_echo_state_reset(self.echo); ++stats_.resets; }
    }
  } else {
    self.missingFrames=0;
    if(std::any_of(reference.begin(),reference.end(),[](auto value) { return std::abs(static_cast<int>(value))>32; })) self.lastFarSignal=at;
  }
  for(unsigned i=0;i<frames;++i) near[i]=pcm((static_cast<int>(stereo[2*i])+stereo[2*i+1])/2);
  // Speex preprocessing uses one frame of overlap/add latency. Returning raw
  // current PCM on a missing reference switched between 0ms and 5ms audio;
  // rebuilding the preprocessor then discarded its overlap on every recovery.
  // Keep it warm, and align safe dry speech to the same previous-frame time.
  speex_echo_cancellation(self.echo,near.data(),reference.data(),clean.data());
  speex_preprocess_run(self.preprocess,clean.data());
  const bool active=available && self.lastFarSignal!=EchoClock::time_point{} && at-self.lastFarSignal<std::chrono::milliseconds(200);
  if(!active) ++stats_.dryBlocks;
  const double target=active ? 1.0 : 0.0;
  for(unsigned i=0;i<frames;++i) {
    self.wet+=std::clamp(target-self.wet,-1.0/frames,1.0/frames);
    const auto sample=static_cast<std::int32_t>(std::lround(self.delayedNear[i]+self.wet*(clean[i]-self.delayedNear[i])));
    stereo[2*i]=stereo[2*i+1]=pcm(sample);
  }
  self.delayedNear=near;
}
}
