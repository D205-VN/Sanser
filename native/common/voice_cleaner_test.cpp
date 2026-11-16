#include "voice_cleaner.h"
#include <array>
#include <cmath>
#include <cstdint>
#include <iostream>
#include <random>
#include <stdexcept>

using sanser::voice::VoiceCleaner;
namespace {
void check(bool ok,const char* why) { if(!ok) throw std::runtime_error(why); }
using Block=std::array<std::int16_t,480>;
double energy(const Block& b) { double e=0; for(unsigned i=0;i<240;++i) e+=double(b[2*i])*b[2*i]; return e/240; }
// Voiced-speech-like test signal: 140 Hz glottal harmonics with formant-ish
// weights and a 4 Hz syllable envelope.
std::int16_t speech(unsigned n,float scale) {
  const double t=n/48000.0,env=0.55+0.45*std::sin(2*3.141592653589793*4*t);
  double v=0; for(unsigned h=1;h<=24;++h) { const double f=140.0*h; const double w=std::exp(-std::pow((f-700)/450,2))+0.6*std::exp(-std::pow((f-1200)/500,2))+0.2/h; v+=w*std::sin(2*3.141592653589793*f*t); }
  return static_cast<std::int16_t>(std::clamp(v*env*scale,-32768.0,32767.0));
}
}
int main() {
  try {
    std::mt19937 random(7); std::normal_distribution<double> noise(0,1500);
    {
      // Background noise alone: denoise plus gate must remove most of it.
      VoiceCleaner cleaner; double in=0,out=0;
      for(unsigned b=0;b<400;++b) {
        Block block{}; for(unsigned i=0;i<240;++i) block[2*i]=block[2*i+1]=static_cast<std::int16_t>(noise(random));
        const auto before=energy(block); cleaner.process(block);
        if(b>=200) { in+=before; out+=energy(block); }
      }
      const double db=10*std::log10(out/in);
      std::cout<<"Voice cleaner: steady noise reduced by "<<-db<<" dB\n";
      check(db<-20,"background noise was not suppressed by at least 20 dB");
      check(cleaner.stats().gatedBlocks>100,"gate never closed on noise-only input");
    }
    {
      // Speech over noise must survive, and the output must never clip.
      VoiceCleaner cleaner; double in=0,out=0; int peak=0; unsigned n=0;
      for(unsigned b=0;b<400;++b) {
        Block block{};
        for(unsigned i=0;i<240;++i,++n) { const auto v=std::clamp(speech(n,9000)+noise(random)*0.3,-32768.0,32767.0); block[2*i]=block[2*i+1]=static_cast<std::int16_t>(v); }
        const auto before=energy(block); cleaner.process(block);
        if(b>=200) { in+=before; out+=energy(block); for(auto v:block) peak=std::max(peak,std::abs(static_cast<int>(v))); }
      }
      const double db=10*std::log10(out/in);
      std::cout<<"Voice cleaner: speech level change "<<db<<" dB, peak "<<peak<<"\n";
      check(db>-10,"speech was removed along with noise");
      check(peak<=29200,"cleaned speech exceeded the -1 dBFS ceiling");
    }
    {
      // A clipping microphone is limited before Opus.
      VoiceCleaner cleaner; int peak=0; unsigned n=0;
      for(unsigned b=0;b<400;++b) {
        Block block{}; for(unsigned i=0;i<240;++i,++n) block[2*i]=block[2*i+1]=speech(n,40000);
        cleaner.process(block);
        if(b>=100) for(auto v:block) peak=std::max(peak,std::abs(static_cast<int>(v)));
      }
      check(peak<=29200,"limiter let a clipped microphone through at full scale");
      check(cleaner.stats().limitedBlocks>0,"limiter never engaged on a clipping microphone");
    }
    {
      VoiceCleaner cleaner; Block wrong{}; cleaner.reset();
      Block silent{}; cleaner.process(silent);
      for(auto v:silent) check(v==0,"silence must stay silent");
      (void)wrong;
    }
    std::cout<<"Voice cleaner: RNNoise suppression, automatic gate, limiter and speech preservation passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
