#include <iostream>
#include <string>
namespace sanser::desktop { bool testWindowsAudioCaptureExclusion(); void testWindowsInput(); void testWindowsMacFixture(const char*,bool=false); }
int main(int argc,char** argv) {
  try {
    if(argc==3 && std::string(argv[1])=="--low-delay") { sanser::desktop::testWindowsMacFixture(argv[2],true); return 0; }
    if(argc!=2) return 2;
    if(std::string(argv[1])=="--audio-exclusion") return sanser::desktop::testWindowsAudioCaptureExclusion() ? 0 : 1;
    if(std::string(argv[1])=="--input") sanser::desktop::testWindowsInput();
    else sanser::desktop::testWindowsMacFixture(argv[1]);
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
