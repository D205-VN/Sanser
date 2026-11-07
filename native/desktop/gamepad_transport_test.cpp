#include "transport.h"
#include <iostream>
#include <random>
#include <stdexcept>
#ifdef _WIN32
#define NOMINMAX
#include <winsock2.h>
using Socket=SOCKET;
#else
#include <sys/socket.h>
using Socket=int;
#endif
using namespace sanser::desktop;
namespace {
const std::string token(48,'g');
PacketCodec asHost(token,true),asViewer(token,false);
std::mutex faultMutex;Bytes delayed;std::intptr_t delayedSocket=-1;
std::atomic<bool> holdNext{false},hideCapability{false},forge{false};
void check(bool value,const char* text){if(!value)throw std::runtime_error(text);}
template<class F>bool waitFor(F test){for(unsigned i=0;i<600;++i){if(test())return true;std::this_thread::sleep_for(std::chrono::milliseconds(5));}return false;}
DatagramIo::Result sendFault(std::intptr_t socket,const std::uint8_t* bytes,std::size_t size){
  Bytes wire(bytes,bytes+size);
  if(const auto packet=asViewer.open(wire);packet && packet->second==Bytes({'S','U','T','1'}))
    return {static_cast<std::ptrdiff_t>(size),0}; // Analog must not depend on Unicode support.
  if(const auto packet=asViewer.open(wire);packet && hideCapability && packet->second.size()==5 &&
      std::equal(packet->second.begin(),packet->second.begin()+4,"SGC1"))return {static_cast<std::ptrdiff_t>(size),0};
  if(const auto packet=asHost.open(wire)){
    auto header=packet->first;
    if(header.packetType==sanser::snv2::PacketType::Gamepad && holdNext.exchange(false)){
      std::lock_guard lock(faultMutex);delayed=wire;delayedSocket=socket;return {static_cast<std::ptrdiff_t>(size),0};
    }
    if(header.packetType==sanser::snv2::PacketType::Keepalive && forge){
      header.packetType=sanser::snv2::PacketType::Gamepad;header.frameId=0;
      GamepadState s;s.connected=true;wire=asViewer.seal(header,encodeGamepad(s));
    }
  }
  const auto sent=::send(static_cast<Socket>(socket),reinterpret_cast<const char*>(wire.data()),static_cast<int>(wire.size()),0);
  return {sent==static_cast<decltype(sent)>(wire.size()) ? static_cast<std::ptrdiff_t>(size):-1,0};
}
void run(bool control,bool sharing,bool old){
  hideCapability=old;holdNext=false;forge=false;
  {std::lock_guard lock(faultMutex);delayed.clear();delayedSocket=-1;}
  std::unique_ptr<Peer> host,client;
  for(unsigned i=0;i<20 && !client;++i){
    const auto port=static_cast<std::uint16_t>(31000+std::random_device{}()%16000);
    Options h;h.host=true;h.input=control;h.sharing=sharing;h.audio=false;h.ultraLowLatency=true;h.token=token;h.port=port;h.peer="127.0.0.1:"+std::to_string(port+1);
    auto c=h;c.host=false;c.input=true;c.sharing=false;c.port=port+1;c.peer="127.0.0.1:"+std::to_string(port);
    try{host=std::make_unique<Peer>(h,DatagramIo{sendFault});client=std::make_unique<Peer>(c,DatagramIo{sendFault});}catch(...){host.reset();}
  }
  check(host && client,"allocate loopback peers");
  std::atomic<bool> available{true};std::atomic<unsigned> frames{0},resets{0};
  std::mutex mutex;std::vector<GamepadState> states;
  host->onGamepadAvailable=[&]{return available.load();};
  host->onInput=[&](Input event){if(event.kind==Input::Reset)++resets;
    if(event.kind==Input::Gamepad){std::lock_guard lock(mutex);states.push_back(event.gamepad);}};
  client->onFrame=[&](Frame){++frames;};
  PeerStop stopHost{*host},stopClient{*client};host->start();client->start();
  check(waitFor([&]{return host->ready() && client->ready();}),"authenticated peers ready");
  GamepadState neutral;neutral.connected=true;auto held=neutral;held.buttons=0x1000;held.lx=-23456;held.ry=12345;held.lt=147;
  const auto count=[&]{std::lock_guard lock(mutex);return states.size();};
  if(!control || sharing || old){
    std::this_thread::sleep_for(std::chrono::milliseconds(180));
    check(!client->gamepadAvailable() && !client->gamepad(neutral,client->remoteGeneration()) && !client->releaseForController(client->remoteGeneration()),"denied/sharing/old host advertised analog input");
    if(!old){forge=true;std::this_thread::sleep_for(std::chrono::milliseconds(200));forge=false;check(count()==0,"forged gamepad bypassed host permissions");}
    check(client->running() && host->running(),"declined gamepad killed media");return;
  }
  check(waitFor([&]{return client->gamepadAvailable();}),"host omitted analog capability");
  const auto epoch=client->remoteGeneration();check(epoch && !client->gamepad(neutral,epoch==UINT32_MAX?1:epoch+1),"stale caller generation accepted");
  check(!client->releaseForController(0) && !client->releaseForController(epoch==UINT32_MAX?1:epoch+1),"stale controller reset accepted");
  check(!client->textAvailable(),"test did not suppress unrelated text capability");
  const auto initialResets=resets.load();
  check(client->releaseForController(epoch) && waitFor([&]{return resets>initialResets;}),"bounded controller reset missing");
  check(client->gamepad(held,epoch),"queue initial held snapshot");std::this_thread::sleep_for(std::chrono::milliseconds(30));check(count()==0,"host armed from held state");
  check(client->gamepad(neutral,epoch) && waitFor([&]{return count()==1;}),"neutral handshake");
  check(client->gamepad(held,epoch) && waitFor([&]{return count()==2;}),"full analog snapshot delivery");
  {std::lock_guard lock(mutex);check(states.back()==held,"analog axes/triggers altered");}
  holdNext=true;check(client->gamepad(neutral,epoch),"queue delayed neutral");
  check(waitFor([&]{std::lock_guard lock(faultMutex);return !delayed.empty();}),"fault did not hold datagram");
  const auto oldResets=resets.load();client->input(Input{});
  check(waitFor([&]{return resets>oldResets;}),"reliable reset not applied");
  {std::lock_guard lock(faultMutex);check(::send(static_cast<Socket>(delayedSocket),reinterpret_cast<const char*>(delayed.data()),static_cast<int>(delayed.size()),0)>0,"release delayed datagram");}
  std::this_thread::sleep_for(std::chrono::milliseconds(25));
  check(client->gamepad(held,epoch),"queue post-reset held state");std::this_thread::sleep_for(std::chrono::milliseconds(25));
  check(count()==2,"pre-reset neutral armed the replacement capture");
  check(client->gamepad(neutral,epoch) && waitFor([&]{return count()==3;}),"rearm neutral");
  check(client->gamepad(held,epoch) && waitFor([&]{return count()==4;}),"rearm held state");
  check(waitFor([&]{return count()==5;}),"lost disconnect failed watchdog release");
  {std::lock_guard lock(mutex);check(states.back()==GamepadState{},"watchdog emitted nonneutral state");}
  host->setCapturePaused(true);check(waitFor([&]{return client->capturePaused();}) && !client->gamepad(neutral,epoch) && !client->releaseForController(epoch),"capture pause accepted analog");
  host->setCapturePaused(false);check(waitFor([&]{return !client->capturePaused();}),"capture resume");
  available=false;check(waitFor([&]{return !client->gamepadAvailable();}),"failed backend capability not withdrawn");
  host->video({Bytes{1,2,3},64,64,true});check(waitFor([&]{return frames>0;}),"backend denial stopped media");
  check(client->running() && host->running(),"analog timeout killed media");
}
}
int main(){try{run(true,false,false);run(false,false,false);run(true,true,false);run(true,false,true);
  std::cout<<"Authenticated analog: capabilities, generation/reset fencing, neutral arming, dropped disconnect, permissions and media survival passed\n";return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
