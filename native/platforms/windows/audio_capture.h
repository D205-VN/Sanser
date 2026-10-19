#pragma once
#include "audio.h"
#include <atomic>
#include <thread>
namespace sanser::desktop {
class AudioCapture {
public:
  // excludeSelf filters this process tree from system playback capture where
  // the OS supports process loopback. The callback reports actual activation;
  // callers must suppress system audio during voice playback when it is false.
  AudioCapture(bool enabled,std::function<void(const AudioSamples&)> sink,
               bool excludeSelf=false,std::function<void(bool)> exclusionStatus={});
  ~AudioCapture();
private:
  void run();
  void reportExclusion(bool excluded) noexcept;
  std::function<void(const AudioSamples&)> sink_;
  bool excludeSelf_=false;
  std::function<void(bool)> exclusionStatus_;
  std::atomic<bool> running_{true};
  std::thread worker_;
};
#if defined(SANSER_CODEC_TEST)
bool testWindowsAudioCaptureExclusion();
#endif
}
