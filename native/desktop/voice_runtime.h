#pragma once
#include "voice.h"
#include <memory>
namespace sanser::desktop {
// Device ownership and codec/playout run outside the UDP reader and UI. A
// disabled or not-yet-joined runtime never opens an input/output audio device.
class VoiceRuntime {
public:
  VoiceRuntime(bool enabled,std::function<void(voice::Packet)> send,
               std::function<void(voice::State)> state);
  ~VoiceRuntime();
  struct Status { voice::State state; bool capturing=false; std::string code="idle"; };
  void configure(voice::State state);
  Status snapshot() const;
  void receive(voice::Packet packet);
  bool listening() const;
private:
  struct Impl; std::unique_ptr<Impl> impl_;
};
}
