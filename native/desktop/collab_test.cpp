#include "transport.h"
#include "collab.h"
#include <atomic>
#include <iostream>
#include <mutex>
#include <random>
#include <stdexcept>
using namespace sanser::desktop;
namespace {
const std::string token(48,'c');
void check(bool value,const char* message){if(!value)throw std::runtime_error(message);}
template<class F> bool waitFor(F condition){const auto until=Clock::now()+std::chrono::seconds(5);while(Clock::now()<until){if(condition())return true;std::this_thread::sleep_for(std::chrono::milliseconds(5));}return condition();}
void commands(){
  const auto parsed=parseCollabCommand("collab-send\t*\t"+userHex("{\"type\":\"chat\"}"));
  check(parsed && parsed->target=="*" && std::string(parsed->payload.begin(),parsed->payload.end())=="{\"type\":\"chat\"}","parse fan-out command");
  check(parseCollabCommand("collab-send\t018f4d89-5e8b-7a80-bd1e-cb7cb9f43189\t61").has_value(),"viewer target rejected");
  check(!parseCollabCommand("collab-send\t-\t").has_value(),"empty payload accepted");
  check(!parseCollabCommand("collab-send\tbad target\t61").has_value(),"unsafe target accepted");
  check(!parseCollabCommand("collab-send\t-\tzz").has_value(),"invalid hex accepted");
  check(!parseCollabCommand("collab-send\t-\t"+std::string((collabMaxPayload+1)*2,'a')).has_value(),"oversized payload accepted");
  check(!parseCollabCommand("user-text\t-\t61").has_value(),"foreign command parsed as collab");
  check(collabPayload(collabBody({1,2,3}))==std::vector<std::uint8_t>({1,2,3}),"envelope round trip");
  check(!collabPayload({'S','C','B','1',1}).has_value(),"clipboard body treated as collab");
  CollabRate rate;const auto now=CollabRate::Clock::now();unsigned taken=0;
  for(unsigned i=0;i<100;++i)if(rate.take(now))++taken;
  check(taken==30,"flood guard burst");check(rate.take(now+std::chrono::milliseconds(100)),"flood guard refill");
}
void session(bool hostCollab,bool viewOnly){
  std::unique_ptr<Peer> host,client;
  for(unsigned attempt=0;attempt<20 && !client;++attempt){
    const auto port=static_cast<uint16_t>(30000+std::random_device{}()%18000);
    Options h;h.host=true;h.collab=hostCollab;h.input=!viewOnly;h.audio=false;h.ultraLowLatency=true;h.token=token;h.port=port;h.peer="127.0.0.1:"+std::to_string(port+1);
    Options c=h;c.host=false;c.collab=true;c.port=port+1;c.peer="127.0.0.1:"+std::to_string(port);
    try{host=std::make_unique<Peer>(h);client=std::make_unique<Peer>(c);}catch(...){host.reset();}
  }
  check(host && client,"allocate collab test ports");
  std::mutex mutex;std::vector<Bytes> atHost,atClient;std::atomic<unsigned> frames{0};
  host->onCollab=[&](Bytes bytes){std::lock_guard lock(mutex);atHost.push_back(std::move(bytes));};
  client->onCollab=[&](Bytes bytes){std::lock_guard lock(mutex);atClient.push_back(std::move(bytes));};
  client->onFrame=[&](Frame){++frames;};
  PeerStop a{*host},b{*client};host->start();client->start();
  check(waitFor([&]{return host->ready() && client->ready();}),"session did not start");
  if(!hostCollab){
    std::this_thread::sleep_for(std::chrono::milliseconds(400));
    check(!client->collabAvailable(),"collab advertised against an older host");
    check(!client->collab({'h','i'}),"collab queued to an older host");
    return;
  }
  check(waitFor([&]{return client->collabAvailable() && host->collabAvailable();}),"collab capability was not negotiated");
  const Bytes chat{'{','"','t','"','}'};
  Bytes stroke(collabMaxPayload,'7');
  check(client->collab(chat),"viewer chat rejected");check(client->collab(stroke),"viewer stroke rejected");
  check(host->collab(chat),"host chat rejected");
  check(!client->collab(Bytes(collabMaxPayload+1,'x')),"oversized collab queued");
  check(!client->collab({}),"empty collab queued");
  for(unsigned i=0;i<4;++i)host->video(Frame{Bytes(3000,7),1280,720,true});
  check(waitFor([&]{std::lock_guard lock(mutex);return atHost.size()==2 && atClient.size()==1;}),"collab payloads were not delivered");
  {std::lock_guard lock(mutex);check(atHost[0]==chat && atHost[1]==stroke && atClient[0]==chat,"collab payload corrupted or reordered");}
  check(waitFor([&]{return frames>0;}),"collab blocked video");
  check(host->running() && client->running(),"collab stopped media");
}
}
int main(){try{commands();session(true,false);session(true,true);session(false,true);std::cout<<"Collab: bounded IPC, negotiated capability, both directions in control and view-only sessions, older-peer fallback passed\n";return 0;}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
