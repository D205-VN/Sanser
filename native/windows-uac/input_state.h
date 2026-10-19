#pragma once
#include "protocol.h"
#include <map>
namespace sanser::uac {
// Record only events accepted by SendInput. Disconnect emits releases, never
// another key/button down or a stale absolute pointer movement.
class InputState {
public:
  void applied(const Request& event) {
    if(event.type==1) {
      const auto flags=event.flags&~2U;
      const auto id=0x1000000U|(flags<<16)|event.data;
      if(event.flags&2U) releases_.erase(id);
      else { auto up=event; up.flags|=2U; releases_[id]=up; }
    } else {
      constexpr std::uint32_t downs[]={2,8,32,128},ups[]={4,16,64,256};
      for(unsigned i=0;i<4;++i) {
        const auto data=i==3?event.data:0;
        const auto id=(i<<16)|data;
        if(event.flags&ups[i]) releases_.erase(id);
        if(event.flags&downs[i]) { Request up; up.operation=Operation::Input; up.flags=ups[i]; up.data=data; releases_[id]=up; }
      }
    }
  }
  template<class Inject> void release(Inject inject) {
    for(const auto& [id,event]:releases_) { (void)id; inject(event); }
    releases_.clear();
  }
private:
  std::map<std::uint32_t,Request> releases_;
};
} // namespace sanser::uac
