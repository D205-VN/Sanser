#pragma once
#include "audio.h"
#include <memory>
namespace sanser::desktop {
class AudioPlayer {
public:
  explicit AudioPlayer(bool enabled);
  ~AudioPlayer();
  void submit(AudioSamples samples);
  void reset();
private:
  struct State;
  std::unique_ptr<State> state_;
};
}
