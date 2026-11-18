#include "privacy_screen_policy.h"
#include <iostream>
#include <stdexcept>
using namespace sanser::privacy;
static void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
int main() {
  try {
    State s; require(!showOverlay(s) && marker(s).empty(),"disabled by default, silent");
    s.enabled=true; require(!showOverlay(s),"no overlay before a viewer connects");
    s.sessionReady=true; require(!showOverlay(s),"never show an overlay that capture would include");
    require(marker(s)=="SNPRIVACY overlay=unavailable blockInput=off","unavailable marker");
    s.captureExcluded=true; require(showOverlay(s) && !blockLocalInput(s),"overlay without blocking");
    s.blockRequested=true; require(!blockLocalInput(s),"blocking needs the elevated helper");
    require(marker(s)=="SNPRIVACY overlay=shown blockInput=unavailable","helper unavailable marker");
    s.blockAvailable=true; require(blockLocalInput(s),"blocking while shown");
    require(marker(s)=="SNPRIVACY overlay=shown blockInput=on","blocking marker");
    s.dismissed=true; require(!showOverlay(s) && !blockLocalInput(s),"local hotkey removes overlay and blocking");
    require(marker(s)=="SNPRIVACY overlay=dismissed blockInput=idle","dismissed marker");
    s.dismissed=false; s.sessionReady=false; require(!showOverlay(s) && !blockLocalInput(s),"session end removes overlay");
    std::cout<<"Privacy screen policy: host-only enablement, capture exclusion and local escape passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
