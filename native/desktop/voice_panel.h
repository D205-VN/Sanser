#pragma once
#include "voice_runtime.h"
namespace sanser::desktop {
// A native, nonactivating floating control belongs to the sharing engine, not
// a WebView page. Constructing it never joins voice or opens a microphone.
class VoicePanel {
public:
  VoicePanel(bool enabled,VoiceRuntime& voice);
  ~VoicePanel();
private:
  friend void testVoicePanelControls();
  struct Impl; std::unique_ptr<Impl> impl_;
};
}
