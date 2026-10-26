#include "voice_echo.h"
#include <algorithm>
#include <array>
#include <cmath>
#include <iostream>
#include <random>
#include <stdexcept>
#include <vector>
using namespace sanser::voice;
namespace {
void require(bool ok,const char* why) { if(!ok) throw std::runtime_error(why); }
constexpr unsigned frames=240,rate=48000;
using Block=std::array<std::int16_t,frames*2>;
auto timeAt(unsigned frame) { return EchoClock::time_point{}+std::chrono::seconds(100)+std::chrono::microseconds(static_cast<std::uint64_t>(frame)*1000000/rate); }
}
int main() {
  try {
    PlaybackReference reference;
    Block app{},external{},full{}; app.fill(2000); external.fill(3000); full.fill(5000);
    reference.publish(app,timeAt(0),RenderSource::Application);
    reference.publish(external,timeAt(0),RenderSource::External);
    std::array<std::int16_t,frames> mono{};
    require(reference.read(mono,timeAt(0)) && mono[100]==5000,"reference combines system and voice actually rendered");
    reference.publish(full,timeAt(0),RenderSource::FullEndpoint);
    require(reference.read(mono,timeAt(0)) && mono[100]==5000,"full endpoint loopback overrides app reference instead of counting voice twice");
    require(!reference.read(mono,timeAt(rate*2)),"old render PCM cannot serve as a fresh reference");
    PlaybackReference batched; PlaybackTap tap(RenderSource::Application,batched);
    for(unsigned callback=0;callback<50;++callback) {
      // Device periods deliver two blocks, with +/-2ms callback jitter. A
      // timestamp per callback must not insert holes or repeat whole blocks.
      const auto jitter=std::chrono::milliseconds(static_cast<int>(callback%5)-2);
      for(unsigned chunk=0;chunk<2;++chunk) tap.publish(app,timeAt(callback*480)+jitter);
    }
    require(batched.read(mono,timeAt(12000)-std::chrono::milliseconds(2)),"batched render callbacks retain a continuous sample timeline");
    // Silent/missing render reference is common while only the local mic is
    // speaking. Both paths must emit the same previous 5ms, not alternate
    // current dry PCM with a newly initialized/delayed WOLA frame.
    for(unsigned missing:{0U,1U,4U,5U}) {
      PlaybackReference intermittent; EchoCanceller local(intermittent);
      Block previous{},silence{}; unsigned lostBlocks=0;
      for(unsigned block=0;block<200;++block) {
        Block speech{};
        for(unsigned i=0;i<frames;++i) speech[i*2]=speech[i*2+1]=static_cast<std::int16_t>(12000*std::sin((block*frames+i)*0.06544984695));
        const auto at=timeAt(block*frames);
        if(block%5>=missing) intermittent.publish(silence,at,RenderSource::Application);
        auto clean=speech; local.process(clean,at);
        require(clean==previous,"silent/intermittent reference changed the near-end sample timeline");
        if(block && std::all_of(clean.begin(),clean.end(),[](auto value) { return value==0; })) ++lostBlocks;
        previous=speech;
      }
      require(lostBlocks==0 && local.stats().resets<=1,"brief missing references repeatedly reset or blanked microphone blocks");
      std::cout<<"Near-only missing reference blocks/5="<<missing<<" blanked5msBlocks="<<lostBlocks<<'\n';
    }
    {
      PlaybackReference holes; EchoCanceller live(holes);
      Block previous{};
      for(unsigned block=0;block<400;++block) {
        Block speech{},render{};
        for(unsigned i=0;i<frames;++i) {
          speech[i*2]=speech[i*2+1]=static_cast<std::int16_t>(12000*std::sin((block*frames+i)*0.06544984695));
          render[i*2]=render[i*2+1]=static_cast<std::int16_t>(2000*std::sin((block*frames+i)*0.1178097245));
        }
        const auto at=timeAt(block*frames);
        if(block%5==0) holes.publish(render,at,RenderSource::Application);
        auto clean=speech; live.process(clean,at);
        if(block>20) {
          double originalEnergy=0,cleanEnergy=0;
          for(unsigned i=0;i<frames;++i) { originalEnergy+=static_cast<double>(previous[i*2])*previous[i*2]; cleanEnergy+=static_cast<double>(clean[i*2])*clean[i*2]; }
          require(cleanEnergy>originalEnergy*0.25,"recovering an active playback reference blanked near-end speech");
        }
        previous=speech;
      }
      require(live.stats().resets==0,"brief active-reference holes erased acoustic adaptation");
    }
    // Feed two independent render streams, delayed multipath acoustic echo,
    // then independent near-end speech while far-end playback continues.
    constexpr unsigned seconds=12,count=rate*seconds;
    std::vector<double> far(count),near(count);
    std::mt19937 random(7331); std::uniform_real_distribution<double> noise(-1,1);
    double low=0;
    for(unsigned i=0;i<count;++i) {
      low=0.65*low+0.35*noise(random);
      far[i]=9500*low+2600*std::sin(i*0.027);
      near[i]=i>=rate*8 ? 3200*std::sin(i*0.051)+1800*std::sin(i*0.081) : 0;
    }
    for(unsigned delayMs:{0U,10U,20U,30U,60U,100U}) {
    PlaybackReference acoustic; EchoCanceller echo(acoustic);
    double originalEnergy=0,cleanEnergy=0,nearEnergy=0,cross=0;
    for(unsigned base=0;base<count;base+=frames) {
      Block music{},voice{},mic{};
      for(unsigned i=0;i<frames;++i) {
        const auto n=base+i;
        music[i*2]=music[i*2+1]=static_cast<std::int16_t>(far[n]*0.6);
        voice[i*2]=voice[i*2+1]=static_cast<std::int16_t>(far[n]*0.4);
        const auto delayed=[&](unsigned delay) { return n>=delay ? far[n-delay] : 0.0; };
        const auto signal=near[n]+0.53*delayed(rate*delayMs/1000)+0.17*delayed(rate*(delayMs+19)/1000);
        mic[i*2]=mic[i*2+1]=static_cast<std::int16_t>(signal);
      }
      acoustic.publish(music,timeAt(base),RenderSource::Application);
      acoustic.publish(voice,timeAt(base),RenderSource::Application);
      auto clean=mic; echo.process(clean,timeAt(base));
      if(base>=rate*6 && base<rate*8) for(unsigned i=0;i<frames;++i) {
        originalEnergy+=static_cast<double>(mic[i*2])*mic[i*2]; cleanEnergy+=static_cast<double>(clean[i*2])*clean[i*2];
      }
      if(base>=rate*10) for(unsigned i=0;i<frames;++i) { nearEnergy+=near[base+i]*near[base+i]; cross+=clean[i*2]*near[base+i]; }
    }
    const auto reduction=10*std::log10(originalEnergy/std::max(1.0,cleanEnergy));
    const auto nearGain=cross/nearEnergy;
    std::cout<<"AEC delay="<<delayMs<<"ms echo reduction="<<reduction<<"dB near-end gain="<<nearGain<<'\n';
    require(reduction>15,"AEC must cancel delayed speaker echo, not merely gate/attenuate the microphone");
    require(nearGain>0.45 && nearGain<1.3,"AEC must preserve independent near-end speech during double talk");
    Block speech{}; speech.fill(6000); auto unchanged=speech;
    echo.process(unchanged,timeAt(count+rate*2));
    require(std::all_of(unchanged.begin(),unchanged.end(),[](auto value) { return value==0; }),"capture gap must clear the delayed block");
    unchanged=speech; echo.process(unchanged,timeAt(count+rate*2+frames));
    require(unchanged==speech,"missing/stale reference retains speech after the fixed one-block warmup");
    echo.reset(); unchanged=speech; echo.process(unchanged,timeAt(count+rate*3));
    require(std::all_of(unchanged.begin(),unchanged.end(),[](auto value) { return value==0; }),"mute/new capture reused delayed microphone PCM");
    unchanged=speech; echo.process(unchanged,timeAt(count+rate*3+frames));
    require(unchanged==speech,"new capture did not resume delayed dry speech");
    }
    std::cout<<"AEC: mixed render references, full-loopback precedence, delay/multipath, double talk and stale-reference safety passed\n";
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
