#import "controller_input.h"
#include <iostream>
#include <stdexcept>
using namespace sanser::desktop;
void check(bool value,const char* text){if(!value)throw std::runtime_error(text);}
int main(){@autoreleasepool{@try{try{
  [NSApplication sharedApplication];std::atomic<bool> available{false};bool focus=true;unsigned resets=0,reads=0;
  ControllerSession session({[&]{return available.load();},[]{return 1U;},[&](auto){++resets;return true;},[](auto,auto){return true;}});
  NSWindow* owner=[[NSWindow alloc] initWithContentRect:NSMakeRect(0,0,800,600) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable backing:NSBackingStoreBuffered defer:NO];owner.releasedWhenClosed=NO;
  SanserControllerInput* ui=[[SanserControllerInput alloc] initWithOwner:owner session:&session source:[&]{++reads;ControllerSample s;s.connected=s.sampled=1;return s;} focused:[&]{return focus;}];
  check(!ui.button.enabled,"unsupported UI enabled");[ui toggle];check(!session.active() && !resets,"unsupported action reset");
  available=true;[ui refresh];[ui toggle];check(session.active() && resets==1 && reads==1 && (session.status()&4),"native UI did not poll/arm");
  focus=false;[ui refresh];check(!session.active(),"focus did not cancel");focus=true;[ui refresh];check(!session.active(),"focus auto-rearmed");
  [ui toggle];available=false;[ui refresh];check(!session.active() && !ui.button.enabled,"capability loss retained capture");
  available=true;[ui refresh];[ui toggle];[ui stop];check(!session.active(),"reset did not stop");
  [ui detach];[ui toggle];check(!session.active() && !ui.button.enabled && !ui.button.target,"detached action reopened");[owner close];
  std::cout<<"AppKit analog: native controls, fake sample source, host/focus gates, reset and detach passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}@catch(NSException* e){std::cerr<<e.name.UTF8String<<'\n';return 1;}}}
