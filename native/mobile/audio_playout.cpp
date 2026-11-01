#include "client.h"
#include <algorithm>
#include <array>
#include <atomic>
#include <new>

struct SanserMobilePCM {
  static constexpr size_t capacity=8192,preroll=480,maximum=1920,target=960;
  struct Frame {float left=0,right=0;};
  std::array<Frame,capacity> ring{};
  std::atomic<size_t> read{0},write{0};
  std::atomic<bool> flush{false},trim{false};
  // Only the render consumer touches these fields.
  bool playing=false;
  unsigned fade=0;
  Frame last{},origin{};
};
extern "C" SanserMobilePCM *sanser_mobile_pcm_create(){return new(std::nothrow) SanserMobilePCM;}
extern "C" void sanser_mobile_pcm_destroy(SanserMobilePCM *p){delete p;}
extern "C" void sanser_mobile_pcm_reset(SanserMobilePCM *p){if(p)p->flush.store(true,std::memory_order_release);}
extern "C" void sanser_mobile_pcm_push(SanserMobilePCM *p,const int16_t *data,size_t samples){
  if(!p || !data || samples==0 || samples%2 || samples/2>SanserMobilePCM::capacity)return;
  const auto w=p->write.load(std::memory_order_relaxed),r=p->read.load(std::memory_order_acquire),n=samples/2;
  if(w-r>SanserMobilePCM::maximum)p->trim.store(true,std::memory_order_release);
  // Never overwrite frames the realtime consumer could still be reading.
  if(w-r+n>SanserMobilePCM::capacity){p->trim.store(true,std::memory_order_release);return;}
  for(size_t i=0;i<n;++i)p->ring[(w+i)%SanserMobilePCM::capacity]={data[i*2]/32768.0F,data[i*2+1]/32768.0F};
  p->write.store(w+n,std::memory_order_release);
}
extern "C" void sanser_mobile_pcm_render(SanserMobilePCM *p,float *left,float *right,size_t frames){
  if(!left || !right)return;
  if(!p){std::fill_n(left,frames,0);std::fill_n(right,frames,0);return;}
  auto r=p->read.load(std::memory_order_relaxed);
  const auto w=p->write.load(std::memory_order_acquire);
  if(p->flush.exchange(false,std::memory_order_acquire)){
    r=w;p->playing=false;p->fade=0;
  }else if(p->trim.exchange(false,std::memory_order_acquire) && w-r>SanserMobilePCM::maximum){
    r=w-SanserMobilePCM::target;p->playing=false;p->fade=0;
  }
  if(!p->playing && w-r>=std::max(SanserMobilePCM::preroll,std::min(frames,SanserMobilePCM::capacity))){
    p->playing=true;p->fade=64;p->origin=p->last;
  }
  for(size_t i=0;i<frames;++i){
    if(p->playing && r<w){
      auto next=p->ring[r++%SanserMobilePCM::capacity];
      if(p->fade){const float mix=(65-p->fade--)/64.0F;next={p->origin.left+(next.left-p->origin.left)*mix,p->origin.right+(next.right-p->origin.right)*mix};}
      p->last=next;
    }else{
      p->playing=false;p->fade=0;
      // A short decay prevents a discontinuity at an underrun/route reset.
      p->last.left*=0.97F;p->last.right*=0.97F;
      if(p->last.left<0.00001F && p->last.left>-0.00001F)p->last.left=0;
      if(p->last.right<0.00001F && p->last.right>-0.00001F)p->last.right=0;
    }
    left[i]=p->last.left;right[i]=p->last.right;
  }
  p->read.store(r,std::memory_order_release);
}
