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
  {
    // Captures refused while the budget still looks healthy: step down twice.
    ResolutionPolicy p;
    p.update(10'000'000,t0,true);
    check(p.update(10'000'000,t0+1600ms,true) && p.level()==1,"starved sender reduces even at a high budget");
    p.update(10'000'000,t0+2s,true);
    check(p.level()==1,"second step waits for the dwell");
    p.update(10'000'000,t0+9700ms,true);
    check(p.level()==2,"still starved after the dwell: half size");
    p.update(10'000'000,t0+10s,true);
    check(p.level()==2,"never below half size");
    // Healthy again: restores one level at a time, slowly after starvation.
    p.update(10'000'000,t0+11s,false);
    check(p.update(10'000'000,t0+25s,false) && p.level()==2,"starved reduction waits 20 s");
    p.update(10'000'000,t0+31100ms,false);
    check(p.level()==1,"one level back after 20 s healthy");
    p.update(10'000'000,t0+40s,false);
    p.update(10'000'000,t0+60100ms,false);
    check(p.level()==0,"full size again");
  }
  {
    using sanser::desktop::CaptureAdmission;
    CaptureAdmission a;
    check(!a.starved(0,0,t0),"no history");
    check(!a.starved(30,10,t0+500ms),"too short a window");
    check(a.starved(50,15,t0+900ms),"50 changes, 15 sent: starved");
    CaptureAdmission idle;
    idle.starved(0,0,t0);
    check(!idle.starved(5,1,t0+900ms),"an idle desktop is never starved");
    CaptureAdmission fine;
    fine.starved(0,0,t0);
    check(!fine.starved(60,55,t0+900ms),"most changes sent: fine");
  }
  check(ResolutionPolicy::dimension(1920,2)==960,"1920 -> 960");
  check(ResolutionPolicy::dimension(1080,2)==540,"1080 -> 540");
  check(ResolutionPolicy::dimension(1920,0)==1920,"level 0 keeps the size");
  check(ResolutionPolicy::reducedDimension(1920)==1280,"1920 -> 1280");
  check(ResolutionPolicy::reducedDimension(1080)==720,"1080 -> 720");
  check(ResolutionPolicy::reducedDimension(1661)%2==0,"reduced size stays even");
  check(ResolutionPolicy::reducedDimension(64)==64,"never below 64");
  if(failures) return EXIT_FAILURE;
  std::cout<<"resolution policy tests passed\n";
  return EXIT_SUCCESS;
}
