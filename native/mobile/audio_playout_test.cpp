#include "client.h"
#include <array>
#include <cmath>
#include <iostream>
#include <stdexcept>
void require(bool value,const char *message){if(!value)throw std::runtime_error(message);}
int main(){try{
  auto *pcm=sanser_mobile_pcm_create();require(pcm,"PCM allocation failed");
  std::array<int16_t,480> input{};
  for(size_t i=0;i<240;++i){input[i*2]=16384;input[i*2+1]=-8192;}
  std::array<float,512> left{},right{};
  sanser_mobile_pcm_push(pcm,input.data(),input.size());
  sanser_mobile_pcm_render(pcm,left.data(),right.data(),240);require(left[239]==0,"preroll was not held");
  sanser_mobile_pcm_push(pcm,input.data(),input.size());
  sanser_mobile_pcm_render(pcm,left.data(),right.data(),240);
  require(std::abs(left[239]-.5F)<.00001F && std::abs(right[239]+.25F)<.00001F,"stereo conversion corrupted");
  sanser_mobile_pcm_render(pcm,left.data(),right.data(),512);
  require(left[240]>0 && left[240]<.5F,"underrun clicked to silence");
  for(unsigned i=0;i<100;++i)sanser_mobile_pcm_push(pcm,input.data(),input.size());
  sanser_mobile_pcm_render(pcm,left.data(),right.data(),512);
  for(auto value:left)require(std::isfinite(value) && std::abs(value)<=1,"overflow caused invalid PCM");
  sanser_mobile_pcm_reset(pcm);sanser_mobile_pcm_render(pcm,left.data(),right.data(),512);
  require(std::abs(left[511])<.0001F,"route reset retained stale PCM");
  sanser_mobile_pcm_destroy(pcm);
  std::cout<<"PCM playout: preroll, stereo, underrun fade, bounded overflow and reset passed\n";return 0;
}catch(const std::exception &e){std::cerr<<e.what()<<'\n';return 1;}}
