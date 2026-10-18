#include "transport.h"
#include <iostream>
#include <memory>
#include <random>
#include <stdexcept>

using namespace sanser::desktop;
void check(bool ok,const char* message) { if(!ok) throw std::runtime_error(message); }
int main() {
  try {
    std::unique_ptr<Peer> host,client; std::random_device random;
    for(unsigned attempt=0;attempt<20 && !client;++attempt) {
      Options h; h.host=true; h.input=false; h.audio=false; h.ultraLowLatency=true;
      h.port=static_cast<std::uint16_t>(30000+random()%20000);
      h.peer="127.0.0.1:"+std::to_string(h.port+1); h.token=std::string(48,'d');
      auto c=h; c.host=false; c.input=true; c.port=h.port+1; c.peer="127.0.0.1:"+std::to_string(h.port);
      try { host=std::make_unique<Peer>(h); client=std::make_unique<Peer>(c); }
      catch(...) { host.reset(); }
    }
    check(host && client,"bind session recovery peers");
    std::atomic<unsigned> injected{0},pictures{0};
    host->onInput=[&](Input input) { if(input.kind!=Input::Reset) ++injected; };
    client->onFrame=[&](Frame) { ++pictures; };
    PeerStop stopHost{*host},stopClient{*client}; host->start(); client->start();
    const auto handshake=Clock::now()+std::chrono::seconds(3);
    while((!host->ready() || !client->ready()) && Clock::now()<handshake) std::this_thread::sleep_for(std::chrono::milliseconds(2));
    check(host->ready() && client->ready(),"session recovery handshake");
    client->input({Input::Key,0,0,4,0,true}); client->input({Input::Key,0,0,4,0,false});
    client->input({Input::Button,100,200,0,0,true}); client->input({Input::Button,100,200,0,0,false});
    client->input({Input::Move,300,400});
    // A stale client preference must not turn the host's input policy into an
    // ACK black hole. No events may be injected; viewing must keep working.
    const auto started=Clock::now();
    const auto until=started+std::chrono::seconds(8);
    unsigned frame=0;
    // Verify continued delivery beyond the input ACK timeout, not a particular
    // FPS on a shared CI runner. Keep a finite deadline if delivery is broken.
    while(Clock::now()<until && host->running() && client->running() &&
          (Clock::now()-started<std::chrono::milliseconds(2400) || pictures<=20)) {
      host->video({Bytes(1000,static_cast<std::uint8_t>(frame)),640,360,true}); ++frame;
      std::this_thread::sleep_for(std::chrono::milliseconds(40));
    }
    check(host->running() && client->running(),"denied remote input must not cause an input ACK timeout or close the viewer");
    check(injected==0,"view-only host must never inject denied input");
    check(pictures>20,"viewing must continue while input is denied");
    host->setCapturePaused(true);
    const auto paused=Clock::now()+std::chrono::seconds(2);
    while(!client->capturePaused() && Clock::now()<paused) std::this_thread::sleep_for(std::chrono::milliseconds(2));
    check(client->capturePaused() && !client->inputAvailable(),"authenticated capture pause must disable viewer input");
    // Simulate a 25-second Secure Desktop/installer pause without touching a
    // real desktop. No video is published; authenticated control stays alive.
    const auto pauseUntil=Clock::now()+std::chrono::seconds(25);
    while(host->running() && client->running() && Clock::now()<pauseUntil)
      std::this_thread::sleep_for(std::chrono::milliseconds(20));
    check(host->running() && client->running() && client->controlRoundTripMs().has_value(),"capture silence must not become a peer timeout");
    host->setCapturePaused(false);
    const auto before=pictures.load();
    host->video({Bytes(1000,0x7f),640,360,true});
    const auto resumed=Clock::now()+std::chrono::seconds(2);
    while((client->capturePaused() || pictures==before) && Clock::now()<resumed) std::this_thread::sleep_for(std::chrono::milliseconds(2));
    check(!client->capturePaused() && pictures>before,"fresh IDR must resume the same session after capture recovery");
    std::cout<<"Session recovery: denied input ACK, 25-second capture pause/control heartbeat and resumed IDR passed\n";
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
