#include "audio.h"
#include "tx_scheduler.h"
#include "transport.h"
#include "audio_playout.h"
#include <iostream>
#include <limits>
#include <stdexcept>
using namespace sanser::desktop;
void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
int main() {
  try {
    AudioSamples input{}; input[0]=-32768; input[1]=32767; input[2]=-1;
    require(decodeAudio(encodeAudio(input))==input,"PCM signed/endian roundtrip");
    auto bytes=encodeAudio(input); bytes.pop_back(); require(!decodeAudio(bytes),"truncated audio accepted");
    bytes=encodeAudio(input); bytes.push_back(0); require(!decodeAudio(bytes),"oversized audio accepted");
    bytes=encodeAudio(input); bytes[0]=0; require(!decodeAudio(bytes),"unknown PCM format accepted");
    PacketCodec host(std::string(48,'a'),true),client(std::string(48,'a'),false);
    sanser::snv2::Header header; header.packetType=sanser::snv2::PacketType::Audio; header.sequence=1; header.keyId=42;
    const auto sealed=host.seal(header,encodeAudio(input));
    require(sealed.size()<1200,"PCM packet exceeds conservative MTU");
    const auto opened=client.open(sealed); require(opened && decodeAudio(opened->second)==input,"authenticated audio roundtrip");
    std::vector<AudioSamples> packets;
    AudioPacketizer packetizer([&](const AudioSamples& samples) { packets.push_back(samples); });
    for(unsigned i=0;i<audioFrames-1;++i) packetizer.sample(-2,2);
    require(packets.empty(),"partial callback emitted a short packet");
    packetizer.sample(std::numeric_limits<float>::quiet_NaN(),std::numeric_limits<float>::infinity());
    require(packets.size()==1 && packets[0][0]==-32767 && packets[0][1]==32767 && packets[0].back()==0,"conversion or clamp failed");
    for(unsigned i=0;i<audioFrames;++i) packetizer.sample(0.5F,-0.5F);
    require(packets.size()==2 && packets[1][0]==16383 && packets[1][1]==-16383,"stereo packet boundary failed");
    AudioQueue queue; const auto now=AudioQueue::Clock::now();
    for(unsigned i=0;i<10;++i) { AudioSamples block{}; block[0]=static_cast<std::int16_t>(i); queue.push(block,now); }
    require(queue.pop(now)->front()==2,"slow output did not drop oldest audio");
    require(!queue.pop(now+std::chrono::milliseconds(41)),"stale sound played after stall");
    queue.push(input,now); queue.clear(); require(!queue.pop(now),"device reset retained old sound");
    AudioQueue primed(3);
    primed.push(input,now);
    require(!primed.pop(now),"playback started without jitter reserve");
    primed.push(input,now+std::chrono::milliseconds(5));
    require(!primed.pop(now+std::chrono::milliseconds(5)),"playback alternates packets with silence before priming");
    primed.push(input,now+std::chrono::milliseconds(10));
    require(primed.pop(now+std::chrono::milliseconds(10)).has_value(),"15ms reserve never started");
    require(primed.pop(now+std::chrono::milliseconds(15)).has_value() && primed.pop(now+std::chrono::milliseconds(20)).has_value(),"continuous playback rebuffered between blocks");
    require(!primed.pop(now+std::chrono::milliseconds(25)),"empty output should underrun");
    primed.push(input,now+std::chrono::milliseconds(30));
    require(!primed.pop(now+std::chrono::milliseconds(30)),"underflow did not re-prime");
    require(primed.pop(now+std::chrono::milliseconds(45)).has_value(),"short final audio block was held forever");
    AudioQueue jittered(3); bool began=false; unsigned gaps=0;
    for(unsigned ms=0;ms<1000;++ms) {
      for(unsigned batch=0;batch<100;++batch) {
        const unsigned arrival=batch*10+(batch%3==1 ? 3U : batch%3==2 ? 1U : 0U);
        if(ms==arrival) { jittered.push(input,now+std::chrono::milliseconds(ms)); jittered.push(input,now+std::chrono::milliseconds(ms)); }
      }
      if(ms%5==0) {
        const auto block=jittered.pop(now+std::chrono::milliseconds(ms));
        if(block) began=true; else if(began) ++gaps;
      }
    }
    require(began && gaps==0,"batched 10ms capture with small arrival jitter produced repeated silence");
    sanser::AudioDeviceBudget budget;
    for(unsigned i=0;i<8;++i) { require(budget.fits(48000,120),"20ms output rejected early"); budget.submitted(120); }
    require(!budget.fits(48000,120),"hardware queue exceeded 20ms budget");
    budget.completed(120); require(budget.fits(48000,120),"hardware completion did not restore capacity");
    budget.clear(); require(budget.fits(44100,200),"device reset retained queued audio");
    sanser::AudioContinuity continuity;
    AudioSamples tone; tone.fill(12000);
    continuity.process(tone.data(),audioFrames,audioChannels,audioRate,false);
    require(tone.front()==12000 && tone.back()==12000,"continuous PCM was modified");
    AudioSamples silence{};
    continuity.process(silence.data(),audioFrames,audioChannels,audioRate,true);
    require(silence[0]>11000 && silence[94]==0 && silence.back()==0,"underflow did not ramp down within 1ms");
    tone.fill(-12000); continuity.process(tone.data(),audioFrames,audioChannels,audioRate,true);
    require(tone[0]>-1000 && tone[94]==-12000,"resume did not ramp up from silence");
    std::vector<float> floats(240*2,0.5F);
    floats[0]=std::numeric_limits<float>::quiet_NaN(); floats[1]=2;
    std::vector<std::uint8_t> floatBytes(floats.size()*sizeof(float));
    std::memcpy(floatBytes.data(),floats.data(),floatBytes.size());
    sanser::AudioContinuity floatContinuity;
    floatContinuity.floats(floatBytes.data(),240,2,48000,false);
    std::memcpy(floats.data(),floatBytes.data(),floatBytes.size());
    require(floats[0]==0 && floats[1]==1 && floats[2]==0.5F,"float playback emitted invalid samples or changed valid PCM");
    std::fill(floats.begin(),floats.end(),-0.5F); std::memcpy(floatBytes.data(),floats.data(),floatBytes.size());
    floatContinuity.floats(floatBytes.data(),240,2,48000,true); std::memcpy(floats.data(),floatBytes.data(),floatBytes.size());
    require(floats[0]>0.45F && floats[94]==-0.5F,"lost Legacy packet caused an abrupt PCM edge");
    TxScheduler scheduler;
    for(unsigned i=0;i<10;++i) scheduler.push(TxScheduler::Audio,{{static_cast<std::uint8_t>(i)},now,0});
    scheduler.push(TxScheduler::Video,{{99},now,1});
    scheduler.push(TxScheduler::Control,{{100},now,0});
    scheduler.push(TxScheduler::InputLane,{{101},now,0});
    require(scheduler.pop(true)->bytes.front()==100 && scheduler.pop(true)->bytes.front()==101,"audio blocked control/input");
    for(unsigned i=2;i<10;++i) { const auto audio=scheduler.pop(true); require(audio && audio->audio && audio->bytes.front()==i,"audio priority/capacity incorrect"); }
    require(scheduler.pop(true)->frame==1,"audio did not precede video");
    std::cout<<"Audio: wire, MTU, PCM conversion, bounded freshness and priority passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
