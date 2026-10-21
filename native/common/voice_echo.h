#pragma once
#include <chrono>
#include <cstdint>
#include <memory>
#include <span>

namespace sanser::voice {
using EchoClock=std::chrono::steady_clock;
// Sources are actual rendered PCM, not packets waiting in a network queue.
// FullEndpoint takes precedence over app PCM, avoiding double-counting when
// Windows cannot exclude Sanser from its loopback capture.
enum class RenderSource { Application, External, FullEndpoint };
class PlaybackReference {
public:
  PlaybackReference(); ~PlaybackReference();
  void publish(std::span<const std::int16_t> stereo,EchoClock::time_point at,RenderSource source);
  bool read(std::span<std::int16_t> mono,EchoClock::time_point at);
  static PlaybackReference& shared();
private:
  struct Impl; std::unique_ptr<Impl> impl_;
};
// Smooth callback scheduling jitter while retaining a bounded physical-time
// anchor. Device restarts/gaps reanchor instead of replaying an old reference.
class PlaybackTap {
public:
  explicit PlaybackTap(RenderSource source=RenderSource::Application,PlaybackReference& reference=PlaybackReference::shared())
    :source_(source),reference_(reference) {}
  void publish(std::span<const std::int16_t> stereo,EchoClock::time_point at);
private:
  RenderSource source_; PlaybackReference& reference_; EchoClock::time_point next_{};
};
class EchoCanceller {
public:
  explicit EchoCanceller(PlaybackReference& reference=PlaybackReference::shared());
  ~EchoCanceller();
  void reset();
  // 5ms/48kHz stereo input (dual mono). Process only after explicit mic consent;
  // no devices/files are opened and reference PCM lives in a bounded RAM ring.
  void process(std::span<std::int16_t> stereo,EchoClock::time_point captured);
private:
  struct Impl; std::unique_ptr<Impl> impl_;
};
}
