#include "blocked_apps_policy.h"
#include <iostream>
#include <stdexcept>

using namespace sanser::blocked;
namespace {
void check(bool ok,const char* why) { if(!ok) throw std::runtime_error(why); }
bool throws(auto&& f) { try { f(); } catch(const std::exception&) { return true; } return false; }
}

int main() {
  try {
    {
      List list;
      list.add("com.google.Chrome"); list.add(" chrome.exe "); list.add("COM.GOOGLE.CHROME");
      check(list.size()==2,"duplicates differing only in case are merged");
      check(list.matches("com.google.chrome") && list.matches("Chrome.EXE"),"matching ignores case");
      check(!list.matches("com.google.chrome.helper") && !list.matches("") && !list.matches("chrome"),"matching is exact");
      check(list.matchesPath("C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"),"a Windows image path matches by file name");
      check(list.matchesPath("/usr/bin/chrome.exe") && !list.matchesPath("C:\\chrome.exe\\other.exe"),"only the last path segment counts");
      for(const char* bad:{"","   ","a/b","a\\b","C:x","a*","a?","a\"b","a<b","a>b","a|b","a\nb"})
        check(throws([&] { List l; l.add(bad); }),"invalid identifiers are rejected");
      check(throws([&] { List l; l.add(std::string(256,'a')); }),"identifiers longer than 255 are rejected");
      List full; for(int i=0;i<64;++i) full.add("app"+std::to_string(i)+".exe");
      check(throws([&] { full.add("one-more.exe"); }),"at most 64 apps");
    }
    {
      InputGate gate;
      const auto key=[](std::uint16_t usage,bool down) { return hidKey(usage,down); };
      check(gate.allowKey(key(4,true),false),"keys pass when no blocked app is in front");
      check(!gate.allowKey(key(4,true),true),"a key press is dropped while a blocked app is in front");
      check(gate.allowKey(key(4,false),true),"a key release always passes");
      check(gate.allowKey(key(225,true),true) && gate.allowKey(key(225,false),true),"modifiers pass");
      check(!gate.allowKey(key(43,true),true),"Tab alone is dropped in a blocked app");
      check(gate.allowKey(key(227,true),true),"Command passes");
      check(gate.allowKey(key(43,true),true),"Command+Tab switches away from a blocked app");
      check(!gate.allowKey(key(6,true),true),"Command+C cannot copy from a blocked app");
      check(gate.allowKey(key(227,false),true),"Command release passes");
      check(!gate.allowKey(key(43,true),true),"Tab is dropped again after Command is released");
      check(gate.allowKey(key(226,true),true) && gate.allowKey(key(43,true),true),"Alt+Tab switches away on Windows");
      gate.allowKey(key(226,false),true);
      check(!gate.allowKey(key(43,true),true),"Alt release ends the switch chord");
      check(gate.allowPointer(false,true),"pointer moves and releases always pass");
      check(!gate.allowPointer(true,true),"a click or scroll over a blocked window is dropped");
      check(gate.allowPointer(true,false),"clicks elsewhere pass, so the viewer can switch apps");
      check(gate.dropped()==6,"dropped events are counted");
      gate.allowKey(key(227,true),false); gate.reset();
      check(!gate.allowKey(key(43,true),true),"reset forgets held switch modifiers");
    }
    {
      const auto local=toOutput({{-50,-50,100,100},{1900,1000,2100,1200},{3000,0,3100,10},{10,10,10,20}},0,0,1920,1080);
      check(local.size()==2,"rectangles outside the output or empty are dropped");
      check(local[0].left==0 && local[0].top==0 && local[0].right==100 && local[0].bottom==100,"rectangles are clipped at the top-left");
      check(local[1].right==1920 && local[1].bottom==1080,"rectangles are clipped at the bottom-right");
      const auto second=toOutput({{1920,0,2000,50}},1920,0,1280,720);
      check(second.size()==1 && second[0].left==0 && second[0].right==80,"coordinates become output-local on a second monitor");
      const auto negative=toOutput({{-1280,100,-1200,200}},-1280,0,1280,1024);
      check(negative.size()==1 && negative[0].left==0 && negative[0].right==80,"monitors left of the primary use negative origins");
    }
    {
      std::vector<std::uint8_t> pixels(8*4*4,0x7f);
      paintBlack(pixels.data(),8*4,4,4,{{1,1,3,3},{-5,-5,0,0},{3,3,9,9}});
      auto at=[&](int x,int y) { return pixels.data()+y*8*4+x*4; };
      check(at(1,1)[0]==0 && at(2,2)[2]==0 && at(1,1)[3]==255,"masked pixels are opaque black");
      check(at(0,0)[0]==0x7f && at(3,1)[0]==0x7f,"pixels outside rectangles are untouched");
      check(at(3,3)[0]==0,"clipped rectangles still paint inside the frame");
      check(pixels[4*4]==0x7f,"stride padding beyond the width is untouched");
    }
    std::cout<<"Blocked apps: identifiers, input gate, Cmd/Alt+Tab escape, output rectangles and masking passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
