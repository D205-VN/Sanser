#include "transport.h"
#include <array>
#include <cstdlib>
#include <iostream>
#include <limits>
#include <stdexcept>
#ifdef _WIN32
#define NOMINMAX
#include <winsock2.h>
#include <ws2tcpip.h>
using Socket=SOCKET;
static void closeSocket(Socket socket) { closesocket(socket); }
#else
#include <arpa/inet.h>
#include <sys/socket.h>
#include <unistd.h>
using Socket=int;
static void closeSocket(Socket socket) { close(socket); }
#endif
using namespace sanser::desktop;
namespace wire=sanser::snv2;
void check(bool ok,const char* message) { if(!ok) throw std::runtime_error(message); }
struct Release {
  std::mutex& mutex; std::condition_variable& wake; bool& released;
  void now() { { std::lock_guard lock(mutex); released=true; } wake.notify_all(); }
  ~Release() { now(); }
};
struct Endpoint {
  Socket socket; std::uint16_t port;
  Endpoint() {
    socket=::socket(AF_INET,SOCK_DGRAM,0);
    sockaddr_in address{}; address.sin_family=AF_INET; address.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
    check(::bind(socket,reinterpret_cast<sockaddr*>(&address),sizeof(address))==0,"bind input test endpoint");
#ifdef _WIN32
    int length=sizeof(address); DWORD timeout=5;
#else
    socklen_t length=sizeof(address); timeval timeout{0,5000};
#endif
    getsockname(socket,reinterpret_cast<sockaddr*>(&address),&length); port=ntohs(address.sin_port);
    setsockopt(socket,SOL_SOCKET,SO_RCVTIMEO,reinterpret_cast<const char*>(&timeout),sizeof(timeout));
  }
  ~Endpoint() { closeSocket(socket); }
  void send(std::uint16_t destination,const Bytes& bytes) {
    sockaddr_in address{}; address.sin_family=AF_INET; address.sin_addr.s_addr=htonl(INADDR_LOOPBACK); address.sin_port=htons(destination);
    check(sendto(socket,reinterpret_cast<const char*>(bytes.data()),static_cast<int>(bytes.size()),0,reinterpret_cast<sockaddr*>(&address),sizeof(address))==static_cast<int>(bytes.size()),"send input test datagram");
  }
  Bytes receive() {
    std::array<char,1500> bytes{}; const auto count=recv(socket,bytes.data(),static_cast<int>(bytes.size()),0);
    return count>0 ? Bytes(bytes.begin(),bytes.begin()+count) : Bytes{};
  }
};
void testQueue() {
  std::mutex mutex; std::condition_variable wake; bool entered=false,released=false;
  std::vector<Input> events; unsigned failures=0;
  InputWorker worker([&](Input input) {
    std::unique_lock lock(mutex); events.push_back(input);
    if(input.kind==Input::Key && input.down && input.code==4) {
      entered=true; wake.notify_all(); wake.wait(lock,[&] { return released; });
    }
    wake.notify_all();
  },[&] { ++failures; });
  Release cleanup{mutex,wake,released};
  check(worker.submit({Input::Key,0,0,4,0,true}),"queue held key");
  { std::unique_lock lock(mutex); check(wake.wait_for(lock,std::chrono::seconds(2),[&] { return entered; }),"input worker did not enter callback"); }
  for(unsigned i=0;i<10000;++i) check(worker.submit({Input::Move,static_cast<std::uint16_t>(i),0}),"coalesce pointer burst");
  check(worker.submit({Input::Button,9999,0,0,0,true}),"queue click boundary");
  check(worker.submit({Input::Move,12000,0}),"queue drag after button");
  check(worker.submit({Input::Key,0,0,4,0,false}),"queue key-up");
  cleanup.now();
  { std::unique_lock lock(mutex); check(wake.wait_for(lock,std::chrono::seconds(2),[&] { return events.size()>=5; }),"input sequence did not drain"); }
  worker.stop();
  check(events.size()==6 && events[1].kind==Input::Move && events[1].x==9999 &&
    events[2].kind==Input::Button && events[3].kind==Input::Move && events[3].x==12000 &&
    events[4].kind==Input::Key && !events[4].down && events.back().kind==Input::Reset,
    "coalescing crossed an action boundary, lost key-up or failed shutdown reset");
  check(worker.takeStats().coalescedMoves==9999 && failures==0,"pointer burst must stay bounded without callback failure");
}
void testResetAndCapacity() {
  std::mutex mutex; std::condition_variable wake; bool entered=false,released=false;
  std::vector<Input> events;
  InputWorker worker([&](Input input) {
    std::unique_lock lock(mutex); events.push_back(input);
    if(input.kind==Input::Key && input.code==4) { entered=true; wake.notify_all(); wake.wait(lock,[&] { return released; }); }
    wake.notify_all();
  },[] {});
  Release cleanup{mutex,wake,released};
  check(worker.submit({Input::Key,0,0,4,0,true}),"queue blocking test key");
  { std::unique_lock lock(mutex); check(wake.wait_for(lock,std::chrono::seconds(2),[&] { return entered; }),"blocking test did not start"); }
  for(unsigned i=0;i<256;++i) check(worker.submit({Input::Key,0,0,5,0,true}),"bounded queue fills without losing reliable events");
  check(!worker.submit({Input::Key,0,0,5,0,false}),"overflow must be explicit, never silently lose key-up");
  check(worker.submit(Input{}),"focus reset must be accepted even when input queue is full");
  cleanup.now();
  { std::unique_lock lock(mutex); check(wake.wait_for(lock,std::chrono::seconds(2),[&] { return events.size()>=2; }),"reset did not drain"); }
  worker.stop();
  check(events.size()==3 && events[1].kind==Input::Reset && events[2].kind==Input::Reset,"reset/stop must discard stale presses and release held state");
  unsigned failures=0,resets=0;
  InputWorker throwing([&](Input input) { if(input.kind==Input::Reset) ++resets; else throw std::runtime_error("synthetic injection failure"); },[&] { ++failures; });
  check(throwing.submit({Input::Key,0,0,4,0,true}),"queue synthetic failure");
  // Wait on a barrier rather than stopping before the callback has a chance.
  const auto deadline=Clock::now()+std::chrono::seconds(2);
  while(throwing.submit({Input::Move,1,1}) && Clock::now()<deadline) std::this_thread::sleep_for(std::chrono::milliseconds(1));
  throwing.stop();
  check(failures==1 && resets==1,"injection exceptions must fail closed and still reset held input");
}
void testRetryAndAckQueue() {
  using namespace std::chrono;
  check(sanser::inputRetryDelay(10)==milliseconds(17),"shared LAN input retry");
  check(sanser::inputRetryDelay(200)>=milliseconds(200),"WAN input must get a round trip before retry");
  check(sanser::inputRetryDelay(10,20)>sanser::inputRetryDelay(10),"RTT variation needs repair headroom");
  check(sanser::inputRetryDelay(10,0,2)==milliseconds(34),"input retry backs off after a lost ACK");
  check(sanser::inputRetryDelay(5000,0,100)==milliseconds(1000),"retry backoff remains bounded");
  check(sanser::inputRetryDelay(std::numeric_limits<double>::quiet_NaN())==milliseconds(40),"invalid RTT cannot become zero retry");
  InputDeliveryPolicy policy; policy.acknowledged(milliseconds(10)); const auto stable=policy.retry(true,1);
  policy.acknowledged(milliseconds(60)); check(policy.retry(true,1)>stable,"input RTT estimator tracks variation");
  TxScheduler tx; const auto now=Clock::now();
  for(unsigned i=1;i<=3;++i) { TxScheduler::Packet packet{{static_cast<std::uint8_t>(i)},now,0}; packet.inputSequence=i; packet.input=true; check(tx.push(TxScheduler::InputLane,std::move(packet)),"queue reliable input retry"); }
  tx.acknowledgeInput(1,2); // cumulative one, selectively received three
  const auto missing=tx.pop(false);
  check(missing && missing->inputSequence==2 && !tx.pop(false),"ACK/SACK must remove obsolete queued sends without losing the gap");
}
void testSlowInjectionDoesNotBlockNetwork() {
  const std::string token(48,'i'); Endpoint remote,reserved;
  Options options; options.host=true; options.ultraLowLatency=true; options.audio=false; options.token=token;
  // Release the reservation before Peer binds; no production capture or input.
  options.port=reserved.port; closeSocket(reserved.socket); reserved.socket=static_cast<Socket>(-1);
  options.peer="127.0.0.1:"+std::to_string(remote.port);
  Peer host(options); PacketCodec codec(token,false);
  std::mutex mutex; std::condition_variable wake; bool entered=false,released=false;
  std::vector<Input> events;
  host.onInput=[&](Input input) {
    std::unique_lock lock(mutex); events.push_back(input);
    if(input.kind==Input::Key && input.down) { entered=true; wake.notify_all(); wake.wait(lock,[&] { return released; }); }
    wake.notify_all();
  };
  PeerStop stop{host}; Release cleanup{mutex,wake,released}; host.start();
  wire::Header header; header.keyId=123; header.sequence=1; header.packetType=wire::PacketType::Keepalive;
  remote.send(options.port,codec.seal(header,{'S','A','K','1'}));
  header.streamId=2; header.packetType=wire::PacketType::Keyboard; header.flags=wire::kFlagAcknowledgementRequired;
  remote.send(options.port,codec.seal(header,encodeInput({Input::Key,0,0,4,0,true})));
  { std::unique_lock lock(mutex); check(wake.wait_for(lock,std::chrono::seconds(2),[&] { return entered; }),"host did not begin slow synthetic injection"); }
  ++header.sequence; remote.send(options.port,codec.seal(header,encodeInput({Input::Key,0,0,4,0,false})));
  header.streamId=0; header.sequence=2; header.packetType=wire::PacketType::Keepalive; header.flags=0;
  Bytes probe{'S','R','Q','1',0,0,0,0,0,0,0,7}; remote.send(options.port,codec.seal(header,probe));
  host.video({Bytes(500,1),640,360,true});
  bool pong=false,ack=false,video=false; const auto deadline=Clock::now()+std::chrono::seconds(2);
  while(Clock::now()<deadline && !(pong && ack && video)) {
    const auto packet=codec.open(remote.receive()); if(!packet) continue;
    const auto& [h,b]=*packet;
    if(h.packetType==wire::PacketType::Keepalive && b.size()==12 && b[2]=='P' && b.back()==7) pong=true;
    if(h.packetType==wire::PacketType::NetworkFeedback && b.size()==16 &&
       std::all_of(b.begin(),b.begin()+7,[](auto value) { return value==0; }) && b[7]==2) ack=true;
    if(h.packetType==wire::PacketType::Video) video=true;
  }
  check(pong && ack && video,"slow OS input must not block UDP pong, reliable receipt ACK or video");
  { std::lock_guard lock(mutex); check(!released && events.back().down,"ACK reports ordered queue acceptance, not OS execution"); }
  cleanup.now();
  { std::unique_lock lock(mutex); check(wake.wait_for(lock,std::chrono::seconds(2),[&] { return events.back().kind==Input::Key && !events.back().down; }),"key-up lost after slow injection resumed"); }
  bool timing=false; const auto timingDeadline=Clock::now()+std::chrono::seconds(2);
  while(!timing && Clock::now()<timingDeadline) {
    const auto packet=codec.open(remote.receive());
    if(packet && packet->first.packetType==wire::PacketType::NetworkFeedback && packet->second.size()==16)
      timing=std::equal(packet->second.begin(),packet->second.begin()+4,Bytes{'S','I','T','1'}.begin());
  }
  check(timing,"authenticated host execution timing must reach the viewing peer separately from ACK");
  host.stop();
  check(events.back().kind==Input::Reset,"disconnect must release held host state on its input worker");
}
int main() {
#ifdef _WIN32
  WSADATA winsock{}; if(WSAStartup(MAKEWORD(2,2),&winsock)!=0) return 1;
  _putenv_s("SANSER_TELEMETRY","1");
#else
  setenv("SANSER_TELEMETRY","1",1);
#endif
  try {
    testQueue(); testResetAndCapacity(); testRetryAndAckQueue(); testSlowInjectionDoesNotBlockNetwork();
    std::cout<<"Input engine: bounded/coalesced actions, reset barriers, retry/SACK and independent UDP control passed\n";
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
#ifdef _WIN32
  WSACleanup();
#endif
}
