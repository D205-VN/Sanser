#include <exception>
#include <iostream>
namespace sanser::desktop { void testVoicePanelControls(); }
int main() {
  try {
    sanser::desktop::testVoicePanelControls();
    std::cout<<"Floating voice: expand, listen-only join, explicit unmute, deafen and leave passed using a fake device-free controller\n";
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
