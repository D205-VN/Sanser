#include "opus_audio.h"
#include <cmath>
#include <atomic>
#include <algorithm>
#include <iostream>
#include <stdexcept>
using namespace sanser::audio;
void check(bool value,const char* why) { if(!value) throw std::runtime_error(why); }
int main() {
 try {
  Encoder encoder; Decoder decoder;
  Samples samples{};
  std::vector<Packet> packets;
  for(unsigned p=0;p<40;++p) {
    for(unsigned f=0;f<frames;++f) { const auto value=static_cast<std::int16_t>(12000*std::sin((p*frames+f)*2*3.141592653589793*440/rate)); samples[f*2]=value; samples[f*2+1]=-value; }
    auto bytes=encoder.encode(samples); check(bytes.size()<200,"Opus payload exceeds bitrate budget");
    auto decoded=decoder.decode(bytes); check(decoded.has_value(),"Opus decoder rejected encoded block");
    packets.push_back({p+1,p*frames,1000000+p*5000,std::move(bytes)});
  }
  auto body=pack(packets.front()); const auto parsed=unpack(body);
  check(parsed && parsed->sequence==1 && parsed->timestamp==0 && parsed->payload==packets[0].payload,"Opus wire roundtrip");
  body[19]=1; check(!unpack(body),"Inconsistent audio timestamp accepted");
  check(!decoder.decode(Bytes{0xff,0xff}),"Malformed Opus accepted");
  const auto now=Jitter::Clock::time_point{}+std::chrono::seconds(1);
  Jitter jitter;
  check(jitter.push(packets[0],now),"Push initial audio");
  check(jitter.push(packets[2],now),"Push out-of-order audio");
  check(jitter.push(packets[1],now),"Repair reorder before deadline");
  check(!jitter.push(packets[1],now),"Duplicate packet queued");
  check(!jitter.poll(now+std::chrono::milliseconds(9)),"Audio started without reorder reserve");
  for(unsigned i=0;i<3;++i) { auto d=jitter.poll(now+std::chrono::milliseconds(10+i*5)); check(d && d->sequence==i+1 && !d->concealed,"Reordered audio lost or reordered on playback"); }
  check(jitter.push(packets[4],now+std::chrono::milliseconds(20)),"Future packet queued");
  auto missing=jitter.poll(now+std::chrono::milliseconds(25));
  check(missing && missing->sequence==4 && missing->concealed && missing->unixMicros==1015000,"Missing block did not invoke timed PLC");
  bool plcSignal=false; for(auto sample:missing->samples) if(std::abs(static_cast<int>(sample))>100) plcSignal=true;
  check(plcSignal,"Packet loss concealment was replaced by silence");
  auto recovered=jitter.poll(now+std::chrono::milliseconds(30)); check(recovered && recovered->sequence==5 && !recovered->concealed,"Audio did not resume after PLC");
  check(!jitter.push(packets[3],now+std::chrono::milliseconds(31)),"Expired audio replayed after PLC");
  check(jitter.reserveMs()==20,"A late packet must grow the playout reserve");
  // Two stretch blocks for the grown reserve, then at most six PLC blocks.
  for(unsigned i=0;i<8;++i) check(jitter.poll(now+std::chrono::milliseconds(35+i*5)).has_value(),"Bounded PLC ended early");
  check(!jitter.poll(now+std::chrono::milliseconds(75)),"PLC continued indefinitely after source stopped");
  check(!jitter.push(packets[3],now+std::chrono::milliseconds(80)),"Retired audio replayed after PLC became idle");
  jitter.reset(); check(jitter.push(packets[0],now+std::chrono::seconds(1)),"Restart sequence rejected");
  check(!jitter.poll(now+std::chrono::seconds(1)+std::chrono::milliseconds(10)),"Restart ignored the learned reserve");
  auto fresh=jitter.poll(now+std::chrono::seconds(1)+std::chrono::milliseconds(20)); check(fresh && fresh->sequence==1 && !fresh->concealed,"Restart retained old decoder sequence");
  Jitter stalled; stalled.push(packets[0],now); check(!stalled.poll(now+std::chrono::milliseconds(50)),"Stale audio survived 40ms age limit");
  Jitter backlog;
  backlog.push(packets[0],now); backlog.push(packets[1],now);
  check(backlog.poll(now+std::chrono::milliseconds(10)).has_value(),"Backlog test did not start");
  backlog.push(packets[2],now+std::chrono::milliseconds(25));
  // Due is 15ms, so the 36ms scheduler delay is below the old 40ms
  // overdue guard, while packet 2 itself has expired. Never PLC that packet.
  auto resumed=backlog.poll(now+std::chrono::milliseconds(51));
  check(resumed && resumed->sequence==3 && !resumed->concealed && backlog.stats().concealed==0,
    "Freshness discard must skip stale playout time instead of concealing retired audio");
  for(unsigned i=3;i<40;++i) {
    const auto at=now+std::chrono::milliseconds(51+(i-2)*5);
    backlog.push(packets[i],at);
    auto block=backlog.poll(at);
    check(block && block->sequence==i+1 && !block->concealed,"Fresh audio remained stuck behind an expired backlog");
  }
  {
    // Wi-Fi jitter: every 4th block arrives 25ms late. A fixed 10ms reserve
    // concealed each of them; the adaptive reserve stops concealing.
    Jitter wifi; unsigned lateAfterWarmup=0;
    const auto make=[&](unsigned i) { return Packet{i+1,std::uint64_t{i}*frames,0,packets[i%40].payload}; };
    auto arrival=[&](unsigned i) { return now+std::chrono::milliseconds(i*5+(i%4==3 ? 25 : 0)); };
    std::vector<std::pair<Jitter::Clock::time_point,unsigned>> schedule;
    for(unsigned i=0;i<600;++i) schedule.push_back({arrival(i),i});
    std::sort(schedule.begin(),schedule.end());
    std::size_t next=0; auto clock=now;
    for(unsigned tick=0;tick<3200;++tick,clock+=std::chrono::milliseconds(1)) {
      while(next<schedule.size() && schedule[next].first<=clock) {
        const auto before=wifi.stats().late; wifi.push(make(schedule[next].second),clock);
        if(schedule[next].second>=300 && wifi.stats().late>before) ++lateAfterWarmup; ++next;
      }
      (void)wifi.poll(clock);
    }
    check(wifi.reserveMs()>=30 && lateAfterWarmup==0,"Adaptive reserve did not absorb periodic 25ms jitter");
    // Phone Wi-Fi power save, as measured: a ~100 ms delivery stall every
    // 2 s, then the held blocks arrive together. The reserve must grow past
    // the old 60 ms ceiling and the burst must not be flushed as stale.
    Jitter spikes; std::uint64_t concealedBefore=0,droppedBefore=0;
    std::vector<std::pair<Jitter::Clock::time_point,unsigned>> bursty;
    for(unsigned i=0;i<4000;++i) {
      const auto sent=i*5,window=sent/2000*2000;
      const auto stalled=sent-window<100 ? window+100 : sent;
      bursty.push_back({now+std::chrono::milliseconds(stalled),i});
    }
    std::sort(bursty.begin(),bursty.end());
    next=0; clock=now;
    for(unsigned tick=0;tick<20200;++tick,clock+=std::chrono::milliseconds(1)) {
      while(next<bursty.size() && bursty[next].first<=clock) { spikes.push(make(bursty[next].second),clock); ++next; }
      (void)spikes.poll(clock);
      if(tick==6000) { concealedBefore=spikes.stats().concealed; droppedBefore=spikes.stats().dropped; }
    }
    check(spikes.reserveMs()>60,"Wi-Fi stalls could not grow the reserve past 60 ms");
    // 2800 blocks after warm-up; allow the occasional calm-shrink retry.
    check(spikes.stats().concealed-concealedBefore<=28 && spikes.stats().dropped-droppedBefore<=28,
      "Audio kept concealing or dropping after adapting to 100 ms Wi-Fi stalls");
    // A calm path gives the delay back gradually, without PLC.
    Jitter calm; calm.push(packets[0],now);
    for(unsigned i=0;i<4;++i) (void)calm.poll(now+std::chrono::milliseconds(10+i*5));
    calm.push(packets[1],now+std::chrono::milliseconds(30));
    const auto grown=calm.reserveMs(); check(grown>10,"Late arrival did not grow the reserve");
    Jitter::Clock::time_point t=now+std::chrono::seconds(10); calm.reset();
    std::uint64_t base=0;
    for(unsigned i=0;i<3;++i) calm.push(packets[i],t);
    for(unsigned i=0;i<2000 && calm.reserveMs()==grown;++i,t+=std::chrono::milliseconds(5)) {
      calm.push({packets[0].sequence+base+3+i,(base+3+i)*frames,0,packets[(i+3)%40].payload},t);
      (void)calm.poll(t);
    }
    check(calm.reserveMs()==grown-5,"Calm playout did not shrink the reserve");
  }
  Samples silence{}; Encoder quiet; Jitter silent;
  check(silent.push({1,0,0,quiet.encode(silence)},now),"Valid Opus silence/mono coding rejected for stereo output");
  check(silent.poll(now+std::chrono::milliseconds(10)).has_value(),"Opus silence did not decode");
  std::atomic<unsigned> firstEpoch{0},secondEpoch{0};
  ReceiveWorker worker([&](Decoded block) { if(block.generation==17) ++firstEpoch; if(block.generation==18) ++secondEpoch; });
  worker.reset(17); check(worker.push(packets[0],17),"Active audio epoch rejected");
  const auto deadline=Jitter::Clock::now()+std::chrono::seconds(2);
  while(!firstEpoch && Jitter::Clock::now()<deadline) std::this_thread::sleep_for(std::chrono::milliseconds(1));
  check(firstEpoch>0,"Audio decode worker did not publish PCM");
  worker.reset(18); const auto retired=firstEpoch.load();
  check(!worker.push(packets[1],17),"Retired epoch returned to playout");
  check(worker.push(packets[0],18),"Fresh epoch could not restart at sequence 1");
  while(!secondEpoch && Jitter::Clock::now()<deadline) std::this_thread::sleep_for(std::chrono::milliseconds(1));
  check(secondEpoch>0 && firstEpoch==retired,"Audio reset leaked old-epoch PCM");
  for(unsigned rateIn:{44100U,48000U,96000U}) {
    std::vector<Samples> whole,split;
    std::vector<float> input(rateIn/10*2);
    for(std::size_t i=0;i<input.size();i+=2) { input[i]=static_cast<float>(0.4*std::sin(i*0.015)); input[i+1]=-input[i]; }
    Packetizer one([&](const Samples& s){whole.push_back(s);}),many([&](const Samples& s){split.push_back(s);});
    check(one.push(input,rateIn),"Valid capture rate rejected");
    for(std::size_t i=0;i<input.size();) { const auto n=std::min<std::size_t>(input.size()-i,74); check(many.push({input.data()+i,n},rateIn),"Split callback rejected"); i+=n; }
    check(whole==split && whole.size()>=19 && whole.size()<=20,"Callback boundaries changed resampled PCM");
    many.reset(); check(!many.push(input,0),"Invalid sample rate accepted");
  }
  std::cout<<"Opus: stereo codec, 96kbps budget, wire validation, reorder, PLC, restart, freshness and 44.1/48/96kHz streaming resampling passed\n";
  return 0;
 } catch(const std::exception& e) { std::cerr<<e.what()<<'\n'; return 1; }
}
