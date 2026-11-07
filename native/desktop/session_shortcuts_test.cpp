#include "session_shortcuts.h"
#include "tx_scheduler.h"
#include <iostream>
#include <set>
#include <stdexcept>
using namespace sanser::desktop;
void check(bool value,const char* message){if(!value)throw std::runtime_error(message);}
int main(){try{
  for(auto host:{ShortcutHost::Windows,ShortcutHost::Mac}) {
    std::set<std::string_view> ids;
    for(const auto& action:sessionShortcuts(host)) {
      check(ids.insert(action.id).second && !action.label.empty() && !action.caption.empty(),"invalid profile");
      check(validShortcutKeys(action.keys),"profile contains unsupported or duplicate key");
    }
  }
  for(const auto& invalid:std::vector<std::vector<std::uint16_t>>{{},{0},{50},{70},{101},{232},{227,227},{224,225,226,227,4}})
    check(!validShortcutKeys(invalid),"invalid key batch accepted");
  bool available=true,focused=true,accepted=true;std::uint32_t epoch=1;unsigned begins=0,sent=0;
  std::vector<std::uint16_t> last;std::uint32_t lastEpoch=0;
  ShortcutSession model({[&]{return available;},[&]{return focused;},[&]{return epoch;},[&]{++begins;return true;},
    [&](auto keys,auto generation){++sent;last.assign(keys.begin(),keys.end());lastEpoch=generation;return accepted;}});
  available=false;check(!model.open() && !begins,"unsupported action reset input");
  available=true;focused=false;check(!model.open(),"inactive viewer opened actions");focused=true;
  check(model.open() && begins==1 && !model.host() && !model.send("close"),"host OS guessed or opened without release");
  check(model.select(ShortcutHost::Mac) && model.send("close") && last==std::vector<std::uint16_t>{227,26} && lastEpoch==1,"Mac chord mapping");
  check(model.select(ShortcutHost::Windows) && model.send("close") && last==std::vector<std::uint16_t>{226,61},"Windows chord mapping");
  auto before=sent;check(!model.send("arbitrary-command") && sent==before,"unlisted shortcut sent");
  accepted=false;check(!model.send("save"),"queue rejection reported success");before=sent;
  for(unsigned i=0;i<5;++i)check(model.current(),"rejected action killed model");check(sent==before,"automatic action retry");
  accepted=true;focused=false;check(!model.send("save") && sent==before,"inactive app sent queued action");focused=true;
  ++epoch;check(!model.send("save") && sent==before,"new generation inherited action");model.close();
  check(model.open() && !model.host() && !model.send("save"),"reopened panel inherited OS/action");
  model.select(ShortcutHost::Windows);available=false;check(!model.send("close"),"lost permission sent action");model.close();
  available=true;epoch=0;check(!model.open(),"unauthenticated epoch accepted");
  TxScheduler tx;const auto now=std::chrono::steady_clock::now();
  const auto packet=[&](std::vector<std::uint8_t> bytes){TxScheduler::Packet value{};value.bytes=std::move(bytes);value.queued=now;return value;};
  tx.push(TxScheduler::InputLane,packet({1}));
  std::vector<TxScheduler::Packet> tooLarge{packet(std::vector<std::uint8_t>(256*1024)),packet({2})};
  check(!tx.pushInputBatch(std::move(tooLarge)),"oversized transaction queued");
  check(tx.pop(false)->bytes==std::vector<std::uint8_t>{1} && !tx.pop(false),"failed batch partially published");
  check(tx.pushInputBatch({packet({2}),packet({3}),packet({4})}),"bounded transaction refused");
  tx.push(TxScheduler::Audio,packet({9}));tx.push(TxScheduler::Control,packet({8}));
  check(tx.pop(false)->bytes[0]==8,"batch blocked control lane");
  for(unsigned code:{2,3,4})check(tx.pop(false)->bytes[0]==code,"batch edges reordered");
  check(tx.pop(false)->bytes[0]==9,"audio lost under input transaction");
  std::cout<<"Native shortcuts: fixed profiles, host selection, permission/focus/epoch gates and all-or-none input capacity passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
