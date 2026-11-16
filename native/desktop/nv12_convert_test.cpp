#include "nv12_convert.h"
#include <algorithm>
#include <chrono>
#include <cstdlib>
#include <iostream>
#include <random>
#include <vector>
using namespace sanser::desktop;
static void require(bool ok,const char* message) { if(!ok) { std::cerr<<message<<'\n'; std::exit(1); } }
// The viewer's original per-pixel conversion, kept as the reference.
static std::vector<std::uint8_t> reference(const std::vector<std::uint8_t>& data,std::uint32_t stride,std::uint32_t rows,
    std::uint32_t left,std::uint32_t top,std::uint32_t width,std::uint32_t height) {
  std::vector<std::uint8_t> pixels(static_cast<std::size_t>(width)*height*4);
  auto clamp=[](int n) { return static_cast<std::uint8_t>(std::clamp(n,0,255)); };
  for(std::uint32_t y=0;y<height;++y) for(std::uint32_t x=0;x<width;++x) {
    const auto px=x+left,py=y+top;
    const int luminance=static_cast<int>(data[static_cast<std::size_t>(py)*stride+px])-16;
    const auto uv=static_cast<std::size_t>(stride)*rows+static_cast<std::size_t>(py/2)*stride+(px&~1U);
    const int u=static_cast<int>(data[uv])-128,v=static_cast<int>(data[uv+1])-128;
    const auto offset=(static_cast<std::size_t>(y)*width+x)*4;
    pixels[offset]=clamp((298*luminance+516*u+128)>>8); pixels[offset+1]=clamp((298*luminance-100*u-208*v+128)>>8);
    pixels[offset+2]=clamp((298*luminance+409*v+128)>>8); pixels[offset+3]=255;
  }
  return pixels;
}
int main() {
  std::mt19937 random(7);
  struct Case { std::uint32_t stride,rows,left,top,width,height; };
  // Even and odd apertures, padded strides, a single column, and macroblock padding (1846 -> 1856).
  for(const auto& c:{Case{64,48,0,0,64,48},Case{80,50,3,5,61,41},Case{80,50,1,1,1,1},Case{34,18,2,3,31,15},
                     Case{1856,1088,0,0,1846,1080},Case{1920,1200,0,0,1920,1200},Case{130,258,1,0,129,257}}) {
    std::vector<std::uint8_t> data(static_cast<std::size_t>(c.stride)*c.rows*3/2);
    for(auto& byte:data) byte=static_cast<std::uint8_t>(random());
    const auto expected=reference(data,c.stride,c.rows,c.left,c.top,c.width,c.height);
    for(unsigned threads:{1U,2U,4U,7U}) {
      std::vector<std::uint8_t> actual(expected.size(),0x5a);
      convertNv12ToBgra(data.data(),c.stride,c.rows,c.left,c.top,c.width,c.height,actual.data(),threads);
      require(actual==expected,"NV12 conversion changed a pixel");
    }
  }
  std::vector<std::uint8_t> frame(static_cast<std::size_t>(1920)*1200*3/2);
  for(auto& byte:frame) byte=static_cast<std::uint8_t>(random());
  std::vector<std::uint8_t> out(static_cast<std::size_t>(1920)*1200*4);
  const auto time=[&](auto&& work) {
    const auto began=std::chrono::steady_clock::now();
    for(unsigned i=0;i<20;++i) work();
    return std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-began).count()/20;
  };
  // Same output buffer for every variant; the reference allocates its own, so
  // copy into `out` to keep the comparison fair.
  (void)time([&] { convertNv12ToBgra(frame.data(),1920,1200,0,0,1920,1200,out.data(),1); });
  const auto single=time([&] { convertNv12ToBgra(frame.data(),1920,1200,0,0,1920,1200,out.data(),1); });
  const auto before=time([&] { const auto pixels=reference(frame,1920,1200,0,0,1920,1200); std::copy(pixels.begin(),pixels.end(),out.begin()); });
  const auto banded=time([&] { convertNv12ToBgra(frame.data(),1920,1200,0,0,1920,1200,out.data(),4); });
  std::cout<<"NV12 1920x1200 per frame: original "<<before<<" ms, one thread "<<single<<" ms, four bands "<<banded<<" ms\n";
  return 0;
}
