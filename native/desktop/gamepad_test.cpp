#include "gamepad.h"
#include "input_worker.h"
#include "tx_scheduler.h"
#include "../platforms/windows/gamepad.h"
#include <iostream>
#include <stdexcept>
#include <condition_variable>
using namespace sanser::desktop;
void check(bool value,const char* text){if(!value)throw std::runtime_error(text);}
int main(){try{
  GamepadState neutral;neutral.connected=true;
  GamepadState held=neutral;held.buttons=0xf3ff;held.lt=255;held.rt=128;held.lx=-32768;held.ly=32767;held.rx=-1;held.ry=1;
  check(decodeGamepad(encodeGamepad(held))==held,"analog wire roundtrip/sign/endpoints");
  check(decodeGamepad(encodeGamepad({}))==GamepadState{},"disconnect wire");
  for(unsigned i=0;i<18;++i){auto bytes=encodeGamepad(held);bytes.resize(i);check(!decodeGamepad(bytes),"truncated state accepted");}
  auto bad=encodeGamepad(held);bad.push_back(0);check(!decodeGamepad(bad),"extended state accepted");
  bad=encodeGamepad(held);bad[5]=1;check(!decodeGamepad(bad),"reserved field accepted");
  bad=encodeGamepad(held);bad[4]=0;check(!decodeGamepad(bad),"disconnected press accepted");
  bad=encodeGamepad(held);bad[6]|=4;check(!decodeGamepad(bad),"reserved button accepted");
  GamepadLease lease;const auto now=GamepadLease::Clock::now();
  check(!lease.accept(held,now,now),"held controller armed without neutral");
  check(lease.accept(neutral,now,now)==neutral && lease.accept(held,now,now)==held,"neutral arming failed");
  check(!lease.expired(now+std::chrono::milliseconds(249)) && lease.expired(now+std::chrono::milliseconds(250)),"controller lease boundary");
  check(!lease.accept(held,now,now+std::chrono::milliseconds(251))->connected,"stale queued snapshot resurrected press");
  check(!lease.accept(held,now+std::chrono::milliseconds(252),now+std::chrono::milliseconds(252)),"expired lease rearmed while held");
  unsigned attaches=0,detaches=0,updates=0;bool accepted=true;
  {WindowsGamepad backend({[&]{++attaches;return accepted;},[&](auto s){++updates;check(s==held,"backend altered analog report");return accepted;},[&]{++detaches;}});
    check(backend.available() && backend.apply(held) && attaches==1 && updates==1,"attach/update backend");
    accepted=false;check(!backend.apply(held) && !backend.available() && detaches==1,"failed update left backend active");
    check(!backend.apply(held) && attaches==1,"failed press auto-replayed");
    check(backend.apply({}) && detaches==1,"repeated reset changed driver lifecycle");}
  {WindowsGamepad absent(WindowsGamepad::Driver{});check(!absent.available() && !absent.apply(held) && absent.apply({}),"absent driver claimed support");}
  accepted=false;{WindowsGamepad failed({[&]{++attaches;return false;},[](auto){return true;},[&]{++detaches;}});
    check(!failed.apply(held) && !failed.available(),"attach failure still available");}
  const auto packet=[&](unsigned value){TxScheduler::Packet p;p.bytes={static_cast<std::uint8_t>(value&255)};p.queued=now;return p;};
  TxScheduler tx;for(unsigned i=0;i<1000;++i)tx.latestGamepad(packet(i));
  tx.latestInput(packet(42));check(tx.pop(false)->bytes==std::vector<std::uint8_t>{42},"gamepad evicted mouse slot");
  check(tx.pop(false)->bytes==std::vector<std::uint8_t>{999&255} && !tx.hasPriority(),"gamepad snapshots accumulated");
  tx.latestGamepad(packet(1));tx.clearInput();check(!tx.hasPriority(),"reset retained gamepad slot");
  std::mutex mutex;std::condition_variable wake;std::vector<GamepadState> states;
  {InputWorker worker([&](Input input){std::lock_guard lock(mutex);if(input.kind==Input::Gamepad)states.push_back(input.gamepad);wake.notify_all();},[]{throw std::runtime_error("worker failed");});
    Input event;event.kind=Input::Gamepad;event.gamepad=neutral;check(worker.submit(event),"submit neutral");
    {std::unique_lock lock(mutex);check(wake.wait_for(lock,std::chrono::seconds(1),[&]{return states.size()==1;}),"worker did not arm");}
    event.gamepad=held;worker.submit(event);
    {std::unique_lock lock(mutex);check(wake.wait_for(lock,std::chrono::seconds(1),[&]{return states.size()==3;}),"idle input worker failed to release expired controller");
      check(states[1]==held && states[2]==GamepadState{},"watchdog changed/missed state");}
    worker.submit(event);std::this_thread::sleep_for(std::chrono::milliseconds(30));
    {std::lock_guard lock(mutex);check(states.size()==3,"held snapshot rearmed expired worker");}
  }
  std::cout<<"Gamepad wire/lease, independent latest slot, idle watchdog and optional backend failure checks passed\n";return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
