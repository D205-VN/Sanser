#include "resolution_policy.h"
#include <cstdlib>
#include <iostream>

using sanser::desktop::ResolutionPolicy;
using namespace std::chrono_literals;

namespace {
int failures=0;
void check(bool ok,const char* what) { if(!ok) { std::cerr<<"FAIL: "<<what<<'\n'; ++failures; } }
}

int main() {
  const auto t0=ResolutionPolicy::Clock::time_point{}+1h;
  {
    ResolutionPolicy p;
    check(!p.update(6'000'000,t0),"healthy budget keeps full size");
    check(!p.update(3'000'000,t0+100ms),"a brief dip does not reduce");
    check(!p.update(6'000'000,t0+500ms),"recovery cancels a pending reduction");
    check(!p.update(3'000'000,t0+1s),"new dip starts a new hold");
    check(!p.update(3'000'000,t0+2s),"still inside the 1.5 s hold");
    check(p.update(3'000'000,t0+2600ms),"sustained low budget reduces");
  }
  {
    ResolutionPolicy p;
    p.update(3'000'000,t0); p.update(3'000'000,t0+2s);
    check(p.reduced(),"reduced after sustained dip");
    // Budget recovers quickly, but the dwell keeps the reduced size for 8 s.
    p.update(6'000'000,t0+2100ms);
    check(p.update(6'000'000,t0+8100ms),"restore waits for the minimum dwell");
    check(!p.update(6'000'000,t0+10100ms),"restore after dwell and 6 s healthy");
  }
  {
    ResolutionPolicy p;
    p.update(3'000'000,t0); p.update(3'000'000,t0+2s);
    // Between thresholds: neither restores nor stays pending.
    for(int i=0;i<30;++i) p.update(4'800'000,t0+2s+std::chrono::seconds(i));
    check(p.reduced(),"hysteresis band keeps the reduced size");
  }
  check(ResolutionPolicy::reducedDimension(1920)==1280,"1920 -> 1280");
  check(ResolutionPolicy::reducedDimension(1080)==720,"1080 -> 720");
  check(ResolutionPolicy::reducedDimension(1661)%2==0,"reduced size stays even");
  check(ResolutionPolicy::reducedDimension(64)==64,"never below 64");
  if(failures) return EXIT_FAILURE;
  std::cout<<"resolution policy tests passed\n";
  return EXIT_SUCCESS;
}
