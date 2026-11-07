#define SANSER_LATENCY_TEST 1
#include "src/main.mm"
namespace {
struct NativeInputPolicyTest {
  static auto queued(NativeInputSender& sender){std::lock_guard lock(sender.mutex_);return sender.urgentQueue_.size()+sender.pendingBatches_.size();}
};
void check(bool value,const char* message){if(!value)throw std::runtime_error(message);}
}
int main(){@autoreleasepool{try{
  ScopedFd receiver(socket(AF_INET,SOCK_DGRAM,0)),outgoing(socket(AF_INET,SOCK_DGRAM,0));
  sockaddr_in local{};local.sin_family=AF_INET;local.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
  check(bind(receiver.get(),reinterpret_cast<sockaddr*>(&local),sizeof(local))==0,"bind loopback");
  socklen_t length=sizeof(local);check(getsockname(receiver.get(),reinterpret_cast<sockaddr*>(&local),&length)==0,"get loopback endpoint");
  UdpEndpoint endpoint{};endpoint.length=sizeof(local);std::memcpy(&endpoint.address,&local,sizeof(local));
  const std::string token="legacy-shortcut-test-token";
  auto sender=std::make_shared<NativeInputSender>();sender->setAuthRequired(true,token);sender->setUdpTarget(outgoing.get(),endpoint);
  sender->setControlAuthenticated(true);sender->setPacketAuthEnabled(true);sender->setBatchEnabled(true);
  check(!sender->shortcutGeneration(),"old host implicitly enabled shortcut capability");
  sender->setShortcutCapability(true);auto epoch=sender->shortcutGeneration();check(epoch,"authenticated capability unavailable");
  check(!sender->sendHostShortcut(std::vector<std::uint16_t>{224,6},epoch),"action before panel release");
  check(!sender->beginShortcuts(epoch+1),"stale UI generation accepted");
  sender->setViewOnly(true);check(!sender->shortcutGeneration() && !sender->beginShortcuts(epoch),"view-only enabled shortcut");
  sender->setViewOnly(false);epoch=sender->shortcutGeneration();check(epoch && sender->beginShortcuts(epoch),"control panel reset unavailable");
  check(!sender->sendJson("{\"type\":\"key-down\",\"keyCode\":0}") && !sender->sendJson("{\"type\":\"gamepad-state\",\"connected\":true}") && !sender->sendJson("{\"type\":\"host-shortcut\",\"action\":\"copy\"}"),"physical/unguarded shortcut bypassed capture scope");
  check(!sender->sendHostShortcut(std::vector<std::uint16_t>{227,26},epoch),"Mac-only/arbitrary chord accepted by Windows legacy host");
  check(sender->sendHostShortcut(std::vector<std::uint16_t>{224,6},epoch) && sender->sendHostShortcut(std::vector<std::uint16_t>{224,25},epoch),"queue named Copy/Paste");
  check(sender->sendJson("{\"type\":\"control-ping\"}"),"shortcut barrier blocked control ping");
  std::uint64_t authSequence=0;
  auto receive=[&](std::string_view marker,std::string_view forbidden={}){
    const auto deadline=std::chrono::steady_clock::now()+std::chrono::seconds(3);
    while(std::chrono::steady_clock::now()<deadline){
      pollfd readable{receiver.get(),POLLIN,0};if(poll(&readable,1,100)<=0)continue;
      char bytes[8192];const auto size=recv(receiver.get(),bytes,sizeof(bytes),0);check(size>1 && bytes[0]==2,"legacy control mux");
      std::string body;check(unwrapSecureControlEnvelope(std::string(bytes+1,size-1),token,"c2h",authSequence,body),"shortcut message not authenticated");
      check(forbidden.empty() || body.find(forbidden)==std::string::npos,"next shortcut bypassed unacknowledged predecessor");
      if(body.find(marker)!=std::string::npos)return body;
    }throw std::runtime_error("timed out waiting for named shortcut");
  };
  const auto ack=[&](const std::string& body){sender->observeInputAck(jsonUint64Value(body,"sequence"),1,1,1,0,false,false,false,true);};
  const auto reset=receive("shortcut-panel","\"action\"");check(jsonStringValue(reset,"type")=="input-batch","entry reset lacks reliable batch");ack(reset);
  const auto copy=receive("\"action\":\"copy\"");
  const auto copyRetry=receive("\"action\":\"copy\"","\"action\":\"paste\"");
  check(jsonUint64Value(copy,"sequence")==jsonUint64Value(copyRetry,"sequence"),"lost ACK retried as a new operation");
  ack(copy);const auto paste=receive("\"action\":\"paste\"");check(jsonUint64Value(paste,"sequence")>jsonUint64Value(copy,"sequence"),"operation sequence order");ack(paste);
  check(copy.find("keyCode")==std::string::npos && copy.find("clipboard")==std::string::npos,"shortcut translated through physical Mac keys or local clipboard");
  check(sender->beginShortcuts(epoch),"reopen failed-ACK check");
  check(sender->sendHostShortcut(std::vector<std::uint16_t>{224,6},epoch) && sender->sendHostShortcut(std::vector<std::uint16_t>{224,25},epoch),"queue failed-ACK check");
  ack(receive("shortcut-panel"));const auto rejected=receive("\"action\":\"copy\"");
  sender->observeInputAck(jsonUint64Value(rejected,"sequence"),0,1,1,0,false,false,false,true);
  check(sender->shortcutGeneration()!=epoch && NativeInputPolicyTest::queued(*sender)==0,"failed Copy ACK unlocked dependent Paste");
  epoch=sender->shortcutGeneration();check(sender->beginShortcuts(epoch),"reopen retry-deadline check");
  check(sender->sendHostShortcut(std::vector<std::uint16_t>{224,25},epoch),"queue retry-deadline check");
  const auto expiry=std::chrono::steady_clock::now()+std::chrono::seconds(3);
  while(sender->shortcutGeneration()==epoch && std::chrono::steady_clock::now()<expiry){
    pollfd readable{receiver.get(),POLLIN,0};if(poll(&readable,1,20)<=0)continue;
    char bytes[8192];const auto size=recv(receiver.get(),bytes,sizeof(bytes),0);check(size>1,"deadline datagram");
    std::string body;check(unwrapSecureControlEnvelope(std::string(bytes+1,size-1),token,"c2h",authSequence,body),"deadline envelope");
    check(body.find("\"action\":\"paste\"")==std::string::npos,"deadline unlocked action before reset ACK");
  }
  check(sender->shortcutGeneration()!=epoch,"unconfirmed reset did not cancel panel generation");
  sender->resumePhysicalInput();epoch=sender->shortcutGeneration();
  check(sender->beginShortcuts(epoch),"reopen bounded queue");unsigned accepted=0;
  for(unsigned i=0;i<200;++i)accepted+=sender->sendHostShortcut(std::vector<std::uint16_t>{224,6},epoch);
  check(accepted==127 && NativeInputPolicyTest::queued(*sender)==128,"shortcut admission unbounded or partially evicted");
  sender->resumePhysicalInput();check(NativeInputPolicyTest::queued(*sender)==0 && !sender->sendHostShortcut(std::vector<std::uint16_t>{224,6},epoch),"cancel retained/revived queued shortcuts");
  auto fresh=sender->shortcutGeneration();check(fresh && fresh!=epoch,"resume did not invalidate old UI");
  sender->setControlAuthenticated(false);check(!sender->shortcutGeneration(),"authentication loss left actions active");
  sender->setControlAuthenticated(true);sender->setPacketAuthEnabled(true);sender->setBatchEnabled(true);sender->setShortcutCapability(true);
  fresh=sender->shortcutGeneration();check(fresh && fresh!=epoch,"reauth reused action epoch");
  gNativeInputSender=sender;gExpectedSessionToken=token;gHostPacketAuthVerified=true;gLastHostSecureSequence=0;
  const auto pong=[&](std::uint64_t seq,bool capable,bool paused){
    handleHostControlPayload(makeSecureControlEnvelope(token,"h2c",seq,
      "{\"type\":\"control-pong\",\"hostShortcuts\":"+std::string(capable ? "true":"false")+
      ",\"capturePaused\":"+(paused ? "1":"0")+",\"sentSteadyMicros\":"+std::to_string(steadyMicros()-1)+"}"));
  };
  pong(1,true,true);check(!sender->shortcutGeneration(),"capture pause retained action capability");
  pong(2,true,false);check(sender->shortcutGeneration() && sender->shortcutGeneration()!=fresh,"capture resume revived old epoch");
  pong(3,false,false);check(!sender->shortcutGeneration(),"signed host revocation ignored");
  handleHostControlPayload("{\"type\":\"control-pong\",\"hostShortcuts\":true}");check(!sender->shortcutGeneration(),"unsigned capability enabled actions");
  sender->setViewOnly(true);gNativeInputSender.reset();
  std::cout<<"Legacy shortcuts: authenticated UDP capability, bounded named actions, ACK ordering/retry, physical isolation and cancellation passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';gNativeInputSender.reset();return 1;}}}
