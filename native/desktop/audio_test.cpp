#include "audio.h"
#include "tx_scheduler.h"
#include "transport.h"
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
