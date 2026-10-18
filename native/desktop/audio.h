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
// Forty milliseconds is a hard freshness/capacity limit. Playback may request
// a short priming reserve after an underflow, rather than alternating PCM and
// silence on every slightly late 5ms packet.
class AudioQueue {
public:
  using Clock=std::chrono::steady_clock;
  struct PlayoutBlock { AudioSamples samples; bool discontinuity=false; };
  struct Stats { std::uint64_t underflows=0,capacityDrops=0,staleDrops=0,reprimes=0,resets=0; };
  explicit AudioQueue(unsigned startupBlocks=1):startupBlocks_(std::clamp(startupBlocks,1U,8U)) {}
  void push(AudioSamples samples,Clock::time_point now=Clock::now()) {
    std::lock_guard lock(mutex_);
    if(blocks_.size()>=8) { blocks_.pop_front(); discontinuity_=true; ++stats_.capacityDrops; }
    blocks_.push_back({std::move(samples),now});
  }
  std::optional<AudioSamples> pop(Clock::time_point now=Clock::now()) {
    auto block=popBlock(now);
    if(!block) return {};
    return std::move(block->samples);
  }
  // Overflow, stale eviction and a generation/device reset skip PCM without
  // necessarily returning an empty block. Preserve that edge for the player:
  // otherwise its existing underflow ramp cannot prevent an audible click.
  std::optional<PlayoutBlock> popBlock(Clock::time_point now=Clock::now()) {
    std::lock_guard lock(mutex_);
    while(!blocks_.empty() && now-blocks_.front().queued>std::chrono::milliseconds(40)) { blocks_.pop_front(); discontinuity_=true; ++stats_.staleDrops; }
    if(blocks_.empty()) { if(started_) ++stats_.underflows; started_=false; discontinuity_=true; return {}; }
    if(!started_ && blocks_.size()<startupBlocks_ && now-blocks_.front().queued<std::chrono::milliseconds(5*startupBlocks_)) return {};
    if(!started_ && everStarted_) ++stats_.reprimes;
    started_=everStarted_=true;
    PlayoutBlock block{std::move(blocks_.front().samples),discontinuity_};
    blocks_.pop_front(); discontinuity_=false; return block;
  }
  void clear() { std::lock_guard lock(mutex_); blocks_.clear(); started_=false; discontinuity_=true; ++stats_.resets; }
  Stats stats() const { std::lock_guard lock(mutex_); return stats_; }
private:
  unsigned startupBlocks_; bool started_=false,everStarted_=false,discontinuity_=true;
  struct Block { AudioSamples samples; Clock::time_point queued; };
  mutable std::mutex mutex_; std::deque<Block> blocks_; Stats stats_;
};
}
