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
    const std::string token(48,'r');
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
      client.input({Input::Key,0,0,5,0,true}); // blackhole ACKs
      const auto expired=Clock::now()+std::chrono::milliseconds(300);
      while(client.running() && Clock::now()<expired) std::this_thread::sleep_for(std::chrono::milliseconds(2));
      check(!client.running(),"reliable input must fail closed at deadline, not queue indefinitely");
    }
    std::cout<<"Input: selective ACK, lost key-down recovery, duplicate suppression and deadline passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
