#include "transport.h"
#include <iostream>
#include <memory>
#include <random>
#include <stdexcept>
using namespace sanser::desktop;
void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
template<class F> bool waitFor(F condition) {
  const auto deadline=Clock::now()+std::chrono::seconds(5);
  while(Clock::now()<deadline) { if(condition()) return true; std::this_thread::sleep_for(std::chrono::milliseconds(10)); }
  return condition();
}
int main() {
  try {
    std::unique_ptr<Peer> host,client;
    std::random_device random;
    for(unsigned attempt=0;attempt<10 && !client;++attempt) {
      const auto port=static_cast<uint16_t>(30000+random()%20000);
      Options h; h.host=true; h.port=port; h.peer="127.0.0.1:"+std::to_string(port+1); h.token=std::string(48,'x');
      Options c=h; c.host=false; c.port=port+1; c.peer="127.0.0.1:"+std::to_string(port);
      try { host=std::make_unique<Peer>(h); client=std::make_unique<Peer>(c); } catch(...) { host.reset(); }
    }
    require(host && client,"unable to allocate loopback ports");
    std::mutex mutex; std::vector<Input> inputs; std::optional<Frame> received;
    host->onInput=[&](Input input) { std::lock_guard lock(mutex); inputs.push_back(input); };
    client->onFrame=[&](Frame frame) { std::lock_guard lock(mutex); received=std::move(frame); };
    PeerStop stopHost{*host},stopClient{*client}; host->start(); client->start();
    require(waitFor([&] { return host->ready() && client->ready(); }),"authenticated handshake timed out");
    client->input(Input{Input::Key,0,0,4,0,true}); client->input(Input{Input::Key,0,0,4,0,false});
    require(waitFor([&] { std::lock_guard lock(mutex); unsigned keys=0; for(auto i:inputs) if(i.kind==Input::Key) ++keys; return keys==2; }),"reliable keyboard did not arrive");
    { std::lock_guard lock(mutex); bool down=false,up=false; for(auto i:inputs) if(i.kind==Input::Key) { if(!down) { require(i.down,"key order"); down=true; } else up=!i.down; } require(down && up,"keyboard order mismatch"); }
    Frame sent{Bytes(2500,0x6a),1280,720,true}; host->video(sent);
    require(waitFor([&] { std::lock_guard lock(mutex); return received.has_value(); }),"fragmented video did not arrive");
    { std::lock_guard lock(mutex); require(received->data==sent.data && received->keyframe && received->width==1280,"video mismatch"); }
    client->input(Input{Input::Key,0,0,5,0,true});
    client->stop();
    require(waitFor([&] { return !host->running(); }),"disconnect did not stop the host"); host->stop();
    { std::lock_guard lock(mutex); require(!inputs.empty() && inputs.back().kind==Input::Reset,"disconnect must release held input"); }
    std::cout<<"Loopback: authenticated peers, reliable keyboard, fragmented video, disconnect reset passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
