#include "display_select.h"
#include "transport.h"
#include <iostream>
#include <memory>
#include <random>
#include <stdexcept>
using namespace sanser::desktop;
namespace {
void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
template<class F> bool waitFor(F condition) {
  const auto deadline=Clock::now()+std::chrono::seconds(5);
  while(Clock::now()<deadline) { if(condition()) return true; std::this_thread::sleep_for(std::chrono::milliseconds(10)); }
  return condition();
}
display::List twoDisplays(std::uint8_t selected) {
  display::List list; list.selected=selected;
  list.displays={{2560,1440,true},{1080,1920,false}};
  return list;
}
void codec() {
  const auto list=twoDisplays(1);
  const auto bytes=display::encodeList(list);
  require(bytes.size()==16,"SDL1 size");
  require(display::decodeList(bytes)==list,"SDL1 round trip");
  auto bad=bytes; bad[4]=2; require(!display::decodeList(bad),"selected index must be inside the list");
  bad=bytes; bad[5]=3; require(!display::decodeList(bad),"count must match the payload length");
  bad=bytes; bad[10]=2; require(!display::decodeList(bad),"unknown display flags are rejected");
  bad=bytes; bad[6]=bad[7]=0; require(!display::decodeList(bad),"zero-sized display is rejected");
  require(display::encodeList(display::List{}).empty(),"empty list is never announced");
  display::List many; many.displays.assign(17,{800,600,false});
  require(display::encodeList(many).empty(),"list is bounded");
  require(display::decodeSelect(display::encodeSelect(3))==3,"SDS1 round trip");
  require(!display::decodeSelect(Bytes{'S','D','S','1'}),"SDS1 needs an index");
  require(display::parseCommand("display\t1")==1 && display::parseCommand("display\t15")==15,"stdin command");
  for(const char* line:{"display\t","display\t16","display\t-1","display\t1x","display 1","display\t001"})
    require(!display::parseCommand(line),"malformed display command must be rejected");
  require(display::formatRecord(list)=="SNUSER_DISPLAY\t1\t2560x1440x1,1080x1920x0\n","stdout record");
  require(display::formatRecord(std::nullopt)=="SNUSER_DISPLAY\t-\t\n","empty stdout record");
}
void retry() {
  using C=display::Request::Clock; const auto now=C::now();
  display::Request request; std::optional<display::List> host=twoDisplays(0);
  require(!request.choose(2,host,now),"out-of-range choice");
  require(!request.choose(0,std::nullopt,now),"no list, no choice");
  require(request.choose(1,host,now) && request.due(host,now)==1,"pending choice is resent");
  host->selected=1; require(!request.due(host,now),"host confirmation clears the choice");
  host->selected=0; require(request.choose(1,host,now),"choose again");
  require(!request.due(host,now+std::chrono::seconds(6)),"unanswered choice expires");
}
void loopback() {
  std::unique_ptr<Peer> host,client; std::random_device random;
  for(unsigned attempt=0;attempt<10 && !client;++attempt) {
    const auto port=static_cast<uint16_t>(30000+random()%20000);
    Options h; h.host=true; h.ultraLowLatency=true; h.port=port; h.peer="127.0.0.1:"+std::to_string(port+1); h.token=std::string(48,'d');
    Options c=h; c.host=false; c.port=port+1; c.peer="127.0.0.1:"+std::to_string(port);
    try { host=std::make_unique<Peer>(h); client=std::make_unique<Peer>(c); } catch(...) { host.reset(); }
  }
  require(host && client,"unable to allocate loopback ports");
  PeerStop stopHost{*host},stopClient{*client};
  host->setDisplays(twoDisplays(0)); host->start(); client->start();
  require(waitFor([&] { return host->ready() && client->ready(); }),"handshake timed out");
  require(waitFor([&] { return client->displays()==twoDisplays(0); }),"viewer did not receive the host display list");
  require(!client->selectDisplay(5),"viewer cannot request an unadvertised display");
  require(client->selectDisplay(1),"viewer request queued");
  std::optional<std::uint8_t> requested;
  require(waitFor([&] { requested=host->takeDisplayRequest(); return requested.has_value(); }),"host did not receive the display choice");
  require(*requested==1,"host received the wrong display");
  host->setDisplays(twoDisplays(1));
  require(waitFor([&] { const auto list=client->displays(); return list && list->selected==1; }),"viewer did not observe the switched display");
  std::this_thread::sleep_for(std::chrono::milliseconds(300));
  require(!host->takeDisplayRequest(),"a confirmed choice must not be replayed");
  client->stop(); host->stop();
}
}
int main() {
  try {
    codec(); retry(); loopback();
    std::cout<<"Display selection: SDL1/SDS1 codec, bounded retry and authenticated loopback switch passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
