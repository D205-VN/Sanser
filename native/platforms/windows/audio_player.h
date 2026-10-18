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
private:
  void run();
  std::atomic<bool> running_{true};
  AudioQueue queue_{3};
  std::thread worker_;
};
}
