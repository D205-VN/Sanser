#include "transport.h"
#include "keyboard.h"
#include <algorithm>
#include <iostream>
#include <stdexcept>
using namespace sanser;
using namespace sanser::desktop;
void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
Bytes fragment(const Bytes& bytes,unsigned index) {
  const auto count=(bytes.size()+999)/1000;
  Bytes out{static_cast<uint8_t>(index>>8),static_cast<uint8_t>(index),static_cast<uint8_t>(count>>8),static_cast<uint8_t>(count)};
  for(int i=3;i>=0;--i) out.push_back(static_cast<uint8_t>(bytes.size()>>(8*i)));
  out.insert(out.end(),bytes.begin()+index*1000,bytes.begin()+std::min<std::size_t>(bytes.size(),(index+1)*1000UL)); return out;
}
int main() {
  try {
    const std::string token(48,'a'); PacketCodec host(token,true),client(token,false),wrong(std::string(48,'b'),false);
    snv2::Header h; h.sequence=1; h.keyId=42;
    const Bytes secret{1,2,3,4,5,6,7,8,9,10};
    auto packet=host.seal(h,secret); auto opened=client.open(packet);
    require(opened && opened->second==secret,"encrypted payload round trip");
    require(std::search(packet.begin(),packet.end(),secret.begin(),secret.end())==packet.end(),"plaintext must not appear on wire");
    require(!host.open(packet) && !wrong.open(packet),"reject reflection and wrong session");
    for(size_t i=0;i<packet.size();++i) { auto damaged=packet; damaged[i]^=1; require(!client.open(damaged),"reject tampered packet"); }
    auto truncated=packet; truncated.pop_back(); require(!client.open(truncated),"reject truncation");
    h.keyId++; require(packet!=host.seal(h,secret),"restart changes IV");
    h.sequence++; require(packet!=host.seal(h,secret),"sequence changes IV");
    require(host.open(client.seal(h,{})).has_value(),"reverse direction empty keepalive");
    ReplayWindow replay; require(!replay.accept(0),"zero sequence"); require(replay.accept(10),"new sequence"); require(replay.accept(9),"reordering"); require(!replay.accept(9),"duplicate"); require(replay.accept(3000),"window jump"); require(!replay.accept(10),"old sequence"); require(replay.accept(2999),"within new window");
    Input key{Input::Key,0,0,4,0,true}; auto decoded=decodeInput(encodeInput(key)); require(decoded && decoded->code==4 && decoded->down,"key round trip");
    require(!decodeInput(Bytes(9)),"truncated input"); key.code=0; require(!decodeInput(encodeInput(key)),"invalid HID key"); key.kind=Input::Button; key.code=3; require(!decodeInput(encodeInput(key)),"invalid mouse button");
    Input scroll{Input::Scroll,100,200,0,-120,false}; decoded=decodeInput(encodeInput(scroll)); require(decoded && decoded->delta==-120,"signed wheel");
    require(hidFromMac(macKey(4))==4 && hidFromWindows(windowsKey(4))==4,"keyboard mapping round trip");
    Bytes frame{0,0,7,128,0,0,4,56}; frame.resize(2508,77); h.frameId=1; h.flags=snv2::kFlagKeyFrame;
    Reassembler frames; require(!frames.push(h,fragment(frame,2)),"partial frame"); require(!frames.push(h,fragment(frame,0)),"out of order partial"); require(!frames.push(h,fragment(frame,0)),"duplicate fragment"); auto ready=frames.push(h,fragment(frame,1)); require(ready && ready->width==1920 && ready->height==1080 && ready->keyframe && ready->data==Bytes(2500,77),"reassembled frame");
    require(!frames.push(h,fragment(frame,0)),"completed frame cannot replay"); h.frameId=2; auto malformed=fragment(frame,0); malformed[2]=255; require(!frames.push(h,malformed),"invalid fragment count");
    std::cout<<"Transport: encryption, tamper, replay, input and reassembly passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
