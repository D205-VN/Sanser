#include "broadcast.h"
#include <array>
#include <iostream>
#include <random>
#include <stdexcept>

using namespace sanser::desktop;
void require(bool ok,const char* message) { if(!ok) throw std::runtime_error(message); }
template<class F> bool waitFor(F condition) {
  const auto deadline=Clock::now()+std::chrono::seconds(5);
  while(Clock::now()<deadline) { if(condition()) return true; std::this_thread::sleep_for(std::chrono::milliseconds(2)); }
  return condition();
}
struct Viewer {
  std::unique_ptr<Peer> peer;
  std::mutex mutex;
  std::vector<Frame> frames;
  ViewerSpec spec;
  ~Viewer() { if(peer) peer->stop(); }
};
void connect(Broadcaster& host,Viewer& viewer,const Options& source,const std::string& id,char key,std::uint32_t budget=0) {
  std::random_device random;
  for(unsigned attempt=0;attempt<20 && !viewer.peer;++attempt) {
    const auto port=static_cast<std::uint16_t>(25000+random()%25000);
    auto options=source; options.host=false; options.port=port+1; options.peer="127.0.0.1:"+std::to_string(port); options.token=std::string(48,key);
    try {
      auto peer=std::make_unique<Peer>(options);
      peer->onFrame=[&viewer](Frame frame) { std::lock_guard lock(viewer.mutex); viewer.frames.push_back(std::move(frame)); };
      viewer.spec={id,"127.0.0.1:"+std::to_string(port+1),port,options.token,budget ? budget : source.bitrate};
      host.add(viewer.spec); viewer.peer=std::move(peer); viewer.peer->start();
    } catch(...) { host.remove(id); }
  }
  require(viewer.peer && waitFor([&] { return viewer.peer->ready(); }),"sharing viewer handshake");
  require(waitFor([&] { const auto stats=host.stats(); for(const auto& peer:stats) if(peer.id==id) return peer.ready; return false; }),"host viewer handshake");
}
std::size_t count(Viewer& viewer) { std::lock_guard lock(viewer.mutex); return viewer.frames.size(); }
int main() {
  try {
    Options source; source.host=true; source.input=false; source.bitrate=100000000;
    Broadcaster host(source); std::array<Viewer,3> viewers;
    for(std::size_t i=0;i<3;++i) connect(host,viewers[i],source,"viewer-"+std::to_string(i),static_cast<char>('a'+i));
    bool rejected=false;
    try { host.add({"fourth","127.0.0.1:59999",59998,std::string(48,'z'),source.bitrate}); } catch(...) { rejected=true; }
    require(rejected && host.stats().size()==3,"fourth viewer must be rejected before allocating a socket");
    host.publish(Frame{Bytes(2500,0x6a),1280,720,true});
    require(waitFor([&] { return count(viewers[0]) && count(viewers[1]) && count(viewers[2]); }),"all three viewers receive one published frame");
    require(host.publishedFrames()==1,"one encoded publication feeds three viewers");
    for(auto& viewer:viewers) { std::lock_guard lock(viewer.mutex); require(viewer.frames[0].data==Bytes(2500,0x6a) && viewer.frames[0].width==1280,"viewer frame contents"); }
    viewers[1].peer->stop();
    require(waitFor([&] { for(auto peer:host.stats()) if(peer.id=="viewer-1") return !peer.running; return false; }),"one viewer disconnect is isolated");
    host.publish(Frame{Bytes(2500,0x7b),1280,720,true});
    require(waitFor([&] { return count(viewers[0])>=2 && count(viewers[2])>=2; }),"survivors continue after another viewer closes");
    host.remove("viewer-1"); host.remove("viewer-1");
    require(host.stats().size()==2 && host.running(),"removing a viewer is idempotent and keeps the encoder output running");

    // A slow pacing budget must affect only that viewer's video queue.
    viewers[1].peer.reset(); viewers[1].frames.clear();
    connect(host,viewers[1],source,"slow",'s',1000000);
    for(unsigned i=0;i<40;++i) {
      host.publish(Frame{Bytes(20000,static_cast<std::uint8_t>(i)),1280,720,i%10==0});
      std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    require(waitFor([&] { for(const auto& peer:host.stats()) if(peer.id=="slow") return peer.sendQueueDrops>0; return false; }),"slow viewer sheds its stale queued video");
    host.publish(Frame{Bytes(2500,0x4f),1280,720,true});
    require(waitFor([&] { std::lock_guard lock(viewers[0].mutex); return viewers[0].frames.back().data==Bytes(2500,0x4f); }),"healthy viewer recovers latest frame without waiting for slow viewer");
    require(count(viewers[0])>10 && count(viewers[2])>10,"healthy viewers keep receiving while another link is constrained");
    host.stop(); require(!host.running() && host.stats().empty(),"group stop joins workers and clears peers");
    require(!parseSharingCommand("share-add\tnonce\tv\t1\t127.0.0.1:1\tshort",1000000),"reject short IPC credential");
    require(!parseSharingCommand(std::string(1025,'x'),1000000),"bounded IPC command");
    require(parseSharingCommand("share-remove\tnonce\tv",1000000).has_value(),"valid remove command");
    std::cout<<"Broadcast: three authenticated viewers, shared publication, isolated disconnect/slow link and bounded lifecycle passed\n";
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
