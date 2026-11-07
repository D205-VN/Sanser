#include "transport.h"
#include "user_ipc.h"
#include <atomic>
#include <random>
#include <iostream>
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
const std::string token(48,'f');
std::atomic<bool> lose{false},duplicate{false},forge{false};
void check(bool value,const char* message){if(!value)throw std::runtime_error(message);}
template<class F> bool waitFor(F condition){const auto until=Clock::now()+std::chrono::seconds(5);while(Clock::now()<until){if(condition())return true;std::this_thread::sleep_for(std::chrono::milliseconds(5));}return condition();}
DatagramIo::Result faultSend(std::intptr_t socket,const uint8_t* data,std::size_t size){
  Bytes wire(data,data+size);PacketCodec client(token,false),host(token,true);bool copy=false;
  auto packet=host.open(wire);if(!packet)packet=client.open(wire);
  if(packet && packet->first.packetType==sanser::snv2::PacketType::Clipboard && packet->second.size()>22 && packet->second[4]==1){
    if(lose.exchange(false))return {static_cast<std::ptrdiff_t>(size),0};copy=duplicate.exchange(false);
  }
  if(forge)if(const auto outgoing=client.open(wire);outgoing && outgoing->first.packetType==sanser::snv2::PacketType::Keepalive && outgoing->second==Bytes({'S','I','C','1'}))wire=host.seal(outgoing->first,Bytes{'S','F','T','1'});
  const auto sent=::send(static_cast<Socket>(socket),reinterpret_cast<const char*>(wire.data()),static_cast<int>(wire.size()),0);
  if(copy && sent==static_cast<decltype(sent)>(wire.size()))(void)::send(static_cast<Socket>(socket),reinterpret_cast<const char*>(wire.data()),static_cast<int>(wire.size()),0);
  return {sent==static_cast<decltype(sent)>(wire.size()) ? static_cast<std::ptrdiff_t>(size):-1,0};
}
void session(bool permission,bool input,bool sharing){
  forge=!permission || !input || sharing;
  std::unique_ptr<Peer> host,client;
  for(unsigned attempt=0;attempt<20 && !client;++attempt){
    const auto port=static_cast<uint16_t>(30000+std::random_device{}()%18000);
    Options h;h.host=true;h.files=permission;h.input=input;h.sharing=sharing;h.audio=false;h.ultraLowLatency=true;h.token=token;h.port=port;h.peer="127.0.0.1:"+std::to_string(port+1);
    Options c=h;c.host=false;c.input=true;c.sharing=false;c.files=true;c.port=port+1;c.peer="127.0.0.1:"+std::to_string(port);
    try{host=std::make_unique<Peer>(h,DatagramIo{faultSend});client=std::make_unique<Peer>(c,DatagramIo{faultSend});}catch(...){host.reset();}
  }
  check(host && client,"allocate file test ports");
  Bytes body{'S','F','T','1',3};body.resize(4125,42);
  std::atomic<unsigned> hostFiles{0},clientFiles{0},frames{0},keys{0};
  host->onFile=[&](Bytes bytes){check(bytes==body,"file bytes corrupted");++hostFiles;};
  client->onFile=[&](Bytes bytes){check(bytes==body,"reverse file bytes corrupted");++clientFiles;};
  host->onInput=[&](Input value){if(value.kind==Input::Key)++keys;};client->onFrame=[&](Frame){++frames;};
  PeerStop a{*host},b{*client};host->start();client->start();
  check(waitFor([&]{return host->ready() && client->ready() && client->filesAvailable();}),"negotiated file capability missing");
  const auto parsed=parseUserCommand("file-send\t018f4d89-5e8b-7a80-bd1e-cb7cb9f43189\t"+userHex(std::string(body.begin(),body.end())));
  check(parsed && parsed->kind==UserCommand::FileMessage && Bytes(parsed->body.begin(),parsed->body.end())==body,"binary private IPC corruption");
  const auto scoped=std::string("\0\0\0\7",4)+std::string(body.begin(),body.end());
  check(userFileBody(scoped,7)==body && !userFileBody(scoped,8),"stale private file command crossed media generation");
  lose=true;duplicate=true;
  check(client->fileMessage(body),"file message rejected");
  for(unsigned i=0;i<6;++i)host->video(Frame{Bytes(3000,7),1280,720,true});
  if(input && !sharing){client->input({Input::Key,0,0,4,0,true});client->input({Input::Key,0,0,4,0,false});}
  check(waitFor([&]{return frames>0;}),"file transport blocked video");
  if(permission && input && !sharing){
    check(waitFor([&]{return hostFiles==1 && keys==2;}),"file loss repair or keyboard progress failed");
    check(host->fileMessage(body),"host could not send reverse file");check(waitFor([&]{return clientFiles==1;}),"reverse file did not arrive");
    std::this_thread::sleep_for(std::chrono::milliseconds(150));check(hostFiles==1 && clientFiles==1,"duplicate file delivered");
  }else{std::this_thread::sleep_for(std::chrono::milliseconds(300));check(hostFiles==0,"forged file capability bypassed host permission/view-only");}
  check(host->running() && client->running(),"file error stopped media");
}
}
int main(){try{session(true,true,false);session(false,true,false);session(true,false,false);session(true,true,true);std::cout<<"Files: binary IPC, bidirectional authenticated delivery under loss/duplicates, permission and media/input isolation passed\n";return 0;}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
