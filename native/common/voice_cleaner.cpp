#include "voice_cleaner.h"
#include <rnnoise.h>
#include <algorithm>
#include <array>
#include <cmath>
#include <deque>
#include <stdexcept>

namespace sanser::voice {
namespace {
constexpr unsigned blockFrames=240,rnnFrames=480,rate=48000;
constexpr float ceiling=0.89f*32767.f;           // -1 dBFS
constexpr float floorGain=0.0316f;               // -30 dB while gated
constexpr float openProbability=0.6f,keepProbability=0.25f;
constexpr unsigned holdBlocks=60;                // 300 ms of 5 ms blocks
}
struct VoiceCleaner::Impl {
  DenoiseState* state=nullptr;
  std::array<float,rnnFrames> input{};
  unsigned used=0;
  std::deque<float> output;
  float gate=1.f,limiter=1.f,probability=0.f;
  unsigned hold=0;
  Stats stats;
  Impl() { state=rnnoise_create(nullptr); if(!state) throw std::runtime_error("Unable to create RNNoise state"); }
  ~Impl() { if(state) rnnoise_destroy(state); }
};
VoiceCleaner::VoiceCleaner():impl_(std::make_unique<Impl>()) { reset(); }
VoiceCleaner::~VoiceCleaner()=default;
void VoiceCleaner::reset() {
  auto& s=*impl_;
  rnnoise_destroy(s.state); s.state=rnnoise_create(nullptr);
  if(!s.state) throw std::runtime_error("Unable to create RNNoise state");
  s.used=0; s.output.assign(rnnFrames,0.f); // fixed 10 ms latency
  s.gate=1.f; s.limiter=1.f; s.probability=0.f; s.hold=holdBlocks;
}
void VoiceCleaner::process(std::span<std::int16_t> stereo) {
  if(stereo.size()!=blockFrames*2) return;
  auto& s=*impl_;
  for(unsigned i=0;i<blockFrames;++i) s.input[s.used++]=(static_cast<float>(stereo[2*i])+stereo[2*i+1])*0.5f;
  if(s.used==rnnFrames) {
    std::array<float,rnnFrames> clean{};
    s.probability=rnnoise_process_frame(s.state,clean.data(),s.input.data());
    s.output.insert(s.output.end(),clean.begin(),clean.end());
    s.used=0; s.stats.frames+=rnnFrames; s.stats.lastVoiceProbability=s.probability;
  }
  // Gate target from the most recent voice-activity estimate.
  if(s.probability>=openProbability) s.hold=0;
  else if(s.probability>=keepProbability && s.hold<holdBlocks) s.hold=0;
  else if(s.hold<holdBlocks) ++s.hold;
  const float target=s.hold<holdBlocks ? 1.f : floorGain;
  if(target<1.f && s.gate<0.5f) ++s.stats.gatedBlocks;
  std::array<float,blockFrames> block{};
  float peak=0;
  for(unsigned i=0;i<blockFrames;++i) {
    // Attack ~2 ms, release ~150 ms.
    s.gate+=(target-s.gate)*(target>s.gate ? 0.01f : 0.00014f);
    block[i]=s.output.front()*s.gate; s.output.pop_front();
    peak=std::max(peak,std::abs(block[i]));
  }
  const float wanted=peak>ceiling ? ceiling/peak : 1.f;
  if(wanted<1.f) ++s.stats.limitedBlocks;
  for(unsigned i=0;i<blockFrames;++i) {
    // Instant attack (no overs), ~200 ms release back to unity gain.
    s.limiter=wanted<s.limiter ? wanted : s.limiter+(std::min(1.f,wanted)-s.limiter)*0.0001f;
    const auto value=static_cast<std::int16_t>(std::clamp(std::lround(block[i]*s.limiter),-32768L,32767L));
    stereo[2*i]=stereo[2*i+1]=value;
  }
}
VoiceCleaner::Stats VoiceCleaner::stats() const { return impl_->stats; }
}
