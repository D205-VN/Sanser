#include "transport.h"
#include <atomic>
#include <iostream>
#include <memory>
#include <random>
#include <stdexcept>
#ifdef _WIN32
#include <winsock2.h>
#else
#include <sys/socket.h>
#include <cerrno>
#endif
using namespace sanser::desktop;
namespace {
std::atomic<bool> loseNextOriginal{false};
std::atomic<unsigned> originalStallMs{0},stallAtPacket{24},originalPackets{0};
std::atomic<unsigned> dropAtOriginal{0};
std::atomic<std::uint64_t> delayedPackets{0},senderDelayUs{0};
DatagramIo::Result sendWithLoss(std::intptr_t socket,const std::uint8_t* data,std::size_t size){
  const auto packet=sanser::snv2::decodePacket({data,size});
  if(packet && packet.header.packetType==sanser::snv2::PacketType::Video &&
     !(packet.header.flags&sanser::snv2::kFlagRetransmitted) && ++originalPackets==stallAtPacket) {
    if(const auto stall=originalStallMs.exchange(0)) {
      const auto began=Clock::now();++delayedPackets;
      std::this_thread::sleep_for(std::chrono::milliseconds(stall));
      senderDelayUs+=std::chrono::duration_cast<std::chrono::microseconds>(Clock::now()-began).count();
    }
  }
  if(packet && packet.header.packetType==sanser::snv2::PacketType::Video &&
     !(packet.header.flags&sanser::snv2::kFlagRetransmitted) &&
     dropAtOriginal.load()!=0 && dropAtOriginal.load()==originalPackets.load() && dropAtOriginal.exchange(0)!=0)
    return {static_cast<std::ptrdiff_t>(size),0};
  if(packet && packet.header.packetType==sanser::snv2::PacketType::Video &&
     !(packet.header.flags&sanser::snv2::kFlagRetransmitted) && loseNextOriginal.exchange(false))
    return {static_cast<std::ptrdiff_t>(size),0};
#ifdef _WIN32
  const auto sent=::send(static_cast<SOCKET>(socket),reinterpret_cast<const char*>(data),static_cast<int>(size),0);
  return {sent,sent<0 ? WSAGetLastError():0};
#else
  const auto sent=::send(static_cast<int>(socket),data,size,0);
  return {sent,sent<0 ? errno:0};
#endif
}
void check(bool value,const char* message){if(!value)throw std::runtime_error(message);}
template<class F> bool waitFor(F condition){
  const auto until=Clock::now()+std::chrono::seconds(5);
  while(!condition() && Clock::now()<until)std::this_thread::sleep_for(std::chrono::milliseconds(2));
  return condition();
}
}
int main(){try{
  std::unique_ptr<Peer> host,client;
  for(unsigned attempt=0;attempt<30 && !client;++attempt){
    Options h;h.host=true;h.audio=false;h.token=std::string(48,'p');
    h.port=static_cast<std::uint16_t>(30000+std::random_device{}()%15000);h.peer="127.0.0.1:"+std::to_string(h.port+1);
    auto c=h;c.host=false;c.bitrate=12000000;c.port=h.port+1;c.peer="127.0.0.1:"+std::to_string(h.port);
    try{host=std::make_unique<Peer>(h,DatagramIo{sendWithLoss,nullptr});client=std::make_unique<Peer>(c);}catch(...){host.reset();}
  }
  check(host && client,"unable to bind paced mobile peers");
  client->receiveVideoLimit(2000000,10);
  std::atomic<unsigned> pictures{0},keys{0};
  client->onFrame=[&](Frame){++pictures;};
  host->onInput=[&](Input event){if(event.kind==Input::Key)++keys;};
  PeerStop stopHost{*host},stopClient{*client};host->start();client->start();
  check(waitFor([&]{return host->ready() && client->controlRoundTripMs().has_value() && host->recommendedBitrate()<=2000000;}),"mobile bitrate negotiation failed");
  // Lossless UDP. The IDR takes ~440 ms at the negotiated wire rate, rather
  // than the ~73 ms predicted by the client's old 12 Mbps reassembly setting.
  // A bounded scheduling interruption, not a per-packet sleep whose cumulative
  // duration is unbounded by CI timer coalescing. Expiry beyond the hard bound
  // is tested separately below and MUST still drop/recover.
  originalStallMs=300;
  host->video({Bytes(128*1024,0x65),1920,1080,true});
  check(!host->acceptsCapture(),"source must stop encoding more references behind a large paced IDR");
  client->input({Input::Key,0,0,4,0,true});client->input({Input::Key,0,0,4,0,false});
  const auto started=waitFor([&]{return pictures>=1 && keys==2;});
  if(!started)std::cerr<<"startup pictures="<<pictures<<" keys="<<keys<<" queueDrops="<<host->videoQueueDrops()<<" repairs="<<host->repairedDatagrams()<<" delayedPackets="<<delayedPackets<<" actualSenderDelayMs="<<senderDelayUs.load()/1000.0<<'\n';
  check(started,"paced IDR or priority input stalled");
  std::cout<<"loaded sender: stalledPackets="<<delayedPackets<<" actualSenderDelayMs="<<senderDelayUs.load()/1000.0<<'\n';
  check(waitFor([&]{return host->acceptsCapture();}),"source must resume after the IDR drains");
  host->video({Bytes(2000,0x41),1920,1080,false});
  check(waitFor([&]{return pictures>=2;}),"video did not continue after paced IDR");
  std::cout<<"lossless paced IDR: repair datagrams="<<host->repairedDatagrams()<<" queue drops="<<host->videoQueueDrops()<<'\n';
  // Kernel/read scheduling may race one tail fragment. It must not duplicate
  // whole blocks of unsent data (the regression queued 96 repairs here).
  const auto beforeLoss=host->repairedDatagrams();
  check(beforeLoss<=4,"lossless mobile pacing generated a premature repair storm");
  check(host->videoQueueDrops()==0,"lossless mobile pacing discarded the reference chain");
  loseNextOriginal=true;
  host->video({Bytes(8000,0x42),1920,1080,false});
  check(waitFor([&]{return pictures>=3;}),"genuinely lost original was not repaired");
  check(host->repairedDatagrams()>beforeLoss,"repair was disabled instead of rejecting premature NACKs");
  // A source briefly exceeds the path's rate. Skip RAW captures before encode,
  // never drop an already encoded P-reference and then demand repeated IDRs.
  unsigned admitted=0,skipped=0;
  const auto until=Clock::now()+std::chrono::seconds(1);
  while(Clock::now()<until) {
    // A burst keeps this an overload test even when the producer's requested
    // 16ms sleep coalesces to 32ms on CI (a single 8KiB frame would then fit).
    for(unsigned raw=0;raw<4;++raw) {
      if(host->acceptsCapture()){host->video({Bytes(8000,0x43),1920,1080,false});++admitted;}
      else ++skipped;
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(16));
  }
  std::cout<<"source admission: admitted="<<admitted<<" supersededRaw="<<skipped<<'\n';
  check(admitted>10 && skipped>0,"fixture must exercise source backpressure and resumed capture");
  const bool drained=waitFor([&]{return pictures>=3+admitted;});
  if(!drained)std::cerr<<"overload pictures="<<pictures<<" expected="<<3+admitted<<" queueDrops="<<host->videoQueueDrops()<<" repaired="<<host->repairedDatagrams()<<" budget="<<host->recommendedBitrate()<<'\n';
  check(drained,"backpressure broke the encoded reference chain");
  check(host->videoQueueDrops()==0,"source overload must not become an IDR/drop loop");
  std::cout<<"overloaded source: admitted="<<admitted<<" supersededRaw="<<skipped<<" encodedQueueDrops="<<host->videoQueueDrops()<<'\n';
  const auto beforeExpiry=pictures.load();
  originalPackets=0;stallAtPacket=1;originalStallMs=1700;
  host->video({Bytes(8000,0x65),1920,1080,true});
  check(waitFor([&]{return host->videoQueueDrops()>0;}),"expired IDR must not accumulate unlimited latency");
  host->video({Bytes(2000,0x65),1920,1080,true});
  check(waitFor([&]{return pictures>beforeExpiry;}),"a fresh IDR must recover after a genuine expiry");
  std::cout<<"Expired IDR discarded; fresh independent picture resumed video\n";
  // Stall one original long enough for early NACKs to exhaust the old
  // whole-frame retry budget, then really lose the last fragment. Forward
  // progress must renew repair, without waiting for another independent IDR.
  const auto beforeTail=pictures.load();const auto dropsBeforeTail=host->videoQueueDrops();
  originalPackets=0;stallAtPacket=24;originalStallMs=600;
  constexpr std::size_t tailBytes=96*1024;
  dropAtOriginal=(tailBytes+8+999)/1000;
  host->video({Bytes(tailBytes,0x65),1920,1080,true});
  check(waitFor([&]{return pictures>beforeTail;}),"paced IDR with a genuinely lost late tail never recovered after early NACKs");
  check(dropAtOriginal==0,"late-tail fixture did not actually lose its final original datagram");
  check(host->videoQueueDrops()==dropsBeforeTail,"late-tail repair must not need another expired IDR");
  std::cout<<"Paced IDR: stalled original plus genuinely lost late tail repaired without restarting the picture\n";
  std::cout<<"Real fragment loss still repairs and resumes dependent pictures\n";
  return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
