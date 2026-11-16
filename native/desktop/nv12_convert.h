#pragma once
#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <thread>
#include <vector>

namespace sanser::desktop {
// BT.601 limited-range NV12 to BGRA for the Windows viewer's software path.
// `source` holds `rows` luma rows of `stride` bytes followed by interleaved UV
// at the same stride; the visible aperture starts at (left, top). The caller
// validates the geometry. Output is byte-identical to the original per-pixel
// loop; rows are split across a few threads.
inline void convertNv12Rows(const std::uint8_t* __restrict source,std::uint32_t stride,std::uint32_t rows,
                            std::uint32_t left,std::uint32_t top,std::uint32_t width,
                            std::uint32_t firstRow,std::uint32_t lastRow,std::uint8_t* __restrict output) {
  // The original loop body, restricted to a band of rows. __restrict matters:
  // uint8_t may alias anything, so without it every store reloads the input.
  const auto clamp=[](int n) { return static_cast<std::uint8_t>(std::clamp(n,0,255)); };
  for(std::uint32_t y=firstRow;y<lastRow;++y) for(std::uint32_t x=0;x<width;++x) {
    const auto px=x+left,py=y+top;
    const int luminance=static_cast<int>(source[static_cast<std::size_t>(py)*stride+px])-16;
    const auto uv=static_cast<std::size_t>(stride)*rows+static_cast<std::size_t>(py/2)*stride+(px&~1U);
    const int u=static_cast<int>(source[uv])-128,v=static_cast<int>(source[uv+1])-128;
    const auto offset=(static_cast<std::size_t>(y)*width+x)*4;
    output[offset]=clamp((298*luminance+516*u+128)>>8); output[offset+1]=clamp((298*luminance-100*u-208*v+128)>>8);
    output[offset+2]=clamp((298*luminance+409*v+128)>>8); output[offset+3]=255;
  }
}

inline void convertNv12ToBgra(const std::uint8_t* source,std::uint32_t stride,std::uint32_t rows,
                              std::uint32_t left,std::uint32_t top,std::uint32_t width,std::uint32_t height,
                              std::uint8_t* output,unsigned maxThreads=4) {
  // Small pictures are not worth a thread; 1080p splits into at most four bands.
  const auto hardware=std::max(1U,std::thread::hardware_concurrency());
  const auto bands=std::clamp<unsigned>(std::min(maxThreads,hardware),1,std::max<unsigned>(1,height/128));
  if(bands<=1) { convertNv12Rows(source,stride,rows,left,top,width,0,height,output); return; }
  std::vector<std::thread> workers;
  workers.reserve(bands-1);
  const auto step=height/bands;
  for(unsigned band=1;band<bands;++band) {
    const auto first=band*step,last=band+1==bands ? height : (band+1)*step;
    workers.emplace_back([=] { convertNv12Rows(source,stride,rows,left,top,width,first,last,output); });
  }
  convertNv12Rows(source,stride,rows,left,top,width,0,step,output);
  for(auto& worker:workers) worker.join();
}
}
