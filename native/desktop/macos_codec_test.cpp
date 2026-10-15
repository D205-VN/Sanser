#include <iostream>
#include <exception>
namespace sanser::desktop { void testMacVideoCodec(); void testMacBroadcastCodec(); void testMacMetalPresenter(); void writeMacInteropFixture(const char* path); }
int main(int argc,char** argv) {
  try {
    if(argc==3 && std::string(argv[1])=="--write-windows-fixture") { sanser::desktop::writeMacInteropFixture(argv[2]); return 0; }
    sanser::desktop::testMacVideoCodec();
    sanser::desktop::testMacBroadcastCodec();
    std::cout<<"VideoToolbox: synthetic frame encoded, encrypted, transferred, decoded and color-checked\n";
    sanser::desktop::testMacMetalPresenter();
    std::cout<<"Metal: offscreen pixels and aspect ratio verified\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
