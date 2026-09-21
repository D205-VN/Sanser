#include <iostream>
#include <exception>
namespace sanser::desktop { void testMacVideoCodec(); void testMacMetalPresenter(); }
int main() {
  try {
    sanser::desktop::testMacVideoCodec();
    std::cout<<"VideoToolbox: synthetic frame encoded, encrypted, transferred, decoded and color-checked\n";
    sanser::desktop::testMacMetalPresenter();
    std::cout<<"Metal: offscreen pixels and aspect ratio verified\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
