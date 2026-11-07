#include "transport.h"
#include "text_input.h"
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
void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
template<class F> bool waitFor(F condition) {
  const auto deadline=Clock::now()+std::chrono::seconds(5);
  while(Clock::now()<deadline) { if(condition()) return true; std::this_thread::sleep_for(std::chrono::milliseconds(5)); }
  return condition();
}
std::atomic<bool> dropText{false}, duplicateText{false}, hideCapability{false}, forgeText{false}, dropAck{false};
std::atomic<unsigned> textAttempts{0};
std::atomic<bool> dropReset{false},duplicateReset{false};
const std::string token(48,'t');
DatagramIo::Result sendFault(std::intptr_t socket,const std::uint8_t* bytes,std::size_t size) {
  PacketCodec hostCodec(token,true),clientCodec(token,false);
  Bytes wire(bytes,bytes+size);
  bool duplicate=false;
  if(const auto packet=hostCodec.open(wire);packet && packet->first.packetType==sanser::snv2::PacketType::Keyboard) {
    if(!packet->second.empty() && packet->second[0]==Input::Reset) {
      if(dropReset.exchange(false))return {static_cast<std::ptrdiff_t>(size),0};
      duplicate=duplicateReset.exchange(false);
    }
    if(!packet->second.empty() && packet->second[0]==Input::Text) {
      ++textAttempts;
      if(dropText.exchange(false)) return {static_cast<std::ptrdiff_t>(size),0};
      duplicate=duplicateText.exchange(false);
    }
    if(forgeText) {
      Input edit; edit.kind=Input::Text; edit.text="forged";
      wire=clientCodec.seal(packet->first,encodeInput(edit));
    }
  }
  if(hideCapability) if(const auto packet=clientCodec.open(wire);
    packet && packet->first.packetType==sanser::snv2::PacketType::Keepalive && packet->second==Bytes({'S','U','T','1'}))
      return {static_cast<std::ptrdiff_t>(size),0};
  if(dropAck) if(const auto packet=clientCodec.open(wire);
    packet && packet->first.packetType==sanser::snv2::PacketType::NetworkFeedback &&
    (packet->second.size()==8 || packet->second.size()==16)) return {static_cast<std::ptrdiff_t>(size),0};
  const auto sent=::send(static_cast<Socket>(socket),reinterpret_cast<const char*>(wire.data()),static_cast<int>(wire.size()),0);
  if(duplicate && sent==static_cast<decltype(sent)>(wire.size()))
    (void)::send(static_cast<Socket>(socket),reinterpret_cast<const char*>(wire.data()),static_cast<int>(wire.size()),0);
  return {sent==static_cast<decltype(sent)>(wire.size()) ? static_cast<std::ptrdiff_t>(size):-1,0};
}
void roundTrip(bool control,bool sharing,bool oldHost,bool pressure=false) {
  hideCapability=oldHost; forgeText=!control || sharing;
  std::unique_ptr<Peer> host,client;
  for(unsigned attempt=0;attempt<20 && !client;++attempt) {
    const auto port=static_cast<std::uint16_t>(30000+std::random_device{}()%18000);
    Options h; h.host=true; h.input=control; h.sharing=sharing; h.ultraLowLatency=true;
    h.port=port; h.peer="127.0.0.1:"+std::to_string(port+1); h.token=token; h.audio=false;
    Options c=h; c.host=false; c.input=true; c.sharing=false; c.port=port+1; c.peer="127.0.0.1:"+std::to_string(port);
    try { host=std::make_unique<Peer>(h,DatagramIo{sendFault}); client=std::make_unique<Peer>(c,DatagramIo{sendFault}); } catch(...) { host.reset(); }
  }
  require(host && client,"allocate text test ports");
  std::mutex mutex; std::vector<Input> received;
  host->onInput=[&](Input input) { if(input.kind!=Input::Reset) { std::lock_guard lock(mutex); received.push_back(std::move(input)); } };
  if(pressure)host->onGamepadAvailable=[]{return true;};
  PeerStop stopHost{*host},stopClient{*client}; host->start(); client->start();
  require(waitFor([&] { return host->ready() && client->ready(); }),"authenticate text peers");
  if(control && !sharing && !oldHost) {
    require(waitFor([&] { return client->textAvailable(); }),"host did not announce Unicode input");
    const auto epoch=client->remoteGeneration();
    require(epoch!=0,"text client missing authenticated generation");
    require(!client->shortcut(std::vector<std::uint16_t>{227,26},epoch==UINT32_MAX ? 1:epoch+1),"stale shortcut epoch accepted");
    require(!client->text("stale draft",epoch==UINT32_MAX ? 1:epoch+1),"old-generation IME draft reserved input sequence");
    host->setCapturePaused(true);
    require(waitFor([&] { return client->capturePaused(); }),"host capture pause did not arrive");
    require(client->textAvailable() && !client->text("paused"),"capture pause lost the capability or queued text");
    require(!client->shortcut(std::vector<std::uint16_t>{227,26},epoch),"capture pause accepted shortcut");
    host->setCapturePaused(false);
    require(waitFor([&] { return !client->capturePaused(); }),"capture resume did not arrive");
    if(pressure) {
      require(waitFor([&]{return client->gamepadAvailable();}),"test host omitted controller capability");
      dropAck=true;
      for(unsigned i=0;i<64;++i) require(client->text("queued"),"text window filled before its bound");
      require(!client->text("overflow") && client->running(),"text backpressure stopped the media session");
      require(!client->shortcut(std::vector<std::uint16_t>{227,26},epoch) && client->running(),"full input window partially accepted shortcut or killed media");
      require(!client->releaseForShortcuts(epoch) && client->running(),"shortcut opener reset killed a full input window");
      require(!client->releaseForController(epoch) && client->running(),"controller opener reset killed a full input window");
      dropAck=false;
      require(waitFor([&] { return client->text("tail"); }),"input window did not recover after ACK delivery resumed");
      require(waitFor([&] { std::lock_guard lock(mutex); return received.size()==65; }),"bounded text window lost or duplicated edits");
      return;
    }
    dropText=true; duplicateText=true; textAttempts=0;
    const std::string text="Tiếng Việt e\xcc\x81 😀 中文";
    require(client->text(text,epoch),"queue generation-fenced Unicode edit");
    client->input({Input::Key,0,0,4,0,true}); client->input({Input::Key,0,0,4,0,false});
    require(waitFor([&] { std::lock_guard lock(mutex); return received.size()==3; }),"lost text did not recover before later keys");
    require(textAttempts>=2,"fault injection did not exercise text retransmission");
    std::this_thread::sleep_for(std::chrono::milliseconds(250));
    { std::lock_guard lock(mutex); require(received.size()==3 && received[0].kind==Input::Text && received[0].text==text &&
      received[1].kind==Input::Key && received[1].down && !received[2].down,"Unicode was duplicated, corrupted or reordered"); }
    require(!client->text(std::string(257,'a')) && !client->text(std::string("a\0b",3)),"invalid edit was partially queued");
    {std::lock_guard lock(mutex);received.clear();}
    dropReset=true;duplicateReset=true;
    require(client->releaseForShortcuts(epoch),"bounded panel-entry reset refused");
    require(client->shortcut(std::vector<std::uint16_t>{227,225,43},epoch),"bounded native shortcut refused");
    require(client->shortcut(std::vector<std::uint16_t>{227,25},epoch),"second native shortcut refused");
    client->input({Input::Key,0,0,4,0,true});client->input({Input::Key,0,0,4,0,false});
    require(waitFor([&]{std::lock_guard lock(mutex);return received.size()==12;}),"lost shortcut reset blocked reliable key sequence");
    std::this_thread::sleep_for(std::chrono::milliseconds(150));
    {std::lock_guard lock(mutex);require(received.size()==12,"shortcut retransmission duplicated an edge");
      const std::uint16_t codes[]={227,225,43,43,225,227,227,25,25,227,4,4};
      for(unsigned i=0;i<12;++i)require(received[i].code==codes[i] && received[i].down==(i<3 || i==6 || i==7 || i==10),"shortcut down/up order changed");}
  } else {
    std::this_thread::sleep_for(std::chrono::milliseconds(250));
    require(!client->textAvailable() && !client->text("ế"),"old or denied host accepted Unicode");
    require(!client->shortcut(std::vector<std::uint16_t>{227,26},client->remoteGeneration()),"old or denied host accepted native action");
    client->input({Input::Key,0,0,4,0,true}); client->input({Input::Key,0,0,4,0,false});
    if(oldHost) require(waitFor([&] { std::lock_guard lock(mutex); return received.size()==2; }),"unsupported Unicode reserved an input sequence and blocked ASCII");
    else { std::this_thread::sleep_for(std::chrono::milliseconds(100)); std::lock_guard lock(mutex); require(received.empty(),"view-only host injected forged text"); }
  }
  require(client->running() && host->running(),"text must not stop the media session");
}
}
int main() { try {
  const auto valid=sanser::textInputUtf16("ếe\xcc\x81😀");
  require(valid && *valid==u"ếé😀","Unicode scalar to UTF-16 conversion");
  for(const std::string invalid:{std::string{},std::string("\xc0\xaf"),std::string("\xed\xa0\x80"),std::string("\xf4\x90\x80\x80"),std::string("\xe2\x82"),std::string("a\0b",3),std::string("a\x1b"),std::string(257,'a')})
    require(!sanser::textInputUtf16(invalid),"malformed/oversized text accepted");
  Input edit; edit.kind=Input::Text; edit.text="Tiếng Việt 😀\n\t";
  const auto decoded=decodeInput(encodeInput(edit));
  require(decoded && decoded->text==edit.text,"text wire round-trip");
  roundTrip(true,false,false); roundTrip(true,false,true); roundTrip(false,false,false); roundTrip(true,true,false); roundTrip(true,false,false,true);
  std::cout<<"Unicode: strict validation, authenticated negotiation, loss/reorder recovery, no partial old-host send and host-enforced view-only passed\n";
  return 0;
} catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; } }
