#include "user_messages.h"
#include "user_ipc.h"
#include "tx_scheduler.h"
#include <iostream>
#include <stdexcept>
using namespace sanser::desktop;
void check(bool value,const char* message){if(!value)throw std::runtime_error(message);}
int main(){try{
  UserMessages sender,receiver;const auto now=UserMessages::Clock::now();
  const std::vector<std::uint8_t> body(8192,123);check(sender.queue(body,now),"queue clipboard-sized message");
  unsigned delivered=0;auto time=now;
  for(unsigned step=0;step<100 && !delivered;++step){
    auto packets=sender.poll(time,std::chrono::milliseconds(60));std::reverse(packets.begin(),packets.end());
    for(const auto& packet:packets){
      if(step==0 && UserMessages::fragment(packet)==0)continue; // Lose a fragment.
      auto received=receiver.receive(packet,time);if(received.body){check(*received.body==body,"reassembled content corrupted");++delivered;}
      if(received.ack && step!=1)sender.receive(*received.ack,time); // Lose ACKs too.
      check(!receiver.receive(packet,time).body,"duplicate message was applied twice");
    }
    time+=std::chrono::milliseconds(20);
  }
  check(delivered==1,"message failed to recover after reordering/loss");
  auto retry=sender.poll(time+std::chrono::milliseconds(100),std::chrono::milliseconds(60));
  for(auto& packet:retry){auto result=receiver.receive(packet,time);check(!result.body,"late duplicate delivered twice");if(result.ack)sender.receive(*result.ack,time);}
  check(!sender.queue({}),"empty application body accepted");check(!sender.queue(std::vector<std::uint8_t>(32769)),"oversized message accepted");
  UserMessages bounded;for(unsigned i=0;i<4;++i)check(bounded.queue(body,now),"bounded message slots");check(!bounded.queue(body,now),"unbounded in-flight messages");
  auto malformed=bounded.poll(now,std::chrono::milliseconds(60))[0];malformed[5]=1;check(!receiver.receive(malformed).ack,"invalid version flags accepted");
  TxScheduler scheduler;TxScheduler::Packet data{{1},now};data.userMessage=1;data.userFragment=0;
  check(scheduler.push(TxScheduler::UserData,data) && scheduler.push(TxScheduler::UserData,data),"coalesce queued data retry");
  scheduler.push(TxScheduler::Control,{{2},now});scheduler.push(TxScheduler::Audio,{{3},now});scheduler.push(TxScheduler::Video,{{4},now});
  check(scheduler.pop(true,true)->bytes[0]==2 && scheduler.pop(true,true)->bytes[0]==3,"user data blocked control/audio");
  check(scheduler.pop(true,true)->bytes[0]==4,"user data bypassed video budget");
  check(scheduler.pop(false,true)->bytes[0]==1 && !scheduler.hasUserData(),"duplicate retry accumulated in TX lane");
  const std::string id="018f4d89-5e8b-7a80-bd1e-cb7cb9f43189";
  const auto command=parseUserCommand("clipboard-write\t"+id+"\t"+userHex("Tiếng Việt 😀"));
  check(command && command->body=="Tiếng Việt 😀","private IPC Unicode round trip");
  check(!parseUserCommand("user-text\tbad\t61") && !parseUserCommand("clipboard-read\t"+id+"\textra"),"malformed private command accepted");
  check(sanser::clipboardUtf16("\r\n\t").has_value() && !sanser::clipboardUtf16(std::string("a\0b",3)),"clipboard NUL validation");
  std::cout<<"User data: bounded reassembly, loss/ACK recovery, duplicate suppression, TX priority and private IPC passed\n";return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
