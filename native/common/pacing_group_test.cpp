#include "pacing_group.h"
#include <iostream>
#include <stdexcept>

void check(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
struct Trace { unsigned waits=0; std::int64_t last=0,deadline=0; };
Trace run(unsigned mbps,bool ultra) {
  sanser::PacingGroup group;
  Trace result;
  const std::int64_t spacing=1200*8/mbps;
  std::int64_t now=0,due=0;
  unsigned advanced=0;
  // Four consecutive 64-datagram frames use one bandwidth/group timeline.
  for(unsigned i=0;i<256;++i) {
    if(group.waitRequired(ultra,due-now)) {
      now=due+25; // Controlled 25 us wake overshoot, not a Windows benchmark.
      group.waited(); ++result.waits; advanced=0;
    }
    if(now<due) {
      check(ultra && due-now<=sanser::PacingGroup::maxAdvanceMicros,"group exceeded its advance budget");
      check(++advanced<=sanser::PacingGroup::maxPackets,"too many advanced datagrams in one wake group");
    }
    group.datagram();
    result.last=now; due+=spacing;
  }
  result.deadline=due;
  return result;
}
int main() {
  try {
    for(unsigned rate : {20U,40U,80U}) {
      const auto baseline=run(rate,false),grouped=run(rate,true);
      check(grouped.waits<baseline.waits,"small UDP groups did not reduce timer wake calls");
      check(grouped.deadline==baseline.deadline,"grouping changed the bandwidth timeline");
      check(grouped.last>=baseline.last-525,"grouping released a frame beyond its bounded advance");
      std::cout<<"virtualMbps="<<rate<<" perPacketWaits="<<baseline.waits<<" groupedWaits="<<grouped.waits
        <<" finalSendAdvanceUs="<<baseline.last-grouped.last<<'\n';
    }
    sanser::PacingGroup group;
    check(group.waitRequired(true,501),"positive pacing debt beyond 500 us must wait");
    for(unsigned i=0;i<4;++i) group.datagram();
    check(group.waitRequired(true,1),"group packet cap must enforce a wait");
    check(!group.waitRequired(true,-1000),"late scheduling must not add artificial sleep");
    group.waited();
    check(!group.waitRequired(true,500) && group.waitRequired(false,1),"Ultra grouping changed Auto pacing");
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
