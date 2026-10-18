#include "opus_audio.h"
#include <opus.h>
#include <algorithm>
#include <cmath>
#include <stdexcept>

namespace sanser::audio {
struct Encoder::State { OpusEncoder* value=nullptr; ~State() { if(value) opus_encoder_destroy(value); } };
struct Decoder::State { OpusDecoder* value=nullptr; ~State() { if(value) opus_decoder_destroy(value); } };
Encoder::Encoder():state_(std::make_unique<State>()) {
  int error=0; state_->value=opus_encoder_create(rate,channels,OPUS_APPLICATION_RESTRICTED_LOWDELAY,&error);
  if(error!=OPUS_OK || !state_->value) throw std::runtime_error("Create Opus audio encoder failed");
  if(opus_encoder_ctl(state_->value,OPUS_SET_BITRATE(96000))!=OPUS_OK ||
     opus_encoder_ctl(state_->value,OPUS_SET_VBR(0))!=OPUS_OK ||
     opus_encoder_ctl(state_->value,OPUS_SET_COMPLEXITY(5))!=OPUS_OK)
    throw std::runtime_error("Configure Opus audio encoder failed");
}
Encoder::~Encoder()=default;
void Encoder::reset() { opus_encoder_ctl(state_->value,OPUS_RESET_STATE); }
Bytes Encoder::encode(const Samples& samples) {
  Bytes out(1000); const auto size=opus_encode(state_->value,samples.data(),frames,out.data(),static_cast<opus_int32>(out.size()));
  if(size<=0) throw std::runtime_error("Encode Opus audio failed");
  out.resize(static_cast<std::size_t>(size)); return out;
}
Decoder::Decoder():state_(std::make_unique<State>()) {
  int error=0; state_->value=opus_decoder_create(rate,channels,&error);
  if(error!=OPUS_OK || !state_->value) throw std::runtime_error("Create Opus audio decoder failed");
}
Decoder::~Decoder()=default;
void Decoder::reset() { opus_decoder_ctl(state_->value,OPUS_RESET_STATE); }
std::optional<Samples> Decoder::decode(std::span<const std::uint8_t> bytes) {
  if(bytes.empty() || bytes.size()>1000 ||
     opus_packet_get_nb_samples(bytes.data(),static_cast<opus_int32>(bytes.size()),rate)!=frames) return {};
  Samples samples; if(opus_decode(state_->value,bytes.data(),static_cast<opus_int32>(bytes.size()),samples.data(),frames,0)!=frames) return {};
  return samples;
}
Samples Decoder::conceal() {
  Samples samples{}; if(opus_decode(state_->value,nullptr,0,samples.data(),frames,0)!=frames) samples.fill(0); return samples;
}
Bytes pack(const Packet& packet) {
  Bytes bytes{'S','O','A','1'};
  for(auto number:{packet.sequence,packet.timestamp}) for(int i=7;i>=0;--i) bytes.push_back(static_cast<std::uint8_t>(number>>(i*8)));
  bytes.insert(bytes.end(),packet.payload.begin(),packet.payload.end()); return bytes;
}
std::optional<Packet> unpack(std::span<const std::uint8_t> bytes) {
  if(bytes.size()<=20 || bytes.size()>1020 || !std::equal(bytes.begin(),bytes.begin()+4,"SOA1")) return {};
  Packet packet;
  for(std::size_t i=4;i<12;++i) packet.sequence=(packet.sequence<<8)|bytes[i];
  for(std::size_t i=12;i<20;++i) packet.timestamp=(packet.timestamp<<8)|bytes[i];
  if(!packet.sequence || packet.sequence>UINT64_MAX/frames || packet.timestamp!=(packet.sequence-1)*frames) return {};
  packet.payload.assign(bytes.begin()+20,bytes.end()); return packet;
}
bool Jitter::push(Packet packet,Clock::time_point now) {
  if(!packet.sequence || packet.sequence>UINT64_MAX/frames || packet.timestamp!=(packet.sequence-1)*frames ||
     packet.payload.empty() || packet.payload.size()>1000 ||
     opus_packet_get_nb_samples(packet.payload.data(),static_cast<opus_int32>(packet.payload.size()),rate)!=frames) { ++stats_.errors; return false; }
  if(packet.sequence<=lastSequence_ || (started_ && packet.sequence<expected_)) { ++stats_.late; return false; }
  if(packets_.contains(packet.sequence)) return false;
  if(!packets_.empty() && packet.sequence<packets_.rbegin()->first) ++stats_.reordered;
  if(packets_.empty() && !started_) first_=now;
  packets_.emplace(packet.sequence,Entry{std::move(packet),now});
  if(packets_.size()>12) { stats_.dropped+=packets_.size()-3; while(packets_.size()>3) packets_.erase(packets_.begin()); started_=false; decoder_.reset(); first_=now; }
  return true;
}
std::optional<Decoded> Jitter::poll(Clock::time_point now) {
  while(!packets_.empty() && now-packets_.begin()->second.received>std::chrono::milliseconds(40)) { packets_.erase(packets_.begin()); ++stats_.dropped; }
  if(started_ && now-due_>std::chrono::milliseconds(40)) { started_=false; decoder_.reset(); first_=now; }
  if(!started_) {
    if(packets_.empty() || now-first_<std::chrono::milliseconds(10)) return {};
    expected_=packets_.begin()->first; due_=now; missing_=0; started_=true;
  }
  if(now<due_) return {};
  Decoded out{}; out.sequence=expected_;
  const auto it=packets_.find(expected_);
  if(it!=packets_.end()) {
    auto samples=decoder_.decode(it->second.packet.payload);
    out.unixMicros=it->second.packet.unixMicros;
    if(samples) { out.samples=*samples; ++stats_.decoded; missing_=0; }
    else { out.samples=decoder_.conceal(); out.concealed=true; ++stats_.errors; ++stats_.concealed; ++missing_; }
    packets_.erase(it);
  } else {
    if(missing_>=6) { started_=false; decoder_.reset(); first_=now; return {}; }
    out.samples=decoder_.conceal(); out.concealed=true; ++stats_.concealed; ++missing_;
    if(lastUnix_ && expected_>lastSequence_) out.unixMicros=lastUnix_+(expected_-lastSequence_)*5000;
  }
  lastSequence_=expected_; lastUnix_=out.unixMicros; ++expected_; due_+=std::chrono::milliseconds(5); return out;
}
void Jitter::reset() { packets_.clear(); decoder_.reset(); started_=false; expected_=lastSequence_=lastUnix_=0; missing_=0; first_=due_={}; }
ReceiveWorker::ReceiveWorker(Sink sink):sink_(std::move(sink)),worker_([this] { run(); }) {}
ReceiveWorker::~ReceiveWorker() { { std::lock_guard lock(mutex_); stopped_=true; } wake_.notify_all(); if(worker_.joinable()) worker_.join(); }
bool ReceiveWorker::push(Packet packet,std::uint64_t generation) {
  std::lock_guard lock(mutex_);
  if(stopped_ || generation!=generation_) return false;
  const bool accepted=jitter_.push(std::move(packet)); wake_.notify_one(); return accepted;
}
void ReceiveWorker::reset(std::uint64_t generation) { std::lock_guard lock(mutex_); generation_=generation; jitter_.reset(); }
Stats ReceiveWorker::stats() const { std::lock_guard lock(mutex_); return jitter_.stats(); }
void ReceiveWorker::run() {
  std::unique_lock lock(mutex_);
  while(!stopped_) {
    auto decoded=jitter_.poll();
    if(decoded) {
      decoded->generation=generation_;
      // Sink is a bounded PCM enqueue, never an OS playback call. Serialize it
      // with reset so an old epoch cannot enqueue after reset returns.
      try { sink_(std::move(*decoded)); } catch(...) { stopped_=true; }
    } else if(jitter_.active()) wake_.wait_for(lock,std::chrono::milliseconds(1));
    else wake_.wait(lock,[this] { return stopped_ || jitter_.active(); });
  }
}
void Packetizer::reset() { used_=inputRate_=0; hasPrevious_=false; untilNext_=0; }
void Packetizer::sample(float left,float right) {
  for(float value:{left,right}) {
    if(!std::isfinite(value)) value=0;
    samples_[used_++]=static_cast<std::int16_t>(std::clamp(value,-1.0F,1.0F)*32767);
  }
  if(used_==samples_.size()) { sink_(samples_); used_=0; }
}
bool Packetizer::push(std::span<const float> stereo,unsigned inputRate) {
  if(inputRate<8000 || inputRate>384000 || stereo.size()%2) return false;
  if(inputRate_!=inputRate) { reset(); inputRate_=inputRate; }
  const double step=static_cast<double>(inputRate)/rate;
  for(std::size_t i=0;i<stereo.size();i+=2) {
    const std::array<float,2> current{stereo[i],stereo[i+1]};
    if(!hasPrevious_) { previous_=current; hasPrevious_=true; sample(current[0],current[1]); untilNext_=step; continue; }
    while(untilNext_<=1.0+1e-9) {
      const auto mix=static_cast<float>(std::clamp(untilNext_,0.0,1.0));
      sample(previous_[0]+(current[0]-previous_[0])*mix,previous_[1]+(current[1]-previous_[1])*mix); untilNext_+=step;
    }
    untilNext_-=1; previous_=current;
  }
  return true;
}
}
