#include "transport.h"
#include "broadcast.h"
#include <array>
#include <iostream>
#include <stdexcept>
#ifdef _WIN32
#define NOMINMAX
#include <winsock2.h>
#include <ws2tcpip.h>
using Socket = SOCKET;
static void closeSocket(Socket s) { closesocket(s); }
#else
#include <arpa/inet.h>
#include <sys/socket.h>
#include <unistd.h>
using Socket = int;
static void closeSocket(Socket s) { close(s); }
#endif
using namespace sanser::desktop;
namespace wire = sanser::snv2;
void check(bool ok, const char* message) { if(!ok) throw std::runtime_error(message); }
struct Stub {
  Socket socket;
  std::uint16_t port;
  Stub() {
    socket=::socket(AF_INET,SOCK_DGRAM,0);
    sockaddr_in address{}; address.sin_family=AF_INET; address.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
    check(::bind(socket,reinterpret_cast<sockaddr*>(&address),sizeof(address))==0,"bind test endpoint");
#ifdef _WIN32
    int length=sizeof(address); DWORD timeout=5;
#else
    socklen_t length=sizeof(address); timeval timeout{0,5000};
#endif
    getsockname(socket,reinterpret_cast<sockaddr*>(&address),&length); port=ntohs(address.sin_port);
    setsockopt(socket,SOL_SOCKET,SO_RCVTIMEO,reinterpret_cast<const char*>(&timeout),sizeof(timeout));
  }
  ~Stub() { closeSocket(socket); }
  void send(std::uint16_t target,const Bytes& packet) {
    sockaddr_in address{}; address.sin_family=AF_INET; address.sin_addr.s_addr=htonl(INADDR_LOOPBACK); address.sin_port=htons(target);
    check(sendto(socket,reinterpret_cast<const char*>(packet.data()),static_cast<int>(packet.size()),0,reinterpret_cast<sockaddr*>(&address),sizeof(address))>=0,"send test packet");
  }
  Bytes receive() {
    std::array<std::uint8_t,1500> bytes{};
    auto size=recv(socket,reinterpret_cast<char*>(bytes.data()),static_cast<int>(bytes.size()),0);
    return size>0 ? Bytes(bytes.begin(),bytes.begin()+size) : Bytes{};
  }
};
std::uint16_t unusedPort() { Stub reservation; return reservation.port; }
std::uint64_t read64(const Bytes& bytes,std::size_t offset) {
  std::uint64_t result=0; for(std::size_t i=offset;i<offset+8;++i) result=(result<<8)|bytes[i]; return result;
}
Bytes ack(std::uint64_t contiguous,std::uint64_t mask) {
  Bytes result;
  for(auto value : {contiguous,mask}) for(int i=7;i>=0;--i) result.push_back(static_cast<std::uint8_t>(value>>(i*8)));
  return result;
}
int main() {
#ifdef _WIN32
  WSADATA winsock{}; if(WSAStartup(MAKEWORD(2,2),&winsock)!=0) return 1;
#endif
  try {
    {
      Stub remote; Options options; options.host=true; options.input=false;
      const std::string token="view-only-host-authenticated-test-token";
      options.token=token; options.port=unusedPort(); options.peer="127.0.0.1:"+std::to_string(remote.port);
      Peer host(options); PacketCodec clientCodec(token,false);
      std::atomic<unsigned> injected{0}, requests{0};
      host.onInput=[&](Input input) { if(input.kind!=Input::Reset) ++injected; };
      host.onKeyframe=[&] { ++requests; };
      PeerStop stop{host}; host.start();
      wire::Header h; h.keyId=123; h.sequence=1; h.packetType=wire::PacketType::Keepalive;
      remote.send(options.port,clientCodec.seal(h,{'S','A','K','1'}));
      h.streamId=2; h.sequence=1; h.packetType=wire::PacketType::Keyboard;
      remote.send(options.port,clientCodec.seal(h,encodeInput({Input::Key,0,0,4,0,true})));
      h.streamId=0; h.sequence=2; h.packetType=wire::PacketType::MouseMove;
      remote.send(options.port,clientCodec.seal(h,encodeInput({Input::Move,100,200})));
      h.sequence=3; h.packetType=wire::PacketType::KeyframeRequest;
      remote.send(options.port,clientCodec.seal(h,{}));
      const auto deadline=Clock::now()+std::chrono::seconds(1);
      while(requests==0 && Clock::now()<deadline) std::this_thread::sleep_for(std::chrono::milliseconds(2));
      check(requests==1 && injected==0,"view-only host must reject authenticated keyboard/mouse while servicing keyframe requests");
    }
    const std::string token(48,'r');
    {
      Stub remote; Options options; options.host=false; options.input=false; options.audio=false; options.token=token;
      options.port=unusedPort(); options.peer="127.0.0.1:"+std::to_string(remote.port);
      Peer client(options); PacketCodec hostCodec(token,true);
      std::mutex mutex; std::vector<Frame> pictures;
      client.onFrame=[&](Frame frame) { std::lock_guard lock(mutex); pictures.push_back(std::move(frame)); };
      PeerStop stop{client}; client.start();
      wire::Header header; header.keyId=123; header.sequence=1; header.packetType=wire::PacketType::Keepalive;
      remote.send(options.port,hostCodec.seal(header,{'S','A','K','1'}));
      Bytes idr{0,0,2,128,0,0,1,104}; idr.resize(2508,0x71);
      Bytes predicted{0,0,2,128,0,0,1,104}; predicted.resize(900,0x72);
      std::uint64_t sequence=0;
      auto sendFragment=[&](const Bytes& frame,unsigned index,std::uint64_t id,bool key,bool repaired=false) {
        const auto count=(frame.size()+999)/1000;
        Bytes body{static_cast<std::uint8_t>(index>>8),static_cast<std::uint8_t>(index),static_cast<std::uint8_t>(count>>8),static_cast<std::uint8_t>(count)};
        for(int i=3;i>=0;--i) body.push_back(static_cast<std::uint8_t>(frame.size()>>(i*8)));
        body.insert(body.end(),frame.begin()+index*1000,frame.begin()+std::min<std::size_t>(frame.size(),(index+1)*1000));
        auto h=header; h.packetType=wire::PacketType::Video; h.streamId=1; h.sequence=++sequence; h.frameId=id;
        h.flags=(key ? wire::kFlagKeyFrame : 0) | (repaired ? wire::kFlagRetransmitted : 0);
        remote.send(options.port,hostCodec.seal(h,body));
      };
      sendFragment(idr,0,1,true); sendFragment(idr,2,1,true);
      sendFragment(predicted,0,2,false);
      std::this_thread::sleep_for(std::chrono::milliseconds(20));
      { std::lock_guard lock(mutex); check(pictures.empty(),"dependent picture must not be decoded before its missing IDR"); }
      sendFragment(idr,1,1,true,true);
      const auto repaired=Clock::now()+std::chrono::seconds(1);
      while(Clock::now()<repaired) {
        { std::lock_guard lock(mutex); if(pictures.size()==2) break; }
        std::this_thread::sleep_for(std::chrono::milliseconds(2));
      }
      { std::lock_guard lock(mutex); check(pictures.size()==2 && pictures[0].id==1 && pictures[0].keyframe && pictures[1].id==2 && !pictures[1].keyframe,"authenticated late repair must deliver IDR then buffered P-frame with their original IDs"); }
      sendFragment(predicted,0,3,false);
      const auto next=Clock::now()+std::chrono::seconds(1);
      while(Clock::now()<next) {
        { std::lock_guard lock(mutex); if(pictures.size()==3) break; }
        std::this_thread::sleep_for(std::chrono::milliseconds(2));
      }
      { std::lock_guard lock(mutex); check(pictures.size()==3 && pictures.back().id==3,"video continues after repair without another keyframe"); }
      sendFragment(predicted,0,5,false);
      std::this_thread::sleep_for(std::chrono::milliseconds(20));
      { std::lock_guard lock(mutex); check(pictures.size()==3,"a gap before an unseen keyframe must not advance delivered video"); }
      sendFragment(idr,0,4,true); sendFragment(idr,2,4,true); sendFragment(idr,1,4,true);
      const auto unseen=Clock::now()+std::chrono::seconds(1);
      while(Clock::now()<unseen) {
        { std::lock_guard lock(mutex); if(pictures.size()==5) break; }
        std::this_thread::sleep_for(std::chrono::milliseconds(2));
      }
      { std::lock_guard lock(mutex); check(pictures.size()==5 && pictures[3].id==4 && pictures[3].keyframe && pictures[4].id==5,"an initially unseen late IDR resumes the authenticated stream and buffered dependent picture"); }
    }
    {
      Stub remote; Options options; options.host=true; options.ultraLowLatency=true; options.token=token;
      options.port=unusedPort(); options.peer="127.0.0.1:"+std::to_string(remote.port);
      Peer host(options); PacketCodec codec(token,false); PeerStop stop{host}; host.start();
      wire::Header h; h.keyId=123; h.sequence=1; h.packetType=wire::PacketType::Keepalive;
      remote.send(options.port,codec.seal(h,{'S','A','K','1'}));
      const auto ready=Clock::now()+std::chrono::seconds(1);
      while(!host.ready() && Clock::now()<ready) std::this_thread::sleep_for(std::chrono::milliseconds(1));
      check(host.ready(),"hiccup test handshake");
      std::this_thread::sleep_for(std::chrono::milliseconds(1250));
      check(host.running(),"a brief Ultra Wi-Fi gap must not destroy an established session");
      ++h.sequence; remote.send(options.port,codec.seal(h,{'S','A','K','1'}));
    }
    {
      Stub remote; Options options; options.host=true; options.input=false; options.token=token;
      options.bitrate=25000000; options.port=unusedPort(); options.peer="127.0.0.1:"+std::to_string(remote.port);
      // Exercise the real capture/encoder adapter, not only Peer's estimator.
      HostOutput host(options); PacketCodec codec(token,false); HostOutputStop stop{host}; host.start();
      wire::Header h; h.keyId=123; h.sequence=1; h.packetType=wire::PacketType::Keepalive;
      remote.send(options.port,codec.seal(h,{'S','A','K','1'}));
      const auto ready=Clock::now()+std::chrono::seconds(1);
      while(!host.ready() && Clock::now()<ready) std::this_thread::sleep_for(std::chrono::milliseconds(1));
      check(host.ready(),"feedback handshake");
      auto feedback=[&](std::uint64_t bytes) {
        std::this_thread::sleep_for(std::chrono::milliseconds(260));
        Bytes payload{'S','V','F','1',0,0,1,244}; // 500 ms window
        for(int i=7;i>=0;--i) payload.push_back(static_cast<std::uint8_t>(bytes>>(i*8)));
        payload.insert(payload.end(),{0,0,0,100,0,0,0,0});
        h.sequence++; h.packetType=wire::PacketType::NetworkFeedback;
        remote.send(options.port,codec.seal(h,payload));
        std::this_thread::sleep_for(std::chrono::milliseconds(5));
      };
      for(unsigned i=0;i<3;++i) feedback(0);
      check(host.bitrate()==options.bitrate,"idle desktop must not reduce bitrate because throughput is low");
      host.video(Frame{Bytes(50000,1),1280,720,true}); feedback(0);
      check(host.bitrate()==options.bitrate,"one transient delivery window must not reduce bitrate");
      host.video(Frame{Bytes(50000,1),1280,720,true}); feedback(0);
      check(host.bitrate()<options.bitrate && host.bitrate()>=5000000,"sustained delivery deficit reaches the single-host encoder budget within quality bounds");
      const auto reduced=host.bitrate();
      for(unsigned i=0;i<10;++i) { host.video(Frame{Bytes(50000,1),1280,720,true}); feedback(50008); }
      check(host.bitrate()>reduced && host.bitrate()<=options.bitrate,"healthy delivery recovers the budget gradually");
      // Explicit progress=0 distinguishes an actual startup black screen from
      // an old peer that simply does not implement the optional progress API.
      (void)host.takeKeyframeRequest();
      Bytes progress{'S','V','P','1',0,0,0,0,0,0,0,0};
      ++h.sequence; h.packetType=wire::PacketType::NetworkFeedback;
      remote.send(options.port,codec.seal(h,progress));
      std::this_thread::sleep_for(std::chrono::milliseconds(5));
      host.video(Frame{Bytes(100000,1),1280,720,true}); feedback(60000);
      host.video(Frame{Bytes(100000,1),1280,720,true}); feedback(60000);
      check(host.bitrate()==5000000,"sustained congestion before the first complete picture uses delivered rate with a quality floor");
      check(host.takeKeyframeRequest(),"startup bitrate fallback also requests a fresh keyframe");
    }
    {
      Stub remote; Options options; options.host=true; options.input=false; options.token=token;
      options.bitrate=12000000; options.port=unusedPort(); options.peer="127.0.0.1:"+std::to_string(remote.port);
      HostOutput host(options); PacketCodec codec(token,false); HostOutputStop stop{host}; host.start();
      wire::Header h; h.keyId=123; h.sequence=1; h.packetType=wire::PacketType::Keepalive;
      remote.send(options.port,codec.seal(h,{'S','A','K','1'}));
      const auto ready=Clock::now()+std::chrono::seconds(1);
      while(!host.ready() && Clock::now()<ready) std::this_thread::sleep_for(std::chrono::milliseconds(1));
      check(host.ready(),"live stall handshake");
      host.video({Bytes(100000,1),1280,720,true});
      h.packetType=wire::PacketType::NetworkFeedback; ++h.sequence;
      const auto progressSent=Clock::now();
      remote.send(options.port,codec.seal(h,{'S','V','P','1',0,0,0,0,0,0,0,1}));
      (void)host.takeKeyframeRequest();
      // One picture was delivered, then progress stalls while source activity
      // and congestion persist. This must recover too, not only black startup.
      for(unsigned window=0;window<3;++window) {
        host.video({Bytes(100000,1),1280,720,true});
        std::this_thread::sleep_for(std::chrono::milliseconds(270));
        Bytes feedback{'S','V','F','1',0,0,1,244};
        for(int i=7;i>=0;--i) feedback.push_back(static_cast<std::uint8_t>(std::uint64_t{60000}>>(i*8)));
        feedback.insert(feedback.end(),{0,0,0,100,0,0,0,0}); ++h.sequence;
        remote.send(options.port,codec.seal(h,feedback));
        std::this_thread::sleep_for(std::chrono::milliseconds(5));
        const auto budget=host.bitrate();
        // Two 270 ms sleeps can exceed the actual 750 ms stall threshold on
        // a busy runner. Judge a brief gap by elapsed time, not loop count.
        if(Clock::now()-progressSent<std::chrono::milliseconds(750))
          check(budget>5000000,"brief delivery variation must not trigger live-stall emergency recovery");
      }
      const auto applied=Clock::now()+std::chrono::seconds(1);
      while(host.bitrate()!=5000000 && Clock::now()<applied) std::this_thread::sleep_for(std::chrono::milliseconds(2));
      check(host.bitrate()==5000000 && host.takeKeyframeRequest(),"a frozen live picture with sustained activity recovers budget and requests a fresh IDR");
    }
    {
      Stub remote; Options options; options.host=true; options.input=false; options.token=token;
      options.bitrate=100000000; options.port=unusedPort(); options.peer="127.0.0.1:"+std::to_string(remote.port);
      Peer host(options); PacketCodec clientCodec(token,false),foreignCodec(std::string(48,'f'),false);
      std::atomic<unsigned> keyframes{0}; host.onKeyframe=[&] { ++keyframes; };
      PeerStop stop{host}; host.start();
      wire::Header request; request.keyId=123; request.sequence=1; request.packetType=wire::PacketType::Keepalive;
      remote.send(options.port,clientCodec.seal(request,{'S','A','K','1'}));
      const auto readyDeadline=Clock::now()+std::chrono::seconds(1);
      while(!host.ready() && Clock::now()<readyDeadline) std::this_thread::sleep_for(std::chrono::milliseconds(1));
      check(host.ready(),"repair host handshake");
      host.video(Frame{Bytes(2500,0x6a),1280,720,true});
      std::uint64_t frame=0,lastSequence=0; unsigned fragments=0;
      const auto deadline=Clock::now()+std::chrono::seconds(1);
      Bytes missing;
      while(fragments<3 && Clock::now()<deadline) {
        const auto packet=clientCodec.open(remote.receive()); if(!packet || packet->first.packetType!=wire::PacketType::Video) continue;
        frame=packet->first.frameId; lastSequence=std::max(lastSequence,packet->first.sequence); ++fragments;
        if(packet->second[0]==0 && packet->second[1]==1) missing=packet->second;
      }
      check(fragments==3 && !missing.empty(),"repair source has three fragments");
      Bytes nack{'S','V','N','1'}; for(int i=7;i>=0;--i) nack.push_back(static_cast<std::uint8_t>(frame>>(i*8)));
      nack.insert(nack.end(),{0,1,0,1}); // one missing fragment, index one
      request.sequence=2; request.packetType=wire::PacketType::Nack;
      remote.send(options.port,foreignCodec.seal(request,nack));
      std::this_thread::sleep_for(std::chrono::milliseconds(5));
      check(host.repairedDatagrams()==0,"foreign viewer credential cannot access repair cache");
      const auto datagram=clientCodec.seal(request,nack); remote.send(options.port,datagram);
      bool repaired=false;
      while(Clock::now()<deadline && !repaired) {
        const auto packet=clientCodec.open(remote.receive()); if(!packet || packet->first.packetType!=wire::PacketType::Video) continue;
        if(packet->first.flags & wire::kFlagRetransmitted) {
          check(packet->first.sequence>lastSequence && packet->first.frameId==frame && packet->second==missing,"repair uses fresh authentication sequence with original frame and fragment");
          repaired=true;
        }
      }
      check(repaired && host.repairedDatagrams()==1,"authenticated NACK recovers missing fragment");
      remote.send(options.port,datagram); std::this_thread::sleep_for(std::chrono::milliseconds(15));
      check(host.repairedDatagrams()==1,"replayed NACK cannot amplify repair traffic");
      std::this_thread::sleep_for(std::chrono::milliseconds(220));
      request.sequence=3; remote.send(options.port,clientCodec.seal(request,nack));
      const auto delayed=Clock::now()+std::chrono::milliseconds(250);
      while(host.repairedDatagrams()<2 && Clock::now()<delayed) std::this_thread::sleep_for(std::chrono::milliseconds(1));
      check(host.repairedDatagrams()==2 && host.repairCacheMisses()==0,"a WAN-delayed NACK must still find its fragment in cache");
      // Keep repair available across a slow WAN RTT; cache residency itself
      // adds no presentation delay. Only requests beyond its TTL must miss.
      std::this_thread::sleep_for(std::chrono::milliseconds(1100));
      request.sequence=4; remote.send(options.port,clientCodec.seal(request,nack));
      const auto expired=Clock::now()+std::chrono::seconds(1);
      while(host.repairCacheMisses()==0 && Clock::now()<expired) std::this_thread::sleep_for(std::chrono::milliseconds(1));
      check(host.repairCacheMisses()==1 && keyframes>0,"expired repair cache requests a fresh keyframe");
    }
    {
      Stub remote; Options options; options.host=true; options.ultraLowLatency=true; options.token=token;
      options.port=unusedPort(); options.peer="127.0.0.1:"+std::to_string(remote.port);
      Peer host(options); PacketCodec clientCodec(token,false);
      std::mutex mutex; std::vector<Input> events;
      host.onInput=[&](Input event) { std::lock_guard lock(mutex); if(event.kind==Input::Key) events.push_back(event); };
      PeerStop stop{host}; host.start();
      wire::Header h; h.keyId=123; h.sequence=1; h.packetType=wire::PacketType::Keepalive;
      remote.send(options.port,clientCodec.seal(h,{'S','A','K','1'}));
      h.streamId=2; h.sequence=2; h.packetType=wire::PacketType::Keyboard; h.flags=wire::kFlagAcknowledgementRequired;
      auto second=clientCodec.seal(h,encodeInput({Input::Key,0,0,4,0,false}));
      remote.send(options.port,second);
      bool sacked=false; const auto deadline=Clock::now()+std::chrono::seconds(1);
      while(Clock::now()<deadline && !sacked) {
        auto packet=clientCodec.open(remote.receive());
        if(packet && packet->first.packetType==wire::PacketType::NetworkFeedback && packet->second.size()==16)
          sacked=read64(packet->second,0)==0 && read64(packet->second,8)==2;
      }
      check(sacked,"out-of-order key-up must be selectively acknowledged");
      { std::lock_guard lock(mutex); check(events.empty(),"key-up must wait for missing key-down"); }
      remote.send(options.port,second); // duplicate cannot inject input twice
      h.sequence=1; remote.send(options.port,clientCodec.seal(h,encodeInput({Input::Key,0,0,4,0,true})));
      bool delivered=false;
      while(Clock::now()<deadline && !delivered) {
        std::this_thread::sleep_for(std::chrono::milliseconds(2));
        std::lock_guard lock(mutex); delivered=events.size()==2;
        if(delivered) check(events[0].down && !events[1].down,"recovered key order");
      }
      check(delivered,"missing reliable input must recover");
    }
    {
      Stub remote; Options options; options.ultraLowLatency=true; options.token=token;
      options.port=unusedPort(); options.peer="127.0.0.1:"+std::to_string(remote.port);
      Peer client(options); PacketCodec hostCodec(token,true); PeerStop stop{client}; client.start();
      wire::Header h; h.keyId=456; h.sequence=1; h.packetType=wire::PacketType::Keepalive;
      remote.send(options.port,hostCodec.seal(h,{'S','A','K','1'}));
      const auto readyDeadline=Clock::now()+std::chrono::seconds(1);
      while(!client.ready() && Clock::now()<readyDeadline) std::this_thread::sleep_for(std::chrono::milliseconds(1));
      check(client.ready(),"input test handshake");
      client.input({Input::Key,0,0,4,0,true}); client.input({Input::Key,0,0,4,0,false});
      unsigned firstCount=0,secondCount=0; bool cumulativeSent=false;
      const auto deadline=Clock::now()+std::chrono::milliseconds(300);
      while(Clock::now()<deadline && !cumulativeSent) {
        auto packet=hostCodec.open(remote.receive()); if(!packet || packet->first.streamId!=2) continue;
        if(packet->first.sequence==1) ++firstCount;
        if(packet->first.sequence==2) {
          ++secondCount; h.sequence++; h.packetType=wire::PacketType::NetworkFeedback;
          remote.send(options.port,hostCodec.seal(h,ack(0,2)));
        }
        if(firstCount>=2 && secondCount>=1) {
          h.sequence++; h.packetType=wire::PacketType::NetworkFeedback;
          remote.send(options.port,hostCodec.seal(h,ack(2,0))); cumulativeSent=true;
        }
      }
      check(cumulativeSent && secondCount==1,"SACK must retry the missing event, not the received event");
      std::this_thread::sleep_for(std::chrono::milliseconds(20));
      check(client.running(),"ACKed input must not terminate session");
      client.input({Input::Button,100,200,0,0,true});
      client.input({Input::Button,100,200,0,0,false});
      const auto delayedAck=Clock::now()+std::chrono::milliseconds(350);
      while(Clock::now()<delayedAck) {
        h.sequence++; h.packetType=wire::PacketType::Keepalive;
        remote.send(options.port,hostCodec.seal(h,{'S','A','K','1'}));
        std::this_thread::sleep_for(std::chrono::milliseconds(30));
      }
      check(client.running(),"a mouse click must not close Ultra before a 350 ms WAN ACK can arrive");
      h.sequence++; h.packetType=wire::PacketType::NetworkFeedback;
      remote.send(options.port,hostCodec.seal(h,ack(4,0)));
      std::this_thread::sleep_for(std::chrono::milliseconds(20));
      check(client.running(),"delayed mouse down/up ACK keeps the session open");
      client.input({Input::Key,0,0,5,0,true}); // blackhole ACKs
      const auto expired=Clock::now()+std::chrono::seconds(5);
      while(client.running() && Clock::now()<expired) {
        h.sequence++; h.packetType=wire::PacketType::Keepalive;
        remote.send(options.port,hostCodec.seal(h,{'S','A','K','1'}));
        std::this_thread::sleep_for(std::chrono::milliseconds(30));
      }
      check(!client.running(),"reliable input must fail closed at deadline, not queue indefinitely");
    }
    std::cout<<"Input: selective ACK, lost key-down recovery, duplicate suppression and deadline passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
