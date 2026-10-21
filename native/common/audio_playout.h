#pragma once
#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>

namespace sanser {
// Bound the hardware queue by duration, not by a count of variable-size UDP
// packets. The caller waits on its playback callback when this budget is full.
class AudioDeviceBudget {
public:
  bool fits(std::uint32_t rate,std::uint32_t frames) const {
    return queued_+frames<=std::max<std::uint64_t>(frames,rate/50);
  }
  void submitted(std::uint32_t frames) { queued_+=frames; }
  void completed(std::uint32_t frames) { queued_-=std::min<std::uint64_t>(queued_,frames); }
  void clear() { queued_=0; }
  bool empty() const { return queued_==0; }
  std::uint64_t queuedFrames() const { return queued_; }
private:
  std::uint64_t queued_=0;
};

// A discontinuity must not jump instantaneously from a nonzero sample to
// silence (or to an unrelated packet). Only these edges get a 1ms ramp;
// contiguous samples pass through unchanged. No repeated/stretched old audio.
class AudioContinuity {
public:
  template<class Sample>
  void process(Sample* samples,std::size_t frames,unsigned channels,unsigned rate,bool gap) {
    if(!frames || !channels || channels>last_.size()) return;
    begin(rate,gap);
    for(std::size_t frame=0;frame<frames;++frame) {
      const auto gain=nextGain();
      for(unsigned ch=0;ch<channels;++ch) {
        const auto index=frame*channels+ch;
        double value=samples[index];
        if(!std::isfinite(value)) value=0;
        if(gain<1) value=start_[ch]+(value-start_[ch])*gain;
        samples[index]=static_cast<Sample>(value);
      }
    }
    for(unsigned ch=0;ch<channels;++ch) last_[ch]=samples[(frames-1)*channels+ch];
  }
  void floats(std::uint8_t* bytes,std::size_t frames,unsigned channels,unsigned rate,bool gap) {
    if(!frames || !channels || channels>last_.size()) return;
    begin(rate,gap);
    for(std::size_t frame=0;frame<frames;++frame) {
      const auto gain=nextGain();
      for(unsigned ch=0;ch<channels;++ch) {
        auto* at=bytes+(frame*channels+ch)*sizeof(float);
        float sample; std::memcpy(&sample,at,sizeof(sample));
        double value=std::isfinite(sample) ? std::clamp(sample,-1.0F,1.0F) : 0.0;
        if(gain<1) value=start_[ch]+(value-start_[ch])*gain;
        sample=static_cast<float>(value); std::memcpy(at,&sample,sizeof(sample));
      }
    }
    for(unsigned ch=0;ch<channels;++ch) {
      float sample; std::memcpy(&sample,bytes+((frames-1)*channels+ch)*sizeof(float),sizeof(sample)); last_[ch]=sample;
    }
  }
private:
  void begin(unsigned rate,bool gap) {
    if(gap) { start_=last_; ramp_=std::max<std::size_t>(1,rate/1000); used_=0; }
  }
  double nextGain() {
    if(used_>=ramp_) return 1;
    return static_cast<double>(++used_)/ramp_;
  }
  std::array<double,8> last_{},start_{};
  std::size_t ramp_=0,used_=0;
};
}
