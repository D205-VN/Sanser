#include "audio_capture_pcm.h"
#include <algorithm>
#include <cmath>
#include <iostream>
#include <stdexcept>
using sanser::audio::captureStereo;
void require(bool ok) { if(!ok) throw std::runtime_error("Audio capture sample conversion failed"); }
int main() {
  try {
    std::vector<float> out;
    require(captureStereo(std::vector<std::uint8_t>{0,128,255,127},2,1,16,false,false,out));
    require(out.size()==4 && out[0]==-1 && out[1]==-1 && out[2]>0.99F && out[3]==out[2]);
    require(captureStereo(std::vector<std::uint8_t>{0,0,128,255,255,127},1,2,24,false,false,out));
    require(out[0]==-1 && out[1]>0.99F);
    require(captureStereo(std::vector<std::uint8_t>{0,0,0,128,255,255,255,127},1,2,32,false,false,out));
    require(out[0]==-1 && out[1]>0.99F);
    require(captureStereo(std::vector<std::uint8_t>{0,0,128,63,0,0,192,127},1,2,32,true,false,out));
    require(out[0]==1 && out[1]==0); // NaN from a device must never enter Opus.
    require(captureStereo({},240,8,32,true,true,out));
    require(out.size()==480 && std::all_of(out.begin(),out.end(),[](auto v) { return v==0; }));
    require(!captureStereo({},2,2,16,false,false,out));
    require(!captureStereo({},240,0,32,true,true,out));
    require(!captureStereo({},240,2,24,true,true,out));
    std::cout<<"Capture PCM: float/non-finite, mono, stereo 16/24/32-bit, silent and truncated buffers passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
