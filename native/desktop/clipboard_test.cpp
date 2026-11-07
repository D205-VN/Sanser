#include "transport.h"
#include <atomic>
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
const std::string token(48,'c');
std::atomic<bool> lose{false},duplicate{false},forgeCapability{false};
void check(bool value,const char* message){if(!value)throw std::runtime_error(message);}
template<class F>bool waitFor(F condition){const auto until=Clock::now()+std::chrono::seconds(5);while(Clock::now()<until){if(condition())return true;std::this_thread::sleep_for(std::chrono::milliseconds(5));}return condition();}
DatagramIo::Result faultSend(std::intptr_t socket,const uint8_t* bytes,std::size_t size){
  Bytes wire(bytes,bytes+size);PacketCodec client(token,false),host(token,true);bool copy=false;
  auto packet=host.open(wire);if(!packet)packet=client.open(wire);
  if(packet && packet->first.packetType==sanser::snv2::PacketType::Clipboard && packet->second.size()>22 && packet->second[4]==1){
    if(lose.exchange(false))return {static_cast<std::ptrdiff_t>(size),0};copy=duplicate.exchange(false);
  }
  if(forgeCapability)if(const auto outgoing=client.open(wire);outgoing && outgoing->first.packetType==sanser::snv2::PacketType::Keepalive && outgoing->second==Bytes({'S','I','C','1'}))
    wire=host.seal(outgoing->first,Bytes{'S','C','B','1'});
  const auto sent=::send(static_cast<Socket>(socket),reinterpret_cast<const char*>(wire.data()),static_cast<int>(wire.size()),0);
  if(copy && sent==static_cast<decltype(sent)>(wire.size()))(void)::send(static_cast<Socket>(socket),reinterpret_cast<const char*>(wire.data()),static_cast<int>(wire.size()),0);
  return {sent==static_cast<decltype(sent)>(wire.size()) ? static_cast<std::ptrdiff_t>(size):-1,0};
}
void session(bool permission,bool control,bool sharing){
  forgeCapability=!permission || !control || sharing;
  std::unique_ptr<Peer> host,client;
  for(unsigned attempt=0;attempt<20 && !client;++attempt){
    const auto port=static_cast<uint16_t>(30000+std::random_device{}()%18000);
    Options h;h.host=true;h.input=control;h.sharing=sharing;h.clipboard=permission;h.audio=false;h.ultraLowLatency=true;h.token=token;h.port=port;h.peer="127.0.0.1:"+std::to_string(port+1);
    Options c=h;c.host=false;c.sharing=false;c.input=true;c.port=port+1;c.peer="127.0.0.1:"+std::to_string(port);
    try{host=std::make_unique<Peer>(h,DatagramIo{faultSend});client=std::make_unique<Peer>(c,DatagramIo{faultSend});}catch(...){host.reset();}
  }
  check(host && client,"allocate clipboard test ports");
  std::mutex mutex;std::string content="Tiếng Việt 😀";content.append(8192-content.size(),'x');
  std::atomic<unsigned> operations{0},frames{0},keys{0};
  host->onInput=[&](Input event){if(event.kind==Input::Key)++keys;};
  host->onClipboard=[&](bool write,std::string text,std::function<bool()> current,Peer::ClipboardResult done){
    check(current(),"provider called after cancellation");std::lock_guard lock(mutex);++operations;if(write)content=std::move(text);done(0,write ? std::string{}:content);
  };
  client->onFrame=[&](Frame){++frames;};
  client->onClipboard=[](bool,std::string,std::function<bool()> current,Peer::ClipboardResult done){done(current()?0:5,"local-only ế");};
  PeerStop a{*host},b{*client};host->start();client->start();
  check(waitFor([&]{return host->ready() && client->ready() && client->clipboardAvailable();}),"clipboard capability did not arrive");
  std::atomic<bool> done{false};unsigned status=99;std::string received;
  lose=true;duplicate=true;
  check(client->clipboard(false,"",[&](unsigned code,std::string text){status=code;received=std::move(text);done=true;}),"queue clipboard read");
  for(unsigned i=0;i<6;++i)host->video(Frame{Bytes(3000,7),1280,720,true});
  if(control && !sharing){client->input({Input::Key,0,0,4,0,true});client->input({Input::Key,0,0,4,0,false});}
  check(waitFor([&]{return done.load();}),"clipboard read did not recover after loss/duplicate");
  check(waitFor([&]{return frames>0;}),"clipboard prevented video progress");
  if(permission && control && !sharing){
    check(status==0 && received==content && operations==1,"clipboard read duplicated or corrupted");
    check(waitFor([&]{return keys==2;}),"clipboard blocked reliable keys");
    std::this_thread::sleep_for(std::chrono::milliseconds(120));done=false;
    check(client->clipboard(true,"clipboard update ế\r\n",[&](unsigned code,std::string text){status=code;received=std::move(text);done=true;}),"queue clipboard write");
    check(waitFor([&]{return done.load();}),"clipboard write timeout");
    {std::lock_guard lock(mutex);check(status==0 && received.empty() && content=="clipboard update ế\r\n" && operations==2,"clipboard write was not applied once");}
    check(!client->clipboard(true,std::string(8193,'a'),[](unsigned,std::string){}),"oversized clipboard queued");
  }else check(status==1 && operations==0,"forged clipboard capability bypassed host permission or view-only");
  check(host->running() && client->running(),"clipboard failure stopped the media session");
  const auto before=operations.load();done=false;
  client->localClipboard(false,"",[&](unsigned code,std::string text){status=code;received=std::move(text);done=true;});
  check(done && status==0 && received=="local-only ế" && operations==before,"local clipboard accidentally accessed the remote host");
}
}
int main(){try{session(true,true,false);session(false,true,false);session(true,false,false);session(true,true,true);std::cout<<"Clipboard: authenticated loss recovery, exact-once OS operation, host permission/read-only enforcement and independent media passed\n";return 0;}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
