#include "broadcast.h"
#include "broadcast_recovery.h"
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
    {
      // Virtual time verifies recovery deadlines without relying on CPU/thread
      // scheduling. A published IDR is not evidence that congestion recovered.
      SharingKeyframeRecovery slow, healthy;
      const auto start=Clock::time_point{}+std::chrono::seconds(100);
      slow.request(start);
      require(slow.take(start),"first viewer recovery must be immediate");
      for(unsigned ms=1;ms<500;++ms) {
        slow.request(start+std::chrono::milliseconds(ms));
        require(!slow.take(start+std::chrono::milliseconds(ms)),"repeat requests must not create a keyframe storm");
      }
      require(slow.take(start+std::chrono::milliseconds(500)),"bounded retry must not lose a pending recovery");
      slow.keyframe();
      slow.request(start+std::chrono::milliseconds(600));
      require(!slow.take(start+std::chrono::milliseconds(600)),"publishing an IDR must not reset a failing viewer's backoff");
      healthy.request(start+std::chrono::milliseconds(600));
      require(healthy.take(start+std::chrono::milliseconds(600)),"one viewer's backoff must not delay another viewer's first recovery");
      slow.keyframe();
      require(!slow.take(start+std::chrono::milliseconds(1500)),"a fresh IDR satisfies an older pending demand even after the cooldown expires");
      slow.request(start+std::chrono::milliseconds(1501));
      require(slow.take(start+std::chrono::milliseconds(1501)),"second retry after one second");
      slow.request(start+std::chrono::milliseconds(1600));
      require(!slow.take(start+std::chrono::milliseconds(3400)),"persistent failures back off to two seconds");
      require(slow.take(start+std::chrono::milliseconds(3501)),"persistent viewer retains a bounded recovery opportunity");
      slow.keyframe();
      slow.request(start+std::chrono::milliseconds(7000));
      require(slow.take(start+std::chrono::milliseconds(7000)),"a quiet interval restores fast recovery for a new loss episode");
      require(slow.signals()>slow.requests(),"stats distinguish coalesced signals from encoder requests");
    }
    Options source; source.host=true; source.input=false; source.bitrate=100000000;
    {
      auto desktop=source; desktop.bitrate=20000000; desktop.ultraLowLatency=true;
      Broadcaster group(desktop); Viewer first,second;
      connect(group,first,desktop,"desktop-first",'v');
      connect(group,second,desktop,"desktop-second",'w');
      group.publish(Frame{Bytes(256*1024,0x3c),1920,1080,true});
      require(waitFor([&] { return count(first)==1 && count(second)==1; }),"large desktop IDR must finish pacing for every sharing viewer");
      for(unsigned i=0;i<20;++i) {
        group.updateHealth();
        group.publish(Frame{Bytes(8000,static_cast<std::uint8_t>(i)),1920,1080,false});
        const auto current=[&](Viewer& viewer) {
          std::lock_guard lock(viewer.mutex);
          return !viewer.frames.empty() && viewer.frames.back().data==Bytes(8000,static_cast<std::uint8_t>(i));
        };
        // Behave like the real encoder loop while waiting for delivery. A
        // late startup/loss request is legitimate; a finite fixture which
        // never answers it can deadlock its dependent P-frame chain itself.
        // Compare the changed picture, since a healthy peer can receive both
        // the original P and its recovery IDR while another peer recovers.
        require(waitFor([&] {
          if(group.takeKeyframeRequest()) group.publish(Frame{Bytes(8000,static_cast<std::uint8_t>(i)),1920,1080,true});
          return current(first) && current(second);
        }),"sharing froze after its first desktop frame");
      }
      for(const auto& peer:group.stats()) require(peer.sendQueueDrops==0 && peer.pauses==0,"pacer's own serialization must not be treated as a slow viewer");
      const auto firstBefore=count(first),secondBefore=count(second);
      // A short burst of encoded reference pictures must survive dispatch
      // scheduling. The former single latest slot could replace its IDR/Ps.
      for(unsigned i=0;i<5;++i) group.publish(Frame{Bytes(8000,static_cast<std::uint8_t>(0x40+i)),1920,1080,i==0});
      require(waitFor([&] { return count(first)>=firstBefore+5 && count(second)>=secondBefore+5; }),"a short encoded burst must preserve its full reference chain for both viewers");
    }
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
    require(waitFor([&] { for(auto peer:host.stats()) if(peer.id=="viewer-1") return !peer.running && peer.stopReason==1; return false; }),"deliberate viewer disconnect is reported and isolated");
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

    {
      // Drive the encoder request API too. The earlier slow-viewer test only
      // supplied preselected IDRs and could not detect group-wide IDR storms.
      auto ultra=source; ultra.ultraLowLatency=true;
      Broadcaster group(ultra); Viewer healthy,slow;
      connect(group,healthy,ultra,"healthy",'h');
      connect(group,slow,ultra,"constrained",'q',1000000);
      require(group.bitrate()==1000000,"single encoder still follows the lowest active viewer budget");
      unsigned keyframes=0;
      for(unsigned i=0;i<240;++i) {
        const bool requested=group.takeKeyframeRequest();
        if(requested) ++keyframes;
        group.publish(Frame{Bytes(20000,static_cast<std::uint8_t>(i)),1280,720,requested || i==0});
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
      }
      bool constrained=false; std::uint64_t slowRequests=0;
      for(const auto& peer:group.stats()) {
        if(peer.id=="constrained") { constrained=peer.sendQueueDrops>0; slowRequests=peer.keyframeRequests; }
      }
      require(constrained,"test must actually saturate the constrained viewer");
      require(slowRequests>=2,"failed viewer still receives recovery retries");
      require(keyframes<=8,"constrained viewer must not force an IDR every 100 ms");
      require(count(healthy)>100,"healthy viewer continues while constrained viewer is recovering");
      std::cout<<"Broadcast recovery: published="<<group.publishedFrames()<<" forcedKeyframes="<<keyframes
        <<" constrainedRequests="<<slowRequests<<" healthyFrames="<<count(healthy)<<'\n';
    }
    {
      auto ultra=source; ultra.ultraLowLatency=true;
      Broadcaster group(ultra); Viewer healthy,slow;
      connect(group,healthy,ultra,"isolated-healthy",'i');
      connect(group,slow,ultra,"isolated-slow",'j',1000000);
      const auto state=[&] {
        for(const auto& peer:group.stats()) if(peer.id=="isolated-slow") return peer.videoState;
        return std::string("missing");
      };
      const auto deadline=Clock::now()+std::chrono::seconds(12);
      while(state()!="paused" && Clock::now()<deadline) {
        group.updateHealth();
        (void)group.takeKeyframeRequest();
        // Keep offering independent pictures above this viewer's 1 Mbps
        // budget. Dependent P-frames can enter IDR backoff and give the viewer
        // clean completed-frame windows, which correctly reset sustained
        // pressure. That tests intermittent recovery, not continuous overload.
        // IDR storm/backoff behavior is covered in the preceding case.
        group.publish(Frame{Bytes(20000,0x5c),1280,720,true});
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
      }
      if(state()!="paused") {
        for(const auto& peer:group.stats()) std::cerr<<"Isolation setup peer="<<peer.id<<" state="<<peer.videoState
          <<" submitted="<<peer.submitted<<" dispatched="<<peer.dispatched<<" replaced="<<peer.replaced
          <<" referenceDrops="<<peer.referenceDrops<<" sendQueueDrops="<<peer.sendQueueDrops
          <<" backoffMs="<<peer.backoffMs<<" completed="<<peer.completedFrame<<'\n';
      }
      require(state()=="paused","a persistently congested viewer is automatically paused");
      require(group.bitrate()>1000000,"paused viewer no longer sets the shared encoder bitrate");
      std::uint64_t dispatched=0,requests=0;
      for(const auto& peer:group.stats()) if(peer.id=="isolated-slow") { dispatched=peer.dispatched; requests=peer.keyframeRequests; }
      const auto healthyBefore=count(healthy);
      const auto pauseCheck=Clock::now()+std::chrono::seconds(1);
      while(Clock::now()<pauseCheck) {
        group.updateHealth();
        (void)group.takeKeyframeRequest();
        // This phase tests isolation, not reference-chain recovery (covered
        // above). Independent small IDRs keep a delayed/lost synthetic P-frame
        // from making the one-second assertion depend on a two-second repair
        // cooldown on a busy CI runner. A paused healthy peer would still fail.
        group.publish(Frame{Bytes(2500,0x5d),1280,720,true});
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
      }
      require(healthy.peer->ready() && slow.peer->ready(),"pause retains authenticated control/keepalives for both viewers");
      if(count(healthy)<=healthyBefore+20) {
        for(const auto& peer:group.stats()) std::cerr<<"Isolation peer="<<peer.id<<" state="<<peer.videoState
          <<" ready="<<peer.ready<<" submitted="<<peer.submitted<<" dispatched="<<peer.dispatched
          <<" replaced="<<peer.replaced<<" referenceDrops="<<peer.referenceDrops
          <<" sendQueueDrops="<<peer.sendQueueDrops<<" keyframes="<<peer.keyframeRequests<<'\n';
      }
      require(count(healthy)>healthyBefore+20,"healthy viewer continues while slow viewer is paused");
      for(const auto& peer:group.stats()) if(peer.id=="isolated-slow") {
        require(peer.dispatched<=dispatched+1,"paused delivery stops dispatching new video");
        require(peer.keyframeRequests==requests,"paused viewer does not demand group-wide keyframes");
      }
      bool probing=false;
      const auto recoveryDeadline=Clock::now()+std::chrono::seconds(10);
      while(Clock::now()<recoveryDeadline) {
        group.updateHealth();
        const auto current=state();
        if(current=="probing") {
          probing=true;
          require(group.bitrate()>1000000,"recovery probes cannot lower healthy viewers' bitrate");
        }
        if(probing && current=="active") break;
        group.publish(Frame{Bytes(1000,0x5e),1280,720,group.takeKeyframeRequest()});
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
      }
      require(probing && state()=="active","viewer automatically rejoins only after a new frame completes and live feedback is clean");
      require(group.bitrate()==1000000,"recovered viewer participates in the shared budget again");
      std::cout<<"Broadcast isolation: suspended slow video, retained control, healthy frames="<<count(healthy)-healthyBefore<<", automatic verified rejoin passed\n";
    }
    require(!parseSharingCommand("share-add\tnonce\tv\t1\t127.0.0.1:1\tshort",1000000),"reject short IPC credential");
    require(!parseSharingCommand(std::string(1025,'x'),1000000),"bounded IPC command");
    require(parseSharingCommand("share-remove\tnonce\tv",1000000).has_value(),"valid remove command");
    std::cout<<"Broadcast: three authenticated viewers, shared publication, isolated disconnect/slow link and bounded lifecycle passed\n";
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
