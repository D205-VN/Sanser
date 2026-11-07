#pragma once
#include <algorithm>
#include <bitset>
#include <chrono>
#include <cstdint>
#include <map>
#include <optional>
#include <vector>

namespace sanser::desktop {
// Small authenticated application messages, independent of ordered input and
// video repair. Peer serializes calls. Actual wire pacing belongs to TxScheduler.
class UserMessages {
public:
  using Bytes=std::vector<std::uint8_t>;
  using Clock=std::chrono::steady_clock;
  static constexpr std::size_t chunk=900,limit=32768,window=4,header=22;
  struct Received { std::optional<Bytes> ack,body; };
  bool queue(Bytes body,Clock::time_point now=Clock::now()) {
    if(body.empty() || body.size()>limit || outgoing_.size()>=4 || serial_==UINT64_MAX) return false;
    const auto count=(body.size()+chunk-1)/chunk;
    outgoing_.emplace(++serial_,Outgoing{std::move(body),std::vector<bool>(count),std::vector<Clock::time_point>(count),now});
    return true;
  }
  std::vector<Bytes> poll(Clock::time_point now,std::chrono::milliseconds retry) {
    std::vector<Bytes> packets;
    for(auto it=outgoing_.begin();it!=outgoing_.end();) {
      auto& value=it->second;
      if(now-value.created>std::chrono::seconds(10)) { it=outgoing_.erase(it); continue; }
      const auto first=std::find(value.acked.begin(),value.acked.end(),false)-value.acked.begin();
      for(auto index=static_cast<std::size_t>(first);index<std::min(value.acked.size(),first+window) && packets.size()<4;++index) {
        if(!value.acked[index] && (value.sent[index]==Clock::time_point{} || now-value.sent[index]>=retry)) {
          auto packet=prefix(1,it->first,index,value.acked.size(),value.body.size());
          const auto start=index*chunk,end=std::min(start+chunk,value.body.size());
          packet.insert(packet.end(),value.body.begin()+start,value.body.begin()+end);
          packets.push_back(std::move(packet)); value.sent[index]=now;
        }
      }
      ++it;
    }
    for(auto it=incoming_.begin();it!=incoming_.end();) {
      if(now-it->second.created>std::chrono::seconds(30)) it=incoming_.erase(it); else ++it;
    }
    return packets;
  }
  Received receive(const Bytes& packet,Clock::time_point now=Clock::now()) {
    if(packet.size()<header || !std::equal(packet.begin(),packet.begin()+4,"SUD1") || packet[5] || (packet[4]!=1 && packet[4]!=2)) return {};
    const auto id=read(packet,6,8),index=read(packet,14,2),count=read(packet,16,2),size=read(packet,18,4);
    if(!id || !size || size>limit || count!=(size+chunk-1)/chunk || index>=count) return {};
    if(packet[4]==2) {
      if(packet.size()!=header) return {};
      const auto found=outgoing_.find(id);
      if(found!=outgoing_.end() && found->second.body.size()==size && found->second.acked.size()==count) {
        found->second.acked[index]=true;
        if(std::all_of(found->second.acked.begin(),found->second.acked.end(),[](bool value) { return value; })) outgoing_.erase(found);
      }
      return {};
    }
    if(packet.size()!=header+std::min<std::uint64_t>(chunk,size-index*chunk)) return {};
    auto ack=prefix(2,id,index,count,size);
    // Old delivered IDs remain ineligible after the bounded history rolls off.
    if(id<=newest_ && (newest_-id>=delivered_.size() || delivered_[newest_-id])) return {std::move(ack),{}};
    if(!incoming_.contains(id)) {
      if(incoming_.size()>=4) return {};
      incoming_.emplace(id,Incoming{Bytes(size),std::vector<bool>(count),now});
    }
    auto& value=incoming_.at(id);
    if(value.body.size()!=size || value.seen.size()!=count) return {};
    if(!value.seen[index]) {
      std::copy(packet.begin()+header,packet.end(),value.body.begin()+index*chunk); value.seen[index]=true;
    }
    if(!std::all_of(value.seen.begin(),value.seen.end(),[](bool item) { return item; })) return {std::move(ack),{}};
    if(id>newest_) { const auto shift=id-newest_; if(shift>=delivered_.size()) delivered_.reset(); else delivered_<<=shift; newest_=id; }
    delivered_.set(newest_-id);
    Bytes body=std::move(value.body); incoming_.erase(id);
    return {std::move(ack),std::move(body)};
  }
  static std::uint64_t key(const Bytes& packet) { return packet.size()>=header ? read(packet,6,8):0; }
  Clock::time_point expires(const Bytes& packet) const {
    const auto found=outgoing_.find(key(packet));
    return found==outgoing_.end() ? Clock::now()+std::chrono::seconds(10):found->second.created+std::chrono::seconds(10);
  }
  static std::uint16_t fragment(const Bytes& packet) {
    return packet.size()>=header ? static_cast<std::uint16_t>(read(packet,14,2)*2+(packet[4]==2)):0;
  }
private:
  static std::uint64_t read(const Bytes& bytes,std::size_t offset,std::size_t count) {
    std::uint64_t value=0;for(std::size_t i=offset;i<offset+count;++i)value=(value<<8)|bytes[i];return value;
  }
  static Bytes prefix(unsigned operation,std::uint64_t id,std::size_t index,std::size_t count,std::size_t size) {
    Bytes result{'S','U','D','1',static_cast<std::uint8_t>(operation),0};
    const auto append=[&](std::uint64_t value,unsigned bytes) { for(int i=bytes-1;i>=0;--i)result.push_back(static_cast<std::uint8_t>(value>>(i*8))); };
    append(id,8);append(index,2);append(count,2);append(size,4);return result;
  }
  struct Outgoing { Bytes body;std::vector<bool> acked;std::vector<Clock::time_point> sent;Clock::time_point created; };
  struct Incoming { Bytes body;std::vector<bool> seen;Clock::time_point created; };
  std::map<std::uint64_t,Outgoing> outgoing_;
  std::map<std::uint64_t,Incoming> incoming_;
  std::uint64_t serial_=0,newest_=0;
  std::bitset<128> delivered_;
};
}
