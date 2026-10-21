#include "voice.h"
#include <opus.h>
#include <algorithm>
#include <cmath>
#include <cstring>
#include <random>
#include <stdexcept>

namespace sanser::voice {
namespace {
std::uint32_t epoch() {
  std::random_device random; std::uint32_t id=0;
  while(!id) id=static_cast<std::uint32_t>(random());
  return id;
}
bool validSequence(std::uint64_t sequence) { return sequence && sequence<=UINT64_MAX/packetFrames; }
bool validOpus(std::span<const std::uint8_t> payload) {
  return !payload.empty() && payload.size()<=400
    && opus_packet_get_nb_samples(payload.data(),static_cast<opus_int32>(payload.size()),48000)==packetFrames;
}
}
std::optional<State> command(const std::string& line) {
  constexpr std::string_view prefix="voice-config\t";
  if(line.size()!=prefix.size()+5 || line.compare(0,prefix.size(),prefix)) return {};
  const auto p=prefix.size();
  if((line[p]!='0' && line[p]!='1') || line[p+1]!='\t' || (line[p+2]!='0' && line[p+2]!='1')
      || line[p+3]!='\t' || (line[p+4]!='0' && line[p+4]!='1')) return {};
  State state{line[p]=='1',line[p+2]=='1',line[p+4]=='1'};
  if(!state.joined) state.microphone=state.deafened=false;
  if(state.deafened) state.microphone=false;
  return state;
}
bool isPacket(std::span<const std::uint8_t> bytes) { return bytes.size()>=4 && !std::memcmp(bytes.data(),"SVO1",4); }
Bytes pack(const Packet& packet) {
  if(packet.speaker>=maxSpeakers || !packet.epoch || !validSequence(packet.sequence) || !validOpus(packet.opus)) throw std::runtime_error("Invalid voice packet");
  Bytes bytes{'S','V','O','1',static_cast<std::uint8_t>(packet.speaker)};
  for(int i=3;i>=0;--i) bytes.push_back(static_cast<std::uint8_t>(packet.epoch>>(i*8)));
  for(int i=7;i>=0;--i) bytes.push_back(static_cast<std::uint8_t>(packet.sequence>>(i*8)));
  bytes.insert(bytes.end(),packet.opus.begin(),packet.opus.end()); return bytes;
}
std::optional<Packet> unpack(std::span<const std::uint8_t> bytes) {
  if(bytes.size()<=17 || bytes.size()>417 || !isPacket(bytes) || bytes[4]>=maxSpeakers) return {};
  Packet packet; packet.speaker=bytes[4];
  for(unsigned i=5;i<9;++i) packet.epoch=(packet.epoch<<8)|bytes[i];
  for(unsigned i=9;i<17;++i) packet.sequence=(packet.sequence<<8)|bytes[i];
  if(!packet.epoch || !validSequence(packet.sequence) || !validOpus(bytes.subspan(17))) return {};
  packet.opus.assign(bytes.begin()+17,bytes.end()); return packet;
}
struct Encoder::Impl {
  OpusEncoder* codec=nullptr;
  std::array<opus_int16,packetFrames> pending{};
  unsigned used=0;
  std::uint32_t generation=0;
  std::uint64_t sequence=0;
  Clock::time_point previous{};
  ~Impl() { if(codec) opus_encoder_destroy(codec); }
};
Encoder::Encoder():impl_(std::make_unique<Impl>()) {
  int error=0; impl_->codec=opus_encoder_create(48000,1,OPUS_APPLICATION_VOIP,&error);
  if(error!=OPUS_OK || !impl_->codec) throw std::runtime_error("Unable to create voice Opus encoder");
  if(opus_encoder_ctl(impl_->codec,OPUS_SET_BITRATE(32000))!=OPUS_OK
      || opus_encoder_ctl(impl_->codec,OPUS_SET_VBR(1))!=OPUS_OK
      || opus_encoder_ctl(impl_->codec,OPUS_SET_VBR_CONSTRAINT(1))!=OPUS_OK
      || opus_encoder_ctl(impl_->codec,OPUS_SET_INBAND_FEC(1))!=OPUS_OK
      || opus_encoder_ctl(impl_->codec,OPUS_SET_PACKET_LOSS_PERC(10))!=OPUS_OK
      || opus_encoder_ctl(impl_->codec,OPUS_SET_DTX(1))!=OPUS_OK
      || opus_encoder_ctl(impl_->codec,OPUS_SET_COMPLEXITY(5))!=OPUS_OK)
    throw std::runtime_error("Unable to configure voice Opus encoder");
  reset();
}
Encoder::~Encoder()=default;
void Encoder::reset() { opus_encoder_ctl(impl_->codec,OPUS_RESET_STATE); impl_->used=0; impl_->sequence=0; impl_->generation=epoch(); impl_->previous={}; }
std::optional<Packet> Encoder::push(const desktop::AudioSamples& samples,Clock::time_point now) {
  if(impl_->previous!=Clock::time_point{} && now-impl_->previous>std::chrono::milliseconds(40)) reset();
  impl_->previous=now;
  for(unsigned i=0;i<desktop::audioFrames;++i)
    impl_->pending[impl_->used++]=static_cast<opus_int16>((static_cast<int>(samples[2*i])+samples[2*i+1])/2);
  if(impl_->used<packetFrames) return {};
  impl_->used=0; Bytes payload(400);
  const auto n=opus_encode(impl_->codec,impl_->pending.data(),packetFrames,payload.data(),static_cast<opus_int32>(payload.size()));
  if(n<=0) throw std::runtime_error("Unable to encode voice packet");
  payload.resize(static_cast<std::size_t>(n));
  return Packet{0,impl_->generation,++impl_->sequence,std::move(payload)};
}
struct Mixer::Impl {
  struct Source {
    OpusDecoder* decoder=nullptr;
    std::uint32_t generation=0;
    std::set<std::uint32_t> previous;
    struct Entry { Packet packet; Clock::time_point at; };
    std::map<std::uint64_t,Entry> packets;
    std::array<opus_int16,packetFrames> pcm{};
    std::uint64_t expected=0,last=0;
    unsigned chunk=4,missing=0;
    bool started=false,muted=false;
    Clock::time_point first{},due{};
    Stats stats;
    Source() { int error=0; decoder=opus_decoder_create(48000,1,&error); if(error!=OPUS_OK || !decoder) throw std::runtime_error("Unable to create voice decoder"); }
    ~Source() { if(decoder) opus_decoder_destroy(decoder); }
    void reset() { packets.clear(); pcm.fill(0); expected=last=0; chunk=4; missing=0; started=false; opus_decoder_ctl(decoder,OPUS_RESET_STATE); }
    std::optional<desktop::AudioSamples> poll(Clock::time_point now) {
      while(!packets.empty() && now-packets.begin()->second.at>std::chrono::milliseconds(80)) { packets.erase(packets.begin()); ++stats.dropped; }
      if(started && now-due>std::chrono::milliseconds(60)) { started=false; chunk=4; opus_decoder_ctl(decoder,OPUS_RESET_STATE); first=now; }
      if(!started) {
        if(packets.empty() || now-first<std::chrono::milliseconds(20)) return {};
        started=true; expected=packets.begin()->first; due=now; chunk=4; missing=0;
      }
      if(now<due) return {};
      if(chunk==4) {
        int n=0;
        const auto current=packets.find(expected);
        if(current!=packets.end()) {
          const auto& payload=current->second.packet.opus;
          n=opus_decode(decoder,payload.data(),static_cast<opus_int32>(payload.size()),pcm.data(),packetFrames,0);
          packets.erase(current); missing=0; if(n==packetFrames) ++stats.decoded;
        } else {
          if(missing>=3) { started=false; first=now; opus_decoder_ctl(decoder,OPUS_RESET_STATE); return {}; }
          const auto next=packets.find(expected+1);
          if(next!=packets.end()) {
            const auto& payload=next->second.packet.opus;
            n=opus_decode(decoder,payload.data(),static_cast<opus_int32>(payload.size()),pcm.data(),packetFrames,1);
            ++stats.fecAttempts; // Opus may conceal when no FEC is present.
          } else n=opus_decode(decoder,nullptr,0,pcm.data(),packetFrames,0);
          ++missing; ++stats.concealed;
        }
        if(n!=packetFrames) { pcm.fill(0); ++stats.invalid; }
        last=expected; ++expected; chunk=0;
      }
      desktop::AudioSamples out{};
      for(unsigned i=0;i<desktop::audioFrames;++i) out[i*2]=out[i*2+1]=pcm[chunk*desktop::audioFrames+i];
      ++chunk; due+=std::chrono::milliseconds(5); return out;
    }
  };
  std::array<Source,maxSpeakers> sources;
};
Mixer::Mixer():impl_(std::make_unique<Impl>()) {}
Mixer::~Mixer()=default;
bool Mixer::receive(Packet packet,Clock::time_point now) {
  if(packet.speaker>=maxSpeakers) return false;
  auto& source=impl_->sources[packet.speaker];
  if(!packet.epoch || !validSequence(packet.sequence) || !validOpus(packet.opus)) { ++source.stats.invalid; return false; }
  if(source.muted) return false;
  if(source.generation!=packet.epoch) {
    if(source.previous.contains(packet.epoch)) return false;
    if(source.generation) {
      if(source.previous.size()>=64) source.previous.erase(source.previous.begin());
      source.previous.insert(source.generation);
    }
    source.reset(); source.generation=packet.epoch; source.first=now;
  }
  if(packet.sequence<=source.last || source.packets.contains(packet.sequence)) { ++source.stats.late; return false; }
  if(source.packets.empty() && !source.started) source.first=now;
  source.packets.emplace(packet.sequence,Mixer::Impl::Source::Entry{std::move(packet),now});
  if(source.packets.size()>6) { while(source.packets.size()>3) { source.packets.erase(source.packets.begin()); ++source.stats.dropped; } source.started=false; source.chunk=4; source.first=now; opus_decoder_ctl(source.decoder,OPUS_RESET_STATE); }
  return true;
}
std::optional<desktop::AudioSamples> Mixer::poll(Clock::time_point now) {
  std::array<std::int32_t,desktop::audioFrames*desktop::audioChannels> mixed{};
  bool any=false;
  for(auto& source:impl_->sources) if(!source.muted) if(const auto block=source.poll(now)) {
    any=true; for(unsigned i=0;i<mixed.size();++i) mixed[i]+=(*block)[i];
  }
  if(!any) return {};
  desktop::AudioSamples out{};
  // Leave headroom and smoothly limit simultaneous loud talkers, avoiding
  // integer overflow and hard clipping while preserving ordinary speech gain.
  for(unsigned i=0;i<out.size();++i) {
    const double value=mixed[i]/32768.0;
    out[i]=static_cast<std::int16_t>(std::tanh(value)*32767);
  }
  return out;
}
void Mixer::mute(unsigned speaker,bool muted) { if(speaker<maxSpeakers) { auto& source=impl_->sources[speaker]; source.muted=muted; source.reset(); } }
std::vector<desktop::AudioSamples> Mixer::drain(Clock::time_point now) {
  std::vector<desktop::AudioSamples> blocks; blocks.reserve(8);
  while(blocks.size()<8) {
    auto samples=poll(now); if(!samples) break;
    blocks.push_back(std::move(*samples));
  }
  return blocks;
}
void Mixer::clear() { for(auto& source:impl_->sources) source.reset(); }
Stats Mixer::stats() const {
  Stats out; for(const auto& source:impl_->sources) { const auto& s=source.stats; out.decoded+=s.decoded; out.concealed+=s.concealed; out.fecAttempts+=s.fecAttempts; out.late+=s.late; out.dropped+=s.dropped; out.invalid+=s.invalid; } return out;
}
} // namespace sanser::voice
