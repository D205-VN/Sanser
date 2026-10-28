#include "udp_io_policy.h"
#include "transport.h"
#include <iostream>
#include <random>
#include <stdexcept>
#ifndef _WIN32
#include <sys/socket.h>
#include <unistd.h>
#endif
using namespace sanser::desktop;
namespace {
#ifdef _WIN32
using Socket=SOCKET;
constexpr int full=WSAENOBUFS,unreachable=WSAENETUNREACH,refused=WSAECONNRESET;
constexpr int invalid=WSAENOTSOCK,oversize=WSAEMSGSIZE,timeout=WSAETIMEDOUT;
int lastError() { return WSAGetLastError(); }
#else
using Socket=int;
constexpr int full=ENOBUFS,unreachable=ENETUNREACH,refused=ECONNREFUSED;
constexpr int invalid=EBADF,oversize=EMSGSIZE,timeout=EAGAIN;
int lastError() { return errno; }
#endif
std::atomic<unsigned> failKeys{0},failVideo{0},failReceive{0};
std::atomic<int> fatalSend{0},fatalReceive{0};
std::atomic<bool> shortSend{false};
void check(bool ok,const char* message) { if(!ok) throw std::runtime_error(message); }
bool take(std::atomic<unsigned>& count) {
  auto value=count.load();
  while(value && !count.compare_exchange_weak(value,value-1)) {}
  return value!=0;
}
DatagramIo::Result sendWithFaults(std::intptr_t fd,const std::uint8_t* data,std::size_t size) {
  if(const auto error=fatalSend.exchange(0)) return {-1,error};
  if(shortSend.exchange(false)) return {static_cast<std::ptrdiff_t>(size)-1,0};
  const auto header=sanser::snv2::decodePacket({data,size});
  if(header) {
    if(header.header.packetType==sanser::snv2::PacketType::Keyboard && take(failKeys)) return {-1,full};
    if(header.header.packetType==sanser::snv2::PacketType::Video && take(failVideo)) return {-1,unreachable};
  }
  const auto sent=::send(static_cast<Socket>(fd),reinterpret_cast<const char*>(data),static_cast<int>(size),0);
  return {sent,sent<0 ? lastError() : 0};
}
DatagramIo::Result receiveWithFaults(std::intptr_t fd,std::uint8_t* data,std::size_t size) {
  if(const auto error=fatalReceive.exchange(0)) return {-1,error};
  if(take(failReceive)) return {-1,refused};
  const auto received=::recv(static_cast<Socket>(fd),reinterpret_cast<char*>(data),static_cast<int>(size),0);
  return {received,received<0 ? lastError() : 0};
}
template<class Predicate> bool waitFor(Predicate predicate) {
  const auto end=Clock::now()+std::chrono::seconds(3);
  while(!predicate() && Clock::now()<end) std::this_thread::sleep_for(std::chrono::milliseconds(2));
  return predicate();
}
void exercise(bool ultra,unsigned terminalFault) {
  std::unique_ptr<Peer> host,client;
  std::random_device random;
  DatagramIo io{sendWithFaults,receiveWithFaults};
  for(unsigned attempt=0;attempt<30 && !client;++attempt) {
    Options h; h.host=true; h.audio=false; h.ultraLowLatency=ultra; h.token=std::string(48,'u');
    h.port=static_cast<std::uint16_t>(30000+random()%20000); h.peer="127.0.0.1:"+std::to_string(h.port+1);
    auto c=h; c.host=false; c.port=h.port+1; c.peer="127.0.0.1:"+std::to_string(h.port);
    try { host=std::make_unique<Peer>(h,io); client=std::make_unique<Peer>(c,io); }
    catch(...) { host.reset(); }
  }
  check(host && client,"bind fault-injection peers");
  std::atomic<unsigned> keys{0},pictures{0},stops{0};
  host->onInput=[&](Input input) { if(input.kind==Input::Key) ++keys; };
  client->onFrame=[&](Frame frame) { check(frame.data==Bytes(6000,0x73),"repaired media contents changed"); ++pictures; };
  host->onStopped=client->onStopped=[&] { ++stops; };
  PeerStop stopHost{*host},stopClient{*client}; host->start(); client->start();
  check(waitFor([&] { return host->ready() && client->ready(); }),"authenticated fault-injection handshake");
  failKeys=2; failVideo=1; failReceive=2;
  client->input({Input::Key,0,0,4,0,true}); client->input({Input::Key,0,0,4,0,false});
  host->video({Bytes(6000,0x73),640,360,true});
  check(waitFor([&] { return pictures==1 && keys==2; }),"temporary send/receive errors froze video or lost reliable key transitions");
  check(host->running() && client->running() && stops==0,"recoverable datagram errors disconnected the session");
  const auto h=host->socketStats(),c=client->socketStats();
  check(h.sendTransientErrors==1 && c.sendTransientErrors==2 && h.receiveTransientErrors+c.receiveTransientErrors==2,
    "transient UDP diagnostics must count actual faults, not ordinary receive timeouts");
  check(host->repairedDatagrams()>0,"the missing video datagram was not repaired from the existing cache");
  // Later traffic still works. No network generation, window or auth reset.
  host->video({Bytes(6000,0x73),640,360,false});
  check(waitFor([&] { return pictures==2; }),"the reference chain did not continue after recovery");
  if(terminalFault==0) fatalReceive=invalid;
  else if(terminalFault==1) fatalSend=invalid;
  else shortSend=true;
  check(waitFor([&] { return !host->running() && !client->running(); }),"fatal socket error did not stop both peers cleanly");
  check(host->stopReason()==5 || client->stopReason()==5,"invalid socket or partial UDP send must remain a terminal transport error");
}
}
int main() {
#ifdef _WIN32
  WSADATA startup{}; if(WSAStartup(MAKEWORD(2,2),&startup)!=0) return 1;
  struct Cleanup { ~Cleanup() { WSACleanup(); } } cleanup;
#endif
  try {
    check(sanser::udp::transientSendError(full) && sanser::udp::transientSendError(unreachable),"temporary UDP send faults must be recoverable");
    check(!sanser::udp::transientSendError(invalid) && !sanser::udp::transientSendError(oversize),"invalid sockets/oversized outbound packets must not be hidden");
    check(sanser::udp::receiveFailure(timeout)==sanser::udp::ReceiveFailure::Idle,"ordinary UDP timeout is not an outage");
    check(sanser::udp::receiveFailure(oversize)==sanser::udp::ReceiveFailure::Retry,"truncated inbound UDP must be discarded without terminating");
    exercise(false,0); exercise(true,1); exercise(false,2);
    std::cout<<"Socket recovery: Auto/Ultra authenticated video NACK, reliable input retry, receive faults and fatal shutdown passed\n";
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
