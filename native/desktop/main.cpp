#include "engine.h"
#include "sanser_version.h"
#include <iostream>
#include <stdexcept>
#include <string_view>
#ifndef SANSER_ENGINE_HOST
#define SANSER_ENGINE_HOST 0
#endif
int main(int argc,char** argv) {
  constexpr bool host=SANSER_ENGINE_HOST!=0;
#ifdef __APPLE__
  const char* engine=host?"host-macos":"client-macos";
#else
  const char* engine=host?"host-windows":"client-windows";
#endif
  try {
#ifdef __APPLE__
    if (host && !sanser::desktop::macHostAvailable()) throw std::runtime_error("Hosting requires macOS 12.3 or newer");
#endif
    if(argc==2 && std::string_view(argv[1])=="--capabilities-json") {
      std::cout<<"{\"product\":\"Sanser\",\"version\":\""<<sanser::kVersion<<"\",\"protocolVersion\":2,\"engine\":\""<<engine<<"\",\"nativeSnv2\":true,\"nativeDirect\":true,\"crossPlatform\":true,\"h264EncoderImplementation\":"<<(host?"true":"false")<<",\"h264DecoderImplementation\":"<<(host?"false":"true")<<",\"hevcEncoderImplementation\":false,\"hevcDecoderImplementation\":false,\"audioImplementation\":false,\"inputImplementation\":true}\n"; return 0;
    }
#ifdef __APPLE__
    return sanser::desktop::runMac(host,argc,argv);
#else
    return sanser::desktop::runWindows(host,argc,argv);
#endif
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
