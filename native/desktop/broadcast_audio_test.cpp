#include "broadcast.h"
#include <array>
#include <cmath>
#include <iostream>
#include <random>
#include <stdexcept>
using namespace sanser::desktop;
void require(bool value,const char* text) { if(!value) throw std::runtime_error(text); }
template<class F> bool waitFor(F check) {
  const auto deadline=Clock::now()+std::chrono::seconds(3);
  while(Clock::now()<deadline) { if(check()) return true; std::this_thread::sleep_for(std::chrono::milliseconds(2)); }
  return check();
}
int main() {
  try {
    Options source; source.host=true; source.input=false; source.audio=true; source.sharing=true;
    Broadcaster group(source);
    std::array<std::unique_ptr<Peer>,3> viewers;
    std::array<std::atomic<unsigned>,3> received{},videos{};
    std::atomic<bool> wrong{false};
    AudioSamples signal{};
    for(unsigned i=0;i<audioFrames;++i) signal[i*2]=signal[i*2+1]=static_cast<std::int16_t>(10000*std::sin(2*3.141592653589793*1000*i/audioRate));
    std::random_device random;
    for(unsigned i=0;i<3;++i) {
      for(unsigned attempt=0;attempt<30 && !viewers[i];++attempt) {
        const auto port=static_cast<std::uint16_t>(25000+random()%25000);
        auto options=source; options.host=false; options.sharing=false;
        options.port=port+1; options.peer="127.0.0.1:"+std::to_string(port); options.token=std::string(48,static_cast<char>('k'+i));
        options.audioOpus=i!=2;
        try {
          auto peer=std::make_unique<Peer>(options);
          peer->onAudio=[&,i](AudioSamples samples) { if(i==2 && samples!=signal) wrong=true; ++received[i]; };
          peer->onFrame=[&,i](Frame) { ++videos[i]; };
          group.add({std::to_string(i),"127.0.0.1:"+std::to_string(port+1),port,options.token,source.bitrate});
          viewers[i]=std::move(peer); viewers[i]->start();
        } catch(...) { group.remove(std::to_string(i)); viewers[i].reset(); }
      }
      require(viewers[i] && waitFor([&] { return viewers[i]->ready(); }),"Group audio handshake");
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(600));
    const auto send=[&](unsigned blocks) {
      for(unsigned i=0;i<blocks;++i) {
        group.audio(signal);
        if(i%10==0) group.publish(Frame{Bytes(1000,0x6a),640,360,true});
        std::this_thread::sleep_for(std::chrono::milliseconds(5));
      }
    };
    const auto before=group.encodedAudioPackets(); send(60);
    require(group.encodedAudioPackets()-before==60,"Opus must encode once per source block, not once per viewer");
    require(waitFor([&] { return received[0]>20 && received[1]>20 && received[2]>20; }),"All viewers receive Opus/PCM with independent credentials");
    require(!wrong,"PCM fallback must keep exact stereo samples");
    viewers[0]->setAudioMuted(true);
    std::this_thread::sleep_for(std::chrono::milliseconds(80));
    const auto muted=received[0].load(),healthy=received[1].load(),picture=videos[0].load();
    send(40);
    require(received[0]==muted,"Muted viewer must not play audio or PLC");
    require(received[1]>healthy+10 && videos[0]>picture,"Mute must preserve other viewers' sound and this viewer's video");
    viewers[0]->setAudioMuted(false); send(40);
    require(waitFor([&] { return received[0]>muted+10; }),"Unmute must clear old playout and resume within the same session");
    viewers[1]->stop(); group.remove("1"); const auto survivor=received[0].load();
    send(30);
    require(waitFor([&] { return received[0]>survivor+10; }),"One viewer closing must not stop group audio");
    group.stop();
    for(auto& viewer:viewers) viewer->stop();
    std::cout<<"Group audio: one Opus encode, mixed codec peers, independent live mute, video continuity and isolated close passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
