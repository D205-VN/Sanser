#pragma once
#include <bit>
#include <cmath>
#include <cstdint>
#include <span>
#include <vector>
namespace sanser::audio {
// WASAPI interleaved mix format. Capture selects the front L/R channels; mono
// is duplicated. Do not reinterpret potentially unaligned device buffers.
inline bool captureStereo(std::span<const std::uint8_t> bytes,unsigned frames,unsigned channels,unsigned bits,bool floating,bool silent,std::vector<float>& stereo) {
  if(channels<1 || channels>8 || (bits!=16 && bits!=24 && bits!=32) || (floating && bits!=32) || frames>192000) return false;
  const auto width=bits/8;
  if(!silent && bytes.size()<static_cast<std::size_t>(frames)*channels*width) return false;
  stereo.resize(static_cast<std::size_t>(frames)*2);
  const auto sample=[&](unsigned frame,unsigned channel) {
    if(silent) return 0.0F;
    const auto offset=(static_cast<std::size_t>(frame)*channels+channel)*width;
    std::uint32_t raw=0;
    for(unsigned i=0;i<width;++i) raw|=static_cast<std::uint32_t>(bytes[offset+i])<<(8*i);
    if(floating) { const auto value=std::bit_cast<float>(raw); return std::isfinite(value) ? value : 0.0F; }
    const auto signedValue=static_cast<std::int64_t>(raw)-(raw&(1U<<(bits-1)) ? (1LL<<bits) : 0);
    return static_cast<float>(signedValue)/static_cast<float>(1ULL<<(bits-1));
  };
  for(unsigned i=0;i<frames;++i) { stereo[i*2]=sample(i,0); stereo[i*2+1]=sample(i,channels>1 ? 1 : 0); }
  return true;
}
}
