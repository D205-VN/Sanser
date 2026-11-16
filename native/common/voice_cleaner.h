#pragma once
#include <cstdint>
#include <memory>
#include <span>

namespace sanser::voice {
// Microphone clean-up after echo cancellation, before Opus:
//  1. RNNoise (Xiph, BSD) removes steady and transient background noise
//     (fans, keyboards, traffic) from each 10 ms frame.
//  2. An automatic gate uses RNNoise's own voice-activity estimate, so no
//     sensitivity setting is needed: it opens fast on speech, holds 300 ms,
//     then fades to -30 dB, so pauses carry no residual noise.
//  3. A peak limiter (fast attack, slow release) keeps the cleaned signal
//     below -1 dBFS so Opus never encodes digital clipping.
// Works on interleaved 48 kHz stereo 5 ms blocks, in place, with a fixed
// 10 ms delay. Single-threaded: call from the voice worker only.
class VoiceCleaner {
public:
  struct Stats { std::uint64_t frames=0,gatedBlocks=0,limitedBlocks=0; float lastVoiceProbability=0; };
  VoiceCleaner();
  ~VoiceCleaner();
  VoiceCleaner(const VoiceCleaner&)=delete; VoiceCleaner& operator=(const VoiceCleaner&)=delete;
  void process(std::span<std::int16_t> stereoBlock);
  void reset();
  Stats stats() const;
private:
  struct Impl; std::unique_ptr<Impl> impl_;
};
}
