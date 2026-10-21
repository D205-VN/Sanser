#pragma once
#include "audio.h"
#include <memory>
#include <string>

namespace sanser::desktop {
// Microphone capture is independent of screen/system audio. Construct enabled
// only after an explicit local unmute; disabled never requests OS permission.
// The sink receives 48 kHz stereo PCM16 in fixed 5 ms blocks on a worker thread, with their first-sample time.
// Sink/status callbacks must be short and must not destroy the capture object.
class MicrophoneCapture {
public:
  MicrophoneCapture(bool enabled,std::function<void(const AudioSamples&,std::chrono::steady_clock::time_point)> sink,
                    std::function<void(std::string)> status={});
  ~MicrophoneCapture();
  MicrophoneCapture(const MicrophoneCapture&)=delete;
  MicrophoneCapture& operator=(const MicrophoneCapture&)=delete;
private:
  struct State;
  std::unique_ptr<State> state_;
};
}
