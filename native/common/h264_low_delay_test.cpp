#include "h264_low_delay.h"
#include <iostream>
#include <stdexcept>

int main() {
  try {
    // Real 1846x1200 VideoToolbox Baseline SPS: POC type 0, one reference,
    // horizontal crop 5, no VUI. The coded geometry must stay unchanged.
    const std::vector<std::uint8_t> source{0x27,0x42,0x00,0x2a,0xab,0x40,0x3a,0x01,0x2f,0xcd,0xa0};
    const auto rewritten=sanser::lowDelayBaselineSps(source);
    if(rewritten==source || rewritten.size()>source.size()+16)
      throw std::runtime_error("Low-delay SPS was not bounded and rewritten");
    if(sanser::lowDelayBaselineSps(rewritten)!=rewritten)
      throw std::runtime_error("Existing VUI was overwritten");
    for(std::size_t i=0;i<source.size();++i) {
      const std::span prefix(source.data(),i);
      if(sanser::lowDelayBaselineSps(prefix)!=std::vector<std::uint8_t>(prefix.begin(),prefix.end()))
        throw std::runtime_error("Truncated SPS was modified");
    }
    for(auto profile:{77,100,244}) {
      auto other=source; other[1]=static_cast<std::uint8_t>(profile);
      if(sanser::lowDelayBaselineSps(other)!=other) throw std::runtime_error("Unknown GOP/profile was rewritten");
    }
    auto pps=source; pps[0]=0x28;
    if(sanser::lowDelayBaselineSps(pps)!=pps) throw std::runtime_error("PPS was mistaken for SPS");
    std::cout<<"H.264 low delay: bounded Baseline SPS rewrite; existing VUI, profiles and truncated input preserved\n";
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
