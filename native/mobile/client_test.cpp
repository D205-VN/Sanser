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
struct Results {std::atomic<unsigned> frames{0},keys{0};};
void video(void *context,const uint8_t *bytes,size_t size,uint32_t width,uint32_t height,int key){if(size==5000 && bytes[0]==42 && width==1280 && height==720 && key)static_cast<Results*>(context)->frames++;}
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
      try {host=std::make_unique<Peer>(options);client.reset(sanser_mobile_create(("127.0.0.1:"+std::to_string(port)).c_str(),port+1,options.token.c_str(),control,0,"",{&results,video,nullptr,nullptr}));}catch(...){host.reset();}
    }
    require(host && client,"unable to allocate test ports");
    host->onInput=[&](Input i){if(i.kind==Input::Key)results.keys++;};
    PeerStop stop{*host};host->start();require(sanser_mobile_start(client.get()),"client start failed");
    require(waitFor([&]{return host->ready() && sanser_mobile_ready(client.get());}),"authenticated handshake failed");
    sanser_mobile_input(client.get(),2,0,0,4,0,1);sanser_mobile_input(client.get(),2,0,0,4,0,0);
    if(control)require(waitFor([&]{return results.keys==2;}),"keyboard did not reach desktop");
    else {std::this_thread::sleep_for(std::chrono::milliseconds(100));require(results.keys==0,"view-only injected keyboard");}
    const auto before=results.frames.load();host->video(Frame{Bytes(5000,42),1280,720,true});
    require(waitFor([&]{return results.frames>before;}),"fragmented video did not reach C callback");
    require(waitFor([&]{return sanser_mobile_rtt(client.get())>=0;}),"unmeasured RTT");
    client.reset();require(waitFor([&]{return !host->running();}),"client close did not disconnect desktop");host->stop();
  }
  std::cout<<"Mobile C ABI: authenticated desktop interoperability, view-only, keyboard, video, RTT and teardown passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
