#pragma once
#include "audio.h"
#include <atomic>
#include <thread>
namespace sanser::desktop {
class AudioCapture {
public:
  AudioCapture(bool enabled,std::function<void(const AudioSamples&)> sink);
  ~AudioCapture();
private:
  void run();
  std::function<void(const AudioSamples&)> sink_;
  std::atomic<bool> running_{true};
  std::thread worker_;
};
}
