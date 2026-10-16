#pragma once
#include <algorithm>
#include <cstdint>
#include <span>
#include <stdexcept>
#include <vector>

namespace sanser {
// Only the progressive Baseline stream that our Mac encoder produces with
// AllowFrameReordering=false is eligible. Unknown profiles, existing VUI and
// malformed input pass through unchanged; never guess a different GOP's order.
inline std::vector<std::uint8_t> lowDelayBaselineSps(std::span<const std::uint8_t> nal) {
  const std::vector<std::uint8_t> original(nal.begin(),nal.end());
  if(nal.size()<5 || nal.size()>1024 || (nal[0]&0x9f)!=7) return original;
  struct Bits {
    std::vector<std::uint8_t> values;
    std::size_t at=0;
    std::uint32_t read(unsigned count) {
      if(count>32 || count>values.size()-std::min(at,values.size())) throw std::runtime_error("SPS bounds");
      std::uint32_t value=0; while(count--) value=(value<<1)|values[at++]; return value;
    }
    std::uint32_t ue() {
      unsigned zeros=0;
      while(read(1)==0) if(++zeros>30) throw std::runtime_error("SPS Golomb bounds");
      return ((1U<<zeros)-1)+read(zeros);
    }
    void bit(bool value) { values.push_back(value ? 1 : 0); }
    void writeUe(std::uint32_t value) {
      const auto code=value+1; unsigned width=0; for(auto n=code;n;n>>=1) ++width;
      for(unsigned i=1;i<width;++i) bit(false);
      for(unsigned i=width;i>0;--i) bit((code>>(i-1))&1);
    }
  } bits;
  unsigned zeros=0;
  for(std::size_t i=1;i<nal.size();++i) {
    const auto byte=nal[i];
    if(zeros>=2 && byte==3) {
      if(i+1>=nal.size() || nal[i+1]>3) return original;
      zeros=0; continue;
    }
    for(int shift=7;shift>=0;--shift) bits.bit((byte>>shift)&1);
    zeros=byte==0 ? zeros+1 : 0;
  }
  try {
    if(bits.read(8)!=66) return original; // No high-profile scaling matrices.
    bits.read(8); bits.read(8); // Constraints and level remain untouched.
    if(bits.ue()>31 || bits.ue()>12) return original;
    const auto poc=bits.ue();
    if(poc==0) { if(bits.ue()>12) return original; }
    else if(poc!=2) return original;
    const auto references=bits.ue();
    if(references>16) return original;
    bits.read(1); // gaps_in_frame_num_value_allowed_flag
    if(bits.ue()>255 || bits.ue()>255 || bits.read(1)!=1) return original;
    bits.read(1); // direct_8x8_inference_flag
    if(bits.read(1)) for(unsigned i=0;i<4;++i) bits.ue();
    const auto vui=bits.at;
    if(bits.read(1)) return original; // Preserve any encoder-supplied VUI/HRD.
    if(bits.read(1)!=1) return original;
    while(bits.at<bits.values.size()) if(bits.read(1)!=0) return original;
    bits.values.resize(vui); bits.bit(true); // vui_parameters_present_flag
    for(unsigned i=0;i<8;++i) bits.bit(false); // No optional aspect/color/timing/HRD/pic_struct.
    bits.bit(true); bits.bit(true); // bitstream_restriction, motion vectors over boundaries
    bits.writeUe(0); bits.writeUe(0); // Do not impose tighter picture/macroblock size limits.
    bits.writeUe(16); bits.writeUe(16); // maximum motion-vector lengths
    bits.writeUe(0); // max_num_reorder_frames: output without waiting for future input
    bits.writeUe(std::max(1U,references)); // Keep every advertised reference buffer.
    bits.bit(true); while(bits.values.size()%8) bits.bit(false); // rbsp_trailing_bits
    std::vector<std::uint8_t> result{nal[0]}; zeros=0;
    for(std::size_t i=0;i<bits.values.size();i+=8) {
      std::uint8_t byte=0; for(unsigned n=0;n<8;++n) byte=static_cast<std::uint8_t>((byte<<1)|bits.values[i+n]);
      if(zeros>=2 && byte<=3) { result.push_back(3); zeros=0; }
      result.push_back(byte); zeros=byte==0 ? zeros+1 : 0;
    }
    return result;
  } catch(const std::runtime_error&) { return original; }
}
}
