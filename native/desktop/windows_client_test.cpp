#include <iostream>
#include <string>
namespace sanser::desktop { void testWindowsInput(); void testWindowsMacFixture(const char*); }
int main(int argc,char** argv) {
  try {
    if(argc!=2) return 2;
    if(std::string(argv[1])=="--input") sanser::desktop::testWindowsInput();
    else sanser::desktop::testWindowsMacFixture(argv[1]);
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
