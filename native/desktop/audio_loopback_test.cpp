#include "transport.h"
#include <iostream>
#include <memory>
#include <random>
#include <stdexcept>
using namespace sanser::desktop;
void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
template<class F> bool waitFor(F condition) {
  const auto deadline=Clock::now()+std::chrono::seconds(3);
  while(Clock::now()<deadline) { if(condition()) return true; std::this_thread::sleep_for(std::chrono::milliseconds(2)); }
  return condition();
}
void checkPair(bool hostAudio,bool clientAudio,bool receiverInstalled) {
  std::unique_ptr<Peer> host,client;
  std::random_device random;
  for(unsigned attempt=0;attempt<20 && !client;++attempt) {
    const auto port=static_cast<std::uint16_t>(30000+random()%20000);
    Options h; h.host=true; h.audio=hostAudio; h.port=port;
    h.peer="127.0.0.1:"+std::to_string(port+1); h.token=std::string(48,'q');
    Options c=h; c.host=false; c.audio=clientAudio; c.port=port+1; c.peer="127.0.0.1:"+std::to_string(port);
    try { host=std::make_unique<Peer>(h); client=std::make_unique<Peer>(c); } catch(...) { host.reset(); client.reset(); }
  }
  require(client!=nullptr,"Audio test could not bind sockets");
  AudioSamples expected{}; expected[0]=1234; expected[1]=-1234;
  std::atomic<unsigned> received{0}; std::atomic<bool> correct{true};
  if(receiverInstalled) client->onAudio=[&](AudioSamples samples) { if(samples!=expected) correct=false; ++received; };
  PeerStop stopHost{*host},stopClient{*client};
  host->start(); client->start();
  require(waitFor([&] { return host->ready() && client->ready(); }),"Audio handshake timeout");
  if(hostAudio && clientAudio && receiverInstalled) {
    require(waitFor([&] { return host->audioReady(); }),"Audio capability not negotiated");
    host->audio(expected);
    require(waitFor([&] { return received==1; }),"Authenticated audio not delivered");
    require(correct,"Stereo PCM differs after UDP transport");
    client->audio(expected); // A viewer may never send audio back to the host.
  } else {
    std::this_thread::sleep_for(std::chrono::milliseconds(550));
    require(!host->audioReady(),"Audio enabled for muted/legacy viewer");
    host->audio(expected);
    std::this_thread::sleep_for(std::chrono::milliseconds(30));
    require(received==0,"Muted or unnegotiated audio leaked");
  }
  require(host->running() && client->running(),"Audio stopped input/video session");
}
int main() {
  try {
    checkPair(true,true,true);
    checkPair(false,true,true);
    checkPair(true,false,true);
    checkPair(true,true,false); // Existing SNV2 viewers have no audio callback.
    std::cout<<"Audio loopback: authenticated delivery, host mute, viewer mute and legacy compatibility passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
