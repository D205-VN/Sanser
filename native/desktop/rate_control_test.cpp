#include "transport.h"
#include <cmath>
#include <cstring>
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
#ifdef _WIN32
using Socket=SOCKET;
int lastError(){return WSAGetLastError();}
#else
using Socket=int;
int lastError(){return errno;}
#endif
std::atomic<bool> delayed{false};
std::atomic<bool> alternating{false};
std::atomic<unsigned> pongCount{0};
std::unique_ptr<PacketCodec> receiverCodec;
std::mutex samplesMutex;
std::vector<Clock::time_point> sends;
void require(bool value,const char* message){if(!value)throw std::runtime_error(message);}
template<class F> bool waitFor(F f){const auto end=Clock::now()+std::chrono::seconds(5);while(!f() && Clock::now()<end)std::this_thread::sleep_for(std::chrono::milliseconds(5));return f();}
DatagramIo::Result sendObserved(std::intptr_t fd,const std::uint8_t* data,std::size_t size){
  const auto header=sanser::snv2::decodePacket({data,size});
  if(header && header.header.packetType==sanser::snv2::PacketType::Video){std::lock_guard lock(samplesMutex);sends.push_back(Clock::now());}
  const auto n=::send(static_cast<Socket>(fd),reinterpret_cast<const char*>(data),static_cast<int>(size),0);
  return {n,n<0 ? lastError():0};
}
DatagramIo::Result receiveDelayed(std::intptr_t fd,std::uint8_t* data,std::size_t size){
  const auto n=::recv(static_cast<Socket>(fd),reinterpret_cast<char*>(data),static_cast<int>(size),0);
  if(n>0 && delayed){
    const auto decoded=receiverCodec->open(Bytes(data,data+n));
    if(decoded && decoded->first.packetType==sanser::snv2::PacketType::Keepalive &&
       decoded->second.size()==12 && std::memcmp(decoded->second.data(),"SRP1",4)==0) {
      const auto count=++pongCount;
      if(!alternating || count%2==1) std::this_thread::sleep_for(std::chrono::milliseconds(100));
    }
  }
  return {n,n<0 ? lastError():0};
}
}
int main(){try{
  {
    VideoDeliveryWindow delivery;
    // A paced keyframe straddles feedback boundaries. A single 500ms sample
    // looks 25% short although the same original bytes arrive next window.
    auto sample=delivery.observe({200000,150000,160,0,500});
    require(!sample.shortfall(),"a partial feedback window must not cut video quality");
    sample=delivery.observe({100000,150000,160,0,500});
    require(sample.ready() && sample.busy() && !sample.shortfall() && !sample.loss(),"paced boundary skew was mistaken for congestion");
    for(unsigned i=0;i<20;++i) {
      sample=delivery.observe({i%2 ? 100000U:200000U,150000,160,0,500});
      require(!sample.shortfall(),"a lossless paced keyframe repeatedly reduced bitrate");
    }
    // The 1Mbps recovery floor can carry under 32KB in one half-second when
    // capture admission skips raw images. Aggregate activity must still allow
    // recovery rather than trapping the session at the minimum forever.
    delivery.reset();
    for(unsigned i=0;i<4;++i)sample=delivery.observe({24000,24000,24,0,500});
    require(sample.ready() && sample.busy() && !sample.shortfall(),"low-rate healthy video could not recover from bitrate backoff");
    delivery.reset();
    for(unsigned i=0;i<4;++i)sample=delivery.observe({100000,50000,100,12,500});
    require(sample.shortfall() && sample.loss(),"smoothing must retain sustained delivery loss");
    for(unsigned i=0;i<4;++i)sample=delivery.observe({100000,100000,100,0,500});
    require(!sample.shortfall() && !sample.loss(),"old congestion must expire after healthy delivery resumes");
    delivery.reset();sample=delivery.observe({0,0,0,0,500});
    require(!sample.ready() && !sample.busy(),"new sessions must not inherit earlier delivery evidence");
  }
  {
    DelayRatePolicy policy;const auto t=Clock::now();
    require(!policy.observe(200,10,6000000,6000000,t),"one RTT spike must not cut quality");
    require(policy.observe(210,10,6000000,6000000,t+std::chrono::milliseconds(250))==3600000,"persistent latency must reduce bitrate without packet loss");
    require(!policy.observe(220,10,3600000,6000000,t+std::chrono::milliseconds(300)),"backoff has a cooldown");
    require(policy.recovering(t+std::chrono::seconds(1)),"recovery cannot immediately undo latency backoff");
    require(!policy.observe(NAN,10,6000000,6000000,t),"reject nonfinite timing");
    require(!policy.observe(10,200,6000000,6000000,t),"reject impossible RTT baseline");
    for(unsigned i=0;i<20;++i) if(auto rate=policy.observe(300,10,1000000,6000000,t+std::chrono::seconds(i)))require(*rate>=1000000,"rate floor violated");
  }
  {
    // A congested Wi-Fi queue can drain between bursts. Requiring consecutive
    // high samples never reacts to low/high/low/high, despite repeated stalls.
    DelayRatePolicy policy;const auto t=Clock::now();
    require(!policy.observe(14,14,4000000,6000000,t),"healthy baseline must preserve quality");
    require(!policy.observe(88,14,4000000,6000000,t+std::chrono::milliseconds(250)),"one burst must not reduce quality");
    require(!policy.observe(17,14,4000000,6000000,t+std::chrono::milliseconds(500)),"a drained queue is not new congestion");
    require(policy.observe(100,14,4000000,6000000,t+std::chrono::milliseconds(750)).has_value(),"alternating latency spikes must trigger backoff");
    require(policy.recovering(t+std::chrono::seconds(2)),"bursty congestion must hold off immediate rate recovery");
    require(!policy.observe(100,14,3000000,6000000,t+std::chrono::milliseconds(800)),"a single burst cannot cause repeated rate cuts");
  }
  {
    DelayRatePolicy policy;const auto t=Clock::now();
    require(!policy.observe(100,14,4000000,6000000,t),"first isolated spike");
    require(!policy.observe(17,14,4000000,6000000,t+std::chrono::milliseconds(500)),"healthy sample after isolated spike");
    require(!policy.observe(100,14,4000000,6000000,t+std::chrono::seconds(2)),"old spikes must expire, not accumulate across a session");
  }
  {
    // A phone on Wi-Fi power save reports 15-35 ms above a 6 ms baseline on
    // roughly every other 250 ms probe without any queue of ours. That must
    // neither cut the rate nor hold recovery; only a standing rise may.
    DelayRatePolicy policy;const auto t=Clock::now();
    const double jitter[]={24,7,30,8,22,6,35,9,26,7,28,8};
    for(unsigned i=0;i<12;++i) {
      const auto at=t+std::chrono::milliseconds(250*i);
      require(!policy.observe(jitter[i],6,4000000,6000000,at),"moderate alternating Wi-Fi jitter reduced the video rate");
      require(!policy.recovering(at+std::chrono::milliseconds(1)),"moderate isolated jitter blocked rate recovery");
    }
    DelayRatePolicy standing;
    require(!standing.observe(26,6,4000000,6000000,t) && !standing.observe(30,6,4000000,6000000,t+std::chrono::milliseconds(250)),
      "two moderate samples are not yet a standing queue");
    require(standing.observe(28,6,4000000,6000000,t+std::chrono::milliseconds(500))==3400000,"a sustained moderate delay rise must still reduce the rate");
    require(standing.recovering(t+std::chrono::seconds(1)),"a standing rise must hold recovery");
  }
  const auto token=std::string(48,'r');receiverCodec=std::make_unique<PacketCodec>(token,false);
  std::unique_ptr<Peer> host,client;
  for(unsigned i=0;i<30 && !client;++i){
    Options h;h.host=true;h.token=token;h.audio=false;h.port=static_cast<std::uint16_t>(30000+std::random_device{}()%15000);h.peer="127.0.0.1:"+std::to_string(h.port+1);
    auto c=h;c.host=false;c.port=h.port+1;c.peer="127.0.0.1:"+std::to_string(h.port);
    try{host=std::make_unique<Peer>(h,DatagramIo{sendObserved,receiveDelayed});client=std::make_unique<Peer>(c,DatagramIo{sendObserved,receiveDelayed});}catch(...){host.reset();}
  }
  require(host && client,"unable to bind rate control peers");
  client->receiveVideoLimit(6000000,10);
  std::atomic<unsigned> frames{0},keys{0};
  host->onInput=[&](Input input){if(input.kind==Input::Key)++keys;};
  client->onFrame=[&](Frame){++frames;};
  PeerStop stopHost{*host},stopClient{*client};host->start();client->start();
  require(waitFor([&]{return host->ready() && client->controlRoundTripMs().has_value();}),"rate control handshake failed");
  require(host->recommendedBitrate()<=4000000,"authenticated mobile ceiling/warm start was ignored");
  // Exercise real authenticated feedback with alternating fast/100ms pong
  // arrival, as well as deterministic virtual-time policy checks above.
  alternating=true;delayed=true;
  require(waitFor([&]{return host->recommendedBitrate()<=2400000;}),"RTT-only congestion did not lower the encoder budget");
  require(pongCount>=3,"bursty fixture did not include a healthy sample between delayed pongs");
  alternating=false;
  host->video({Bytes(16000,0x71),1280,720,true});
  client->input({Input::Key,0,0,4,0,true});client->input({Input::Key,0,0,4,0,false});
  require(waitFor([&]{return frames>0 && keys==2;}),"latency adaptation froze video or lost priority input");
  {std::lock_guard lock(samplesMutex);require(sends.size()>8 && sends.back()-sends.front()>=std::chrono::milliseconds(20),"wire pacer still used the old 20 Mbps configured rate");}
  delayed=false;
  std::cout<<"Authenticated receiver ceiling, lossless RTT backoff, adaptive wire pacing and priority input passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
