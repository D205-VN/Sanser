#pragma once
#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <deque>
#include <functional>
#include <mutex>
#include <optional>
#include <vector>
#include <utility>

namespace sanser::desktop {
// PCM16 little endian, 48 kHz stereo; 5 ms fits in one authenticated UDP
// datagram. Never fragment/retransmit audio or put playback on the UDP reader.
inline constexpr unsigned audioRate=48000, audioChannels=2, audioFrames=240;
using AudioSamples=std::array<std::int16_t,audioFrames*audioChannels>;
inline std::vector<std::uint8_t> encodeAudio(const AudioSamples& samples) {
  std::vector<std::uint8_t> bytes{'S','P','A','1'};
  bytes.reserve(4+samples.size()*2);
  for(auto value:samples) { const auto bits=static_cast<std::uint16_t>(value); bytes.push_back(bits&255); bytes.push_back(bits>>8); }
  return bytes;
}
inline std::optional<AudioSamples> decodeAudio(const std::vector<std::uint8_t>& bytes) {
  if(bytes.size()!=4+audioFrames*audioChannels*2 || !std::equal(bytes.begin(),bytes.begin()+4,"SPA1")) return {};
  AudioSamples samples{};
  for(std::size_t i=0;i<samples.size();++i) {
    const auto bits=static_cast<std::uint16_t>(bytes[4+i*2] | (bytes[5+i*2]<<8));
    samples[i]=static_cast<std::int16_t>(bits<32768 ? bits : static_cast<int>(bits)-65536);
  }
  return samples;
}
// ScreenCaptureKit supplies planar or interleaved float PCM. Preserve partial
// callbacks; send only complete 5 ms blocks and clamp invalid sample values.
class AudioPacketizer {
public:
  explicit AudioPacketizer(std::function<void(const AudioSamples&)> output):output_(std::move(output)) {}
  void sample(float left,float right) {
    for(float value:{left,right}) {
      if(!std::isfinite(value)) value=0;
      pending_[used_++]=static_cast<std::int16_t>(std::clamp(value,-1.0F,1.0F)*32767);
    }
    if(used_==pending_.size()) { output_(pending_); used_=0; }
  }
private:
  AudioSamples pending_{}; std::size_t used_=0;
  std::function<void(const AudioSamples&)> output_;
};
// Forty milliseconds is a hard freshness/capacity limit, not a mandatory
// playout delay. Output consumes immediately; gaps produce silence locally.
class AudioQueue {
public:
  using Clock=std::chrono::steady_clock;
  void push(AudioSamples samples,Clock::time_point now=Clock::now()) {
    std::lock_guard lock(mutex_);
    if(blocks_.size()>=8) blocks_.pop_front();
    blocks_.push_back({std::move(samples),now});
  }
  std::optional<AudioSamples> pop(Clock::time_point now=Clock::now()) {
    std::lock_guard lock(mutex_);
    while(!blocks_.empty() && now-blocks_.front().queued>std::chrono::milliseconds(40)) blocks_.pop_front();
    if(blocks_.empty()) return {};
    auto samples=std::move(blocks_.front().samples); blocks_.pop_front(); return samples;
  }
  void clear() { std::lock_guard lock(mutex_); blocks_.clear(); }
private:
  struct Block { AudioSamples samples; Clock::time_point queued; };
  std::mutex mutex_; std::deque<Block> blocks_;
};
}
