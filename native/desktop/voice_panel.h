#pragma once
#include "voice_runtime.h"
namespace sanser::desktop {
// A native, nonactivating floating control belongs to the sharing engine, not
// a WebView page. Constructing it never joins voice or opens a microphone.
class VoicePanel {
public:
  // A stop callback identifies the sharing host. It gets one combined panel,
  // even without voice support; viewers only get voice controls.
  VoicePanel(bool enabled,VoiceRuntime& voice,std::function<void()> stopSharing={});
  ~VoicePanel();
  void pump();
private:
  friend void testVoicePanelControls();
  struct Impl; std::unique_ptr<Impl> impl_;
};
}
