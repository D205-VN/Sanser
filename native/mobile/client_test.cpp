#include "client.h"
#include "transport.h"
#include <atomic>
#include <iostream>
#include <memory>
#include <random>
#include <stdexcept>
using namespace sanser::desktop;
namespace {
void require(bool ok,const char* text){if(!ok)throw std::runtime_error(text);}
template<class T> bool waitFor(T condition){auto until=Clock::now()+std::chrono::seconds(5);while(Clock::now()<until){if(condition())return true;std::this_thread::sleep_for(std::chrono::milliseconds(5));}return condition();}
struct Results {std::atomic<unsigned> frames{0},keys{0},stopped{0};};
void video(void *context,const uint8_t *bytes,size_t size,uint32_t width,uint32_t height,int key){if(size==5000 && bytes[0]==42 && width==1280 && height==720 && key)static_cast<Results*>(context)->frames++;}
void stopped(void *context,uint32_t){static_cast<Results*>(context)->stopped++;}
struct Delete {void operator()(SanserMobileClient *c)const{sanser_mobile_destroy(c);}};
}
int main(){try{
  Results results;
  require(!sanser_mobile_create("127.0.0.1:1",0,"short",1,0,"",{&results,video,nullptr,nullptr}),"short credentials accepted");
  for(bool control:{false,true}){
    std::unique_ptr<Peer> host;std::unique_ptr<SanserMobileClient,Delete> client;
    for(unsigned attempt=0;attempt<20 && !client;++attempt){
      auto port=static_cast<uint16_t>(30000+std::random_device{}()%18000);
      Options options;options.host=true;options.port=port;options.peer="127.0.0.1:"+std::to_string(port+1);options.token=std::string(48,'m');
      try {host=std::make_unique<Peer>(options);client.reset(sanser_mobile_create(("127.0.0.1:"+std::to_string(port)).c_str(),port+1,options.token.c_str(),control,0,"",{&results,video,nullptr,stopped}));}catch(...){host.reset();}
    }
    require(host && client,"unable to allocate test ports");
    host->onInput=[&](Input i){if(i.kind==Input::Key)results.keys++;};
    sanser_mobile_video_limit(client.get(),2000000,8.0);
    PeerStop stop{*host};host->start();require(sanser_mobile_start(client.get()),"client start failed");
    require(waitFor([&]{return host->ready() && sanser_mobile_ready(client.get());}),"authenticated handshake failed");
    require(waitFor([&]{return host->recommendedBitrate()<=2000000;}),"mobile receive ceiling did not reach the desktop encoder budget");
    sanser_mobile_input(client.get(),2,0,0,4,0,1);sanser_mobile_input(client.get(),2,0,0,4,0,0);
    if(control)require(waitFor([&]{return results.keys==2;}),"keyboard did not reach desktop");
    else {std::this_thread::sleep_for(std::chrono::milliseconds(100));require(results.keys==0,"view-only injected keyboard");}
    const auto before=results.frames.load();host->video(Frame{Bytes(5000,42),1280,720,true});
    require(waitFor([&]{return results.frames>before;}),"fragmented video did not reach C callback");
    require(waitFor([&]{return sanser_mobile_rtt(client.get())>=0;}),"unmeasured RTT");
    require(waitFor([&]{SanserMobileStats stats{};sanser_mobile_stats(client.get(),&stats);return stats.video_frames>0 && stats.video_packets>0;}),"mobile numeric receive snapshot did not update without stdout tracing");
    SanserMobileStats empty{1,1,1,1,1,1,1,1};sanser_mobile_stats(nullptr,&empty);
    require(empty.video_frames==0 && empty.audio_decoded==0,"disconnected diagnostics must be empty");
    const auto stoppedBefore=results.stopped.load();
    client.reset();require(results.stopped==stoppedBefore+1,"stopped callback was not joined safely during destruction");
    require(waitFor([&]{return !host->running();}),"client close did not disconnect desktop");host->stop();
  }
  // Cancel while the host has not answered: this uses the same stopped callback
  // path as a failed connection, before any video frame has been received.
  for(unsigned attempt=0;attempt<3;++attempt){
    auto before=results.stopped.load();
    std::unique_ptr<SanserMobileClient,Delete> pending(sanser_mobile_create("127.0.0.1:9",0,std::string(48,'m').c_str(),1,0,"",{&results,video,nullptr,stopped}));
    require(pending && sanser_mobile_start(pending.get()),"pending connection did not start");
    pending.reset();require(results.stopped==before+1,"cancel before handshake lost stopped callback");
  }
  std::cout<<"Mobile C ABI: authenticated desktop interoperability, view-only, keyboard, video, RTT and teardown passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
