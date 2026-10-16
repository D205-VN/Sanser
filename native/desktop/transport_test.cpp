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
    TxScheduler scheduler;
    const auto now=Clock::now();
    require(videoSendBudget(256*1024,20000000,true)>std::chrono::milliseconds(100),"large IDR budget includes its own serialization");
    require(videoSendBudget(8*1024*1024,1000000,true)==std::chrono::milliseconds(500),"pacing cannot create a multi-second video queue");
    require(scheduler.push(TxScheduler::Video, {Bytes(1024, 1), now, 1}), "queue video");
    require(scheduler.push(TxScheduler::Audio, {{2}, now, 0}), "queue audio separately");
    scheduler.latestInput({{3}, now, 0}); scheduler.latestInput({{4}, now, 0});
    require(scheduler.push(TxScheduler::InputLane, {{5}, now, 0}), "queue reliable input");
    require(scheduler.push(TxScheduler::Control, {{6}, now, 0}), "queue control");
    for (const auto expected : {6,5,4,2}) {
      auto next=scheduler.pop(false);
      require(next && next->bytes.front()==expected, "pacing video must not hold priority lanes");
    }
    require(!scheduler.pop(false), "video pacing remains respected");
    require(scheduler.push(TxScheduler::Repair, {{8},now,1}), "queue repair in its own lane");
    require(scheduler.pop(true)->frame==1, "video resumes after priority traffic");
    auto repair=scheduler.pop(true);
    require(repair && repair->repair && repair->bytes.front()==8,"repair follows realtime video");
    require(!scheduler.push(TxScheduler::Video, {Bytes(6*1024*1024),now,2}), "video lane memory bounded");
    require(scheduler.push(TxScheduler::InputLane, {{7},now,0}), "video overflow cannot consume key-up capacity");
    const std::string token(48,'a'); PacketCodec host(token,true),client(token,false),wrong(std::string(48,'b'),false);
    snv2::Header h; h.sequence=1; h.keyId=42;
    // Control packets have no plaintext; also exercise PKCS#7 block boundaries
    // in both directions on CommonCrypto (Mac) and BCrypt (Windows).
    for (const std::size_t length : {0U, 1U, 15U, 16U, 17U, 31U, 32U, 1000U}) {
      Bytes payload(length, 0x5a);
      try {
        for (const bool fromHost : {true, false}) {
          auto wire = (fromHost ? host : client).seal(h, payload);
          auto decoded = snv2::decodePacket(wire);
          require(decoded && decoded.payload.size() == 16 + (length / 16 + 1) * 16,
                  "IV and PKCS#7 ciphertext size");
          auto result = (fromHost ? client : host).open(wire);
          require(result && result->second == payload, "AES block boundary round trip");
          ++h.sequence;
        }
      } catch (const std::exception& error) {
        throw std::runtime_error("AES payload length " + std::to_string(length) + ": " + error.what());
      }
    }
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
    // Reusing algorithm providers must never reuse mutable hash/key/IV state.
    // Exercise simultaneous sessions and key changes on the same worker thread.
    std::atomic<unsigned> cryptoFailures{0}; std::vector<std::thread> cryptoWorkers;
    const auto cryptoBegan=Clock::now();
    for(unsigned worker=0;worker<4;++worker) cryptoWorkers.emplace_back([worker,&cryptoFailures] {
      try {
        PacketCodec hosts[]{PacketCodec(std::string(48,static_cast<char>('c'+worker)),true),PacketCodec(std::string(48,static_cast<char>('k'+worker)),true)};
        PacketCodec clients[]{PacketCodec(std::string(48,static_cast<char>('c'+worker)),false),PacketCodec(std::string(48,static_cast<char>('k'+worker)),false)};
        for(unsigned i=0;i<2048;++i) {
          snv2::Header header; header.sequence=i+1; header.keyId=7;
          const unsigned session=i%2;
          Bytes plain(i%17==0 ? 0 : 1000,static_cast<std::uint8_t>(i));
          const auto wire=hosts[session].seal(header,plain);
          const auto opened=clients[session].open(wire);
          if(!opened || opened->second!=plain || clients[1-session].open(wire)) ++cryptoFailures;
        }
      } catch(...) { ++cryptoFailures; }
    });
    for(auto& thread:cryptoWorkers) thread.join();
    require(cryptoFailures==0,"cached crypto provider mixed concurrent sessions, keys or message state");
    std::cout<<"Crypto stress: 8192 authenticated media/control round trips across four threads in "
      <<std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now()-cryptoBegan).count()<<" ms\n";
    ReplayWindow replay; require(!replay.accept(0),"zero sequence"); require(replay.accept(10),"new sequence"); require(replay.accept(9),"reordering"); require(!replay.accept(9),"duplicate"); require(replay.accept(3000),"window jump"); require(!replay.accept(10),"old sequence"); require(replay.accept(2999),"within new window");
    Input key{Input::Key,0,0,4,0,true}; auto decoded=decodeInput(encodeInput(key)); require(decoded && decoded->code==4 && decoded->down,"key round trip");
    require(!decodeInput(Bytes(9)),"truncated input"); key.code=0; require(!decodeInput(encodeInput(key)),"invalid HID key"); key.kind=Input::Button; key.code=3; require(!decodeInput(encodeInput(key)),"invalid mouse button");
    Input scroll{Input::Scroll,100,200,0,-120,false}; decoded=decodeInput(encodeInput(scroll)); require(decoded && decoded->delta==-120,"signed wheel");
    require(hidFromMac(macKey(4))==4 && hidFromWindows(windowsKey(4))==4,"keyboard mapping round trip");
    Bytes frame{0,0,7,128,0,0,4,56}; frame.resize(2508,77); h.frameId=1; h.flags=snv2::kFlagKeyFrame;
    Reassembler frames; require(!frames.push(h,fragment(frame,2)),"partial frame"); require(!frames.push(h,fragment(frame,0)),"out of order partial"); require(!frames.push(h,fragment(frame,0)),"duplicate fragment"); auto ready=frames.push(h,fragment(frame,1)); require(ready && ready->width==1920 && ready->height==1080 && ready->keyframe && ready->data==Bytes(2500,77),"reassembled frame");
    require(!frames.push(h,fragment(frame,0)),"completed frame cannot replay"); h.frameId=2; auto malformed=fragment(frame,0); malformed[2]=255; require(!frames.push(h,malformed),"invalid fragment count");
    require(!frames.push(h,fragment(frame,0)) && !frames.push(h,fragment(frame,2)),"missing fragment remains incomplete");
    bool expired=false;
    const auto requests=frames.repairs(expired,Clock::now()+std::chrono::milliseconds(4));
    require(!expired && requests.size()==1 && requests.front().frame==2 && requests.front().missing==std::vector<std::uint16_t>{1},"NACK names only the missing fragment");
    require(frames.push(h,fragment(frame,1)).has_value(),"repaired fragment completes the existing assembly");
    {
      Bytes large{0,0,7,128,0,0,4,56}; large.resize(256*1024,0x44);
      Reassembler paced(20000000,true); h.frameId=3;
      require(!paced.push(h,fragment(large,0)),"large IDR starts incomplete");
      const auto began=Clock::now();
      require(paced.repairs(expired,began+std::chrono::milliseconds(35)).empty() && !expired,"do not NACK the tail still being paced by the sender");
      require(!paced.push(h,fragment(large,2)),"interior gap before later fragment");
      const auto gap=paced.repairs(expired,began+std::chrono::milliseconds(36));
      require(gap.size()==1 && gap.front().missing==std::vector<std::uint16_t>{1},"repair only the actual interior loss, not the unsent tail");
      (void)paced.repairs(expired,began+std::chrono::milliseconds(220));
      require(!expired,"a normal WAN repair RTT must not expire the entire keyframe");
      (void)paced.repairs(expired,began+std::chrono::seconds(1));
      require(expired,"unrepairable pictures retain a bounded deadline");
    }
    std::cout<<"Transport: encryption, tamper, replay, input and reassembly passed\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
