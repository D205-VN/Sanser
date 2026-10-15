#include "transport.h"
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
      Stub remote; Options options; options.host=true; options.input=false; options.token=token;
      options.bitrate=25000000; options.port=unusedPort(); options.peer="127.0.0.1:"+std::to_string(remote.port);
      Peer host(options); PacketCodec codec(token,false); PeerStop stop{host}; host.start();
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
      check(host.recommendedBitrate()==options.bitrate,"idle desktop must not reduce bitrate because throughput is low");
      host.video(Frame{Bytes(50000,1),1280,720,true}); feedback(0);
      check(host.recommendedBitrate()==options.bitrate,"one transient delivery window must not reduce bitrate");
      host.video(Frame{Bytes(50000,1),1280,720,true}); feedback(0);
      check(host.recommendedBitrate()<options.bitrate && host.recommendedBitrate()>=5000000,"sustained delivery deficit reduces the viewer budget within quality bounds");
      const auto reduced=host.recommendedBitrate();
      for(unsigned i=0;i<10;++i) { host.video(Frame{Bytes(50000,1),1280,720,true}); feedback(50008); }
      check(host.recommendedBitrate()>reduced && host.recommendedBitrate()<=options.bitrate,"healthy delivery recovers the budget gradually");
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
      std::this_thread::sleep_for(std::chrono::milliseconds(160));
      request.sequence=3; remote.send(options.port,clientCodec.seal(request,nack));
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
