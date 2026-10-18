#pragma once
#include "audio.h"
#include <atomic>
#include <thread>

namespace sanser::desktop {
class AudioPlayer {
public:
  explicit AudioPlayer(bool enabled);
  ~AudioPlayer();
  void submit(AudioSamples samples) { queue_.push(std::move(samples)); }
  void reset() { queue_.clear(); }
  AudioQueue::Stats stats() const { return queue_.stats(); }
private:
  void run();
  std::atomic<bool> running_{true};
  AudioQueue queue_{3};
  std::thread worker_;
};
}
