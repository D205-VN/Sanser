#include "telemetry.h"
#include "transport.h"
#include <algorithm>
#include <array>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <stdexcept>
#include <string_view>
#include <random>
#include <cstdio>
#include <iostream>
#include <cmath>
#ifdef _WIN32
#define NOMINMAX
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <mmsystem.h>
#include <bcrypt.h>
#else
#include <arpa/inet.h>
#include <sys/socket.h>
#include <unistd.h>
#include <CommonCrypto/CommonCryptor.h>
#endif

namespace sanser::desktop {
namespace {
std::atomic<bool> stopRequested{false};
std::mutex nativeCommandMutex;
std::vector<std::string> nativeCommands;
constexpr std::size_t chunkSize = 1000, maxFrame = 4 * 1024 * 1024;
void u16(Bytes& b, std::uint16_t n) { b.push_back(static_cast<std::uint8_t>(n >> 8)); b.push_back(static_cast<std::uint8_t>(n)); }
void u32(Bytes& b, std::uint32_t n) { u16(b, static_cast<std::uint16_t>(n >> 16)); u16(b, static_cast<std::uint16_t>(n)); }
std::uint16_t r16(const std::uint8_t* p) { return static_cast<std::uint16_t>((p[0] << 8) | p[1]); }
std::uint32_t r32(const std::uint8_t* p) { return (static_cast<std::uint32_t>(r16(p)) << 16) | r16(p + 2); }
snv2::DirectionKey derive(const snv2::DirectionKey& key, std::string_view label) {
  return snv2::deriveKey(key, {reinterpret_cast<const std::uint8_t*>(label.data()), label.size()});
}
#ifdef _WIN32
struct AesAlgorithm {
  BCRYPT_ALG_HANDLE handle=nullptr;
  AesAlgorithm() {
    if(BCryptOpenAlgorithmProvider(&handle,BCRYPT_AES_ALGORITHM,nullptr,0)<0)
      throw std::runtime_error("AES unavailable");
    const auto status=BCryptSetProperty(handle,BCRYPT_CHAINING_MODE,reinterpret_cast<PUCHAR>(const_cast<wchar_t*>(BCRYPT_CHAIN_MODE_CBC)),sizeof(BCRYPT_CHAIN_MODE_CBC),0);
    if(status<0) { BCryptCloseAlgorithmProvider(handle,0); handle=nullptr; throw std::runtime_error("BCryptSetProperty(CBC) failed"); }
  }
  ~AesAlgorithm() { if(handle) BCryptCloseAlgorithmProvider(handle,0); }
};
#endif
Bytes crypt(const snv2::DirectionKey& key, const std::uint8_t* iv, const Bytes& input, bool encrypt) {
  Bytes out(input.size() + 16);
#ifdef _WIN32
  // Thousands of media datagrams must not reopen/configure the CNG provider
  // each time. Key handles and the mutable IV remain private to this call.
  thread_local AesAlgorithm algorithm;
  BCRYPT_KEY_HANDLE handle = nullptr;
  const char* stage = "BCryptGenerateSymmetricKey";
  auto status=BCryptGenerateSymmetricKey(algorithm.handle,&handle,nullptr,0,const_cast<PUCHAR>(key.data()),static_cast<ULONG>(key.size()),0);
  std::array<std::uint8_t, 16> mutableIv{}; std::copy_n(iv, 16, mutableIv.begin());
  ULONG size = 0;
  if (status >= 0) {
    const auto operation = encrypt ? BCryptEncrypt : BCryptDecrypt;
    stage = encrypt ? "BCryptEncrypt" : "BCryptDecrypt";
    // CNG rejects a null input pointer even when cbInput is zero. Empty control
    // packets still need the CBC padding block; the sentinel is not plaintext.
    std::uint8_t emptyInput = 0;
    auto* data = input.empty() ? &emptyInput : const_cast<PUCHAR>(input.data());
    status = operation(handle, data, static_cast<ULONG>(input.size()), nullptr, mutableIv.data(), 16, out.data(), static_cast<ULONG>(out.size()), &size, BCRYPT_BLOCK_PADDING);
  }
  if (handle) BCryptDestroyKey(handle);
  if (status < 0) {
    char error[128];
    std::snprintf(error, sizeof(error), "%s failed (NTSTATUS 0x%08lX)", stage, static_cast<unsigned long>(status));
    throw std::runtime_error(error);
  }
#else
  std::size_t size = 0;
  if (CCCrypt(encrypt ? kCCEncrypt : kCCDecrypt, kCCAlgorithmAES, kCCOptionPKCS7Padding,
              key.data(), key.size(), iv, input.data(), input.size(), out.data(), out.size(), &size) != kCCSuccess) throw std::runtime_error("AES operation failed");
#endif
  out.resize(size); return out;
}
#ifdef _WIN32
using Socket = SOCKET;
void closeSocket(Socket fd) { closesocket(fd); }
#else
using Socket = int;
void closeSocket(Socket fd) { close(fd); }
#endif
sockaddr_in endpoint(const std::string& value) {
  const auto separator = value.rfind(':');
  if (separator == std::string::npos) throw std::runtime_error("Expected an IPv4 address and port");
  const auto host = value.substr(0, separator);
  const auto number = value.substr(separator + 1);
  std::size_t used = 0; const auto port = std::stoul(number, &used);
  if (used != number.size() || port == 0 || port > 65535) throw std::runtime_error("Invalid peer port");
  sockaddr_in address{}; address.sin_family = AF_INET; address.sin_port = htons(static_cast<std::uint16_t>(port));
  if (inet_pton(AF_INET, host.c_str(), &address.sin_addr) != 1) throw std::runtime_error("Expected an IPv4 peer");
  return address;
}
}
Options parseOptions(bool host, int argc, char** argv) {
  Options o; o.host = host;
  if (std::getenv("SANSER_CONTROL_STDIN")) {
    std::thread([] {
      std::string line; bool oversized=false; int c;
      while((c=std::fgetc(stdin))!=EOF) {
        if(c!='\n') { if(line.size()<1024) line.push_back(static_cast<char>(c)); else oversized=true; continue; }
        if(!oversized && line=="stop") break;
        if(!oversized) {
          std::lock_guard lock(nativeCommandMutex);
          if(nativeCommands.size()>=32) break;
          nativeCommands.push_back(std::move(line));
        }
        line.clear(); oversized=false;
      }
      stopRequested=true;
    }).detach();
  }
  const char* secret = std::getenv("SANSER_NATIVE_SESSION_TOKEN");
  if (secret) o.token = secret;
  if (o.token.size() < 32 || o.token.size() > 256) throw std::runtime_error("An authenticated session credential is required");
  for (int i = 1; i < argc; ++i) {
    const std::string arg = argv[i];
    auto value = [&]() -> std::string { if (++i >= argc) throw std::runtime_error("Missing engine argument"); return argv[i]; };
    auto number = [&]() -> std::uint32_t { const auto v = value(); std::size_t used = 0; const auto n = std::stoul(v, &used); if (used != v.size() || n > std::numeric_limits<std::uint32_t>::max()) throw std::runtime_error("Invalid numeric engine argument"); return static_cast<std::uint32_t>(n); };
    if (arg == "--snv2" || arg == "--udp-video" || arg == "--low-latency-encoder" || arg == "--udp-pacing") continue;
    if (arg == "--ultra-low-latency") o.ultraLowLatency = true;
    else if (arg == "--share-host") o.sharing = true;
    else if (arg == "--share-peer-id") o.viewerId = value();
    else if (arg == "--disable-input") o.input = false;
    else if (arg == "--disable-audio") o.audio = false;
    else if (arg == "--relative-mouse") throw std::runtime_error("SNV2 currently supports absolute mouse mode; select Auto or Absolute");
    else if (arg == "--encode-pipe") { if (value() != "h264") throw std::runtime_error("Cross-platform sessions require H.264"); }
    else if (arg == "--control-port" || arg == "--audio-port") { if (number() != 0) throw std::runtime_error("SNV2 uses one authenticated socket"); }
    else if (arg == "--udp-connect") o.peer = value();
    else if (arg == "--udp-bind-port" || arg == "--listen-render-snv") { const auto n = number(); if (n < 1 || n > 65535) throw std::runtime_error("Invalid local port"); o.port = static_cast<std::uint16_t>(n); }
    else if (arg == "--fps") o.fps = number();
    else if (arg == "--bitrate") o.bitrate = number();
    else if (arg == "--stream-width") o.width = number();
    else if (arg == "--stream-height") o.height = number();
    else throw std::runtime_error("Unsupported engine argument: " + arg);
  }
  if (o.port == 0 || o.peer.empty() || o.fps < 1 || o.fps > 120 || o.width < 64 || o.height < 64 || o.width > 3840 || o.height > 2160 || o.bitrate < 1000000 || o.bitrate > 200000000) throw std::runtime_error("Invalid native session configuration");
  if(o.sharing && (!host || o.input || o.viewerId.empty())) throw std::runtime_error("Sharing requires a host, viewer identifier and disabled input");
  if(!o.sharing && !o.viewerId.empty()) throw std::runtime_error("Viewer identifier requires sharing mode");
  if(o.sharing) o.audio=false; // Group audio is not negotiated yet.
  o.width &= ~1U; o.height &= ~1U;
  return o;
}
std::vector<std::string> takeNativeCommands() {
  std::lock_guard lock(nativeCommandMutex); std::vector<std::string> result;
  result.swap(nativeCommands); return result;
}
bool nativeStopRequested() { return stopRequested.load(); }
Bytes encodeInput(const Input& input) {
  Bytes out{static_cast<std::uint8_t>(input.kind), static_cast<std::uint8_t>(input.down)};
  u16(out, input.x); u16(out, input.y); u16(out, input.code); u16(out, static_cast<std::uint16_t>(input.delta)); return out;
}
std::optional<Input> decodeInput(const Bytes& b) {
  if (b.size() != 10 || b[0] > Input::Reset || b[1] > 1) return std::nullopt;
  Input input{static_cast<Input::Kind>(b[0]), r16(b.data()+2), r16(b.data()+4), r16(b.data()+6), static_cast<std::int16_t>(r16(b.data()+8)), b[1] != 0};
  if ((input.kind == Input::Key && (input.code < 4 || input.code > 231)) || (input.kind == Input::Button && input.code > 2)) return std::nullopt;
  return input;
}
PacketCodec::PacketCodec(const std::string& token, bool host) {
  if (token.size() < 32 || token.size() > 256) throw std::runtime_error("Invalid session credential");
  const auto root = derive({}, token);
  sendAuth_ = derive(root, host ? "sanser-snv2-h2c-auth" : "sanser-snv2-c2h-auth");
  receiveAuth_ = derive(root, host ? "sanser-snv2-c2h-auth" : "sanser-snv2-h2c-auth");
  sendCipher_ = derive(root, host ? "sanser-snv2-h2c-aes" : "sanser-snv2-c2h-aes");
  receiveCipher_ = derive(root, host ? "sanser-snv2-c2h-aes" : "sanser-snv2-h2c-aes");
  const auto identity = derive(root, "sanser-snv2-session"); std::copy_n(identity.begin(), 16, session_.begin());
}
Bytes PacketCodec::seal(snv2::Header header, const Bytes& payload) const {
  header.sessionId = session_; header.priority = snv2::canonicalPriority(header.packetType);
  header.payloadLength = static_cast<std::uint32_t>(16 + (payload.size() / 16 + 1) * 16);
  const auto initial = snv2::encodeHeader(header);
  // A PRF of the unique stream/sequence header supplies an unpredictable CBC IV.
  const auto iv = snv2::deriveKey(sendCipher_, initial);
  auto encrypted = crypt(sendCipher_, iv.data(), payload, true);
  Bytes body(iv.begin(), iv.begin()+16); body.insert(body.end(), encrypted.begin(), encrypted.end());
  header.authTag = snv2::computeAuthTag(sendAuth_, {initial.data(), snv2::kAuthenticatedHeaderSize}, body);
  const auto wire = snv2::encodeHeader(header);
  Bytes result(wire.begin(), wire.end()); result.insert(result.end(), body.begin(), body.end()); return result;
}
std::optional<std::pair<snv2::Header, Bytes>> PacketCodec::open(const Bytes& packet) const {
  const auto decoded = snv2::decodePacket(packet);
  if (!decoded || decoded.header.sessionId != session_ || decoded.payload.size() < 32 || decoded.payload.size() % 16 != 0) return std::nullopt;
  if (!snv2::verifyAuthTag(receiveAuth_, {packet.data(), snv2::kAuthenticatedHeaderSize}, decoded.payload, decoded.header.authTag)) return std::nullopt;
  try {
    Bytes encrypted(decoded.payload.begin()+16, decoded.payload.end());
    return std::make_pair(decoded.header, crypt(receiveCipher_, decoded.payload.data(), encrypted, false));
  } catch (...) { return std::nullopt; }
}
bool ReplayWindow::accept(std::uint64_t sequence) {
  if (sequence == 0) return false;
  if (sequence > newest_) { const auto shift = sequence-newest_; if (shift >= seen_.size()) seen_.reset(); else seen_ <<= static_cast<std::size_t>(shift); newest_ = sequence; }
  const auto offset = newest_ - sequence;
  if (offset >= seen_.size() || seen_[static_cast<std::size_t>(offset)]) return false;
  seen_.set(static_cast<std::size_t>(offset)); return true;
}
std::chrono::milliseconds videoSendBudget(std::size_t bytes,std::uint32_t bitrate,bool ultra) {
  // 1.2 is the same wire-overhead allowance used by sendLoop. Include already
  // queued video when admitting a frame, but never permit a multi-second queue.
  const auto serialization=static_cast<std::int64_t>(std::ceil(static_cast<double>(bytes)*8000.0/(std::max(1U,bitrate)*1.2)));
  return std::chrono::milliseconds(std::clamp<std::int64_t>(serialization+(ultra ? 40 : 100),ultra ? 40 : 100,500));
}
std::optional<Frame> Reassembler::push(const snv2::Header& h, const Bytes& b) {
  if (b.size() < 8 || h.frameId <= lastFrame_) return std::nullopt;
  const auto index = r16(b.data()), count = r16(b.data()+2); const auto size = r32(b.data()+4);
  if (size < 8 || size > maxFrame || count != (size+chunkSize-1)/chunkSize || index >= count) return std::nullopt;
  const auto offset = static_cast<std::size_t>(index)*chunkSize;
  if (b.size()-8 != std::min(chunkSize, size-offset)) return std::nullopt;
  const auto now = Clock::now();
  for (auto i = frames_.begin(); i != frames_.end();) {
    if (now-i->second.started > videoSendBudget(i->second.data.size(),bitrate_,ultra_)+std::chrono::milliseconds(350)) {
      lostFragments_+=i->second.seen.size()-i->second.received; haveReference_=false;
      bufferedBytes_-=i->second.data.size(); i=frames_.erase(i);
    } else ++i;
  }
  if (!frames_.contains(h.frameId)) {
    // Bound memory and how much reference history a repaired frame can release.
    // Complete P-pictures do not evict an IDR merely by arriving before its tail.
    while(frames_.size()>=24 || bufferedBytes_+size>8*1024*1024) discardThrough(frames_.begin()->first,true);
    frames_.emplace(h.frameId, Partial{Bytes(size), std::vector<bool>(count), 0, now, (h.flags & snv2::kFlagKeyFrame) != 0});
    bufferedBytes_+=size;
  }
  auto& p = frames_.at(h.frameId);
  if (p.data.size() != size || p.seen.size() != count || p.seen[index] || p.keyframe!=((h.flags&snv2::kFlagKeyFrame)!=0)) return std::nullopt;
  receivedBytes_+=b.size()-8;
  // A provisional gap/NACK can be mere reordering. Count a fragment as lost
  // only when a unique retransmission repairs it, or it expires/is evicted.
  if(h.flags&snv2::kFlagRetransmitted) ++lostFragments_;
  std::copy(b.begin()+8, b.end(), p.data.begin()+static_cast<std::ptrdiff_t>(offset)); p.seen[index]=true; ++p.received;
  p.highestSeen=std::max(p.highestSeen,static_cast<std::size_t>(index)+1);
  if (p.received != count) return std::nullopt;
  const auto width=r32(p.data.data()),height=r32(p.data.data()+4);
  if(width<64 || height<64 || width>3840 || height>2160) { bufferedBytes_-=p.data.size(); frames_.erase(h.frameId); return std::nullopt; }
  return take();
}
void Reassembler::discardThrough(std::uint64_t id,bool lost) {
  while(!frames_.empty() && frames_.begin()->first<=id) {
    const auto& partial=frames_.begin()->second;
    if(lost) { lostFragments_+=partial.seen.size()-partial.received; haveReference_=false; }
    bufferedBytes_-=partial.data.size(); frames_.erase(frames_.begin());
  }
}
std::optional<Frame> Reassembler::take() {
  // A fresh complete IDR is independent and can bypass an unrepairable chain.
  for(auto it=frames_.rbegin();it!=frames_.rend();++it) {
    if(it->second.keyframe && it->second.received==it->second.seen.size()) {
      const auto id=it->first; discardThrough(id-1); break;
    }
  }
  if(frames_.empty()) return std::nullopt;
  const auto& [id,p]=*frames_.begin();
  if(p.received!=p.seen.size()) return std::nullopt;
  // A whole P-picture may be reordered, not lost. Preserve the last valid
  // reference while its bounded recovery window is open; a late picture can
  // still bridge the gap without an IDR. Expiry/eviction invalidates the chain.
  if(!p.keyframe && (!haveReference_ || id!=lastFrame_+1)) return std::nullopt;
  Frame frame{{p.data.begin()+8,p.data.end()},r32(p.data.data()),r32(p.data.data()+4),p.keyframe,id};
  lastFrame_=id; haveReference_=true; discardThrough(id);
  return frame;
}
std::vector<Reassembler::Repair> Reassembler::repairs(bool& expired,Clock::time_point now,std::chrono::milliseconds retry) {
  std::vector<Repair> result; expired=false;
  for(auto it=frames_.begin();it!=frames_.end();) {
    auto& partial=it->second;
    if(now-partial.started>videoSendBudget(partial.data.size(),bitrate_,ultra_)+std::chrono::milliseconds(350)) { expired=true; haveReference_=false; lostFragments_+=partial.seen.size()-partial.received; bufferedBytes_-=partial.data.size(); it=frames_.erase(it); continue; }
    if(partial.received==partial.seen.size()) { ++it; continue; }
    // A missing tail may still be in the sender's pacer. Do not NACK packets
    // that have not been sent yet; interior holes can be repaired immediately.
    const bool gap=partial.received<partial.highestSeen;
    const auto budget=videoSendBudget(partial.data.size(),bitrate_,ultra_);
    const auto serialization=budget-std::chrono::milliseconds(ultra_ ? 40 : 100);
    const auto reorder=std::clamp((retry-std::chrono::milliseconds(8))/4,std::chrono::milliseconds(3),std::chrono::milliseconds(20));
    const auto tailDue=serialization+reorder;
    // On a slower path, allow the picture's own serialization plus a small
    // reorder window before declaring holes. Complete frames are never held.
    // Otherwise harmless packet reordering can fill the repair lane and starve
    // the few repairs that are genuinely needed. Fast paths keep a 3ms NACK.
    const auto gapDue=retry>std::chrono::milliseconds(40) ? tailDue : std::chrono::milliseconds(3);
    if(partial.repairAttempts<3 && now-partial.lastRepair>=retry &&
       now-partial.started>=(gap ? gapDue : tailDue) && result.size()<4) {
      Repair request{it->first,{}};
      const auto limit=now-partial.started<tailDue ? partial.highestSeen : partial.seen.size();
      for(std::size_t i=0;i<limit && request.missing.size()<32;++i) if(!partial.seen[i]) request.missing.push_back(static_cast<std::uint16_t>(i));
      if(!request.missing.empty()) { result.push_back(std::move(request)); partial.lastRepair=now; ++partial.repairAttempts; }
    }
    ++it;
  }
  return result;
}

Peer::Peer(const Options& o) : options_(o), codec_(o.token, o.host), reassembler_(o.bitrate,o.ultraLowLatency) {
  videoBudget_=o.bitrate;
#ifdef _WIN32
  WSADATA data{}; if (WSAStartup(MAKEWORD(2,2), &data) != 0) throw std::runtime_error("Winsock unavailable");
  timerResolution_=timeBeginPeriod(1)==TIMERR_NOERROR;
#endif
  try {
  std::random_device random; do { generation_=random(); } while(generation_==0);
  const auto remote = endpoint(o.peer);
  const auto fd = ::socket(AF_INET, SOCK_DGRAM, 0); socket_ = static_cast<std::intptr_t>(fd);
  if (socket_ == -1) throw std::runtime_error("Unable to create media socket");
  sockaddr_in local{}; local.sin_family = AF_INET; local.sin_port = htons(o.port); local.sin_addr.s_addr = htonl(INADDR_ANY);
  if (::bind(fd, reinterpret_cast<const sockaddr*>(&local), sizeof(local)) != 0 || ::connect(fd, reinterpret_cast<const sockaddr*>(&remote), sizeof(remote)) != 0) throw std::runtime_error("Unable to bind/connect native UDP socket");
#ifdef _WIN32
  DWORD timeout=2; setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,reinterpret_cast<const char*>(&timeout),sizeof(timeout));
  setsockopt(fd,SOL_SOCKET,SO_SNDTIMEO,reinterpret_cast<const char*>(&timeout),sizeof(timeout));
#else
  timeval timeout{0,2000}; setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&timeout,sizeof(timeout));
  setsockopt(fd,SOL_SOCKET,SO_SNDTIMEO,&timeout,sizeof(timeout));
#endif
  int buffer = 4*1024*1024; setsockopt(fd,SOL_SOCKET,SO_RCVBUF,reinterpret_cast<const char*>(&buffer),sizeof(buffer));
  } catch(...) {
    if(socket_!=-1) closeSocket(static_cast<Socket>(socket_));
    socket_=-1;
#ifdef _WIN32
    if(timerResolution_) timeEndPeriod(1);
    WSACleanup();
#endif
    throw;
  }
}
Peer::~Peer() { stop(); if (socket_ != -1) closeSocket(static_cast<Socket>(socket_));
#ifdef _WIN32
  if(timerResolution_) timeEndPeriod(1);
  WSACleanup();
#endif
}
void Peer::start() {
  if (running_.exchange(true)) return;
  lastReceived_=Clock::now();
  try { sender_=std::thread([this] { sendLoop(); }); worker_=std::thread([this] { loop(); }); }
  catch(...) { running_=false; sendReady_.notify_all(); if(sender_.joinable()) sender_.join(); throw; }
}
void Peer::stop() { if (running_) { try { transmit(snv2::PacketType::Disconnect, {}); } catch (...) {} } running_=false; sendReady_.notify_all(); if (worker_.joinable()) worker_.join(); if (sender_.joinable()) sender_.join(); }
void Peer::datagram(const Bytes& b) { ::send(static_cast<Socket>(socket_), reinterpret_cast<const char*>(b.data()), static_cast<int>(b.size()), 0); }
void Peer::transmit(snv2::PacketType type,const Bytes& body,std::uint32_t stream,std::uint64_t frame,std::uint16_t flags,bool reliable,Clock::time_point deadline) {
  std::lock_guard lock(sendMutex_);
  if(type==snv2::PacketType::Video && !(flags & snv2::kFlagRetransmitted) && videoPaused_) return;
  if (stream > 2 || (reliable && pending_.size() >= 64)) { stopReason_=4; running_=false; return; }
  snv2::Header h; h.packetType=type; h.streamId=stream; h.sequence=++sequences_[stream]; h.frameId=frame; h.flags=flags; h.keyId=generation_;
  auto b=codec_.seal(h,body);
  const auto now = Clock::now();
  if (type == snv2::PacketType::Disconnect) { datagram(b); return; }
  const auto lane = type == snv2::PacketType::Video ? ((flags & snv2::kFlagRetransmitted) ? TxScheduler::Repair : TxScheduler::Video)
    : reliable ? TxScheduler::InputLane
    : type == snv2::PacketType::Audio ? TxScheduler::Audio : TxScheduler::Control;
  if (type == snv2::PacketType::MouseMove) tx_.latestInput({b, now, 0});
  else if (!tx_.push(lane, {b, now, frame, false, false, deadline})) {
    if (lane == TxScheduler::Video) { tx_.clearVideo(); outgoingNeedsKeyframe_ = true; ++videoQueueDrops_; }
    else if(lane!=TxScheduler::Repair) { stopReason_=4; running_ = false; } // Stop/reset rather than silently losing key-up.
    return;
  }
  if (reliable) pending_.emplace(h.sequence, Pending{std::move(b),now,now,1,false});
  sendReady_.notify_one();
}
void Peer::video(const Frame& frame) {
  videoPrepared(std::make_shared<const PreparedFrame>(prepareFrame(frame)));
}
PreparedFrame prepareFrame(Frame frame) {
  if(frame.data.empty() || frame.data.size()>maxFrame-8 || frame.width<64 || frame.height<64 || frame.width>3840 || frame.height>2160)
    throw std::runtime_error("Invalid encoded video frame");
  Bytes prefix; u32(prefix,frame.width); u32(prefix,frame.height);
  frame.data.insert(frame.data.begin(),prefix.begin(),prefix.end());
  return {std::move(frame.data),frame.keyframe};
}
void Peer::videoPrepared(std::shared_ptr<const PreparedFrame> prepared) {
  const auto& frame=*prepared;
  std::lock_guard lock(videoMutex_);
  if (!ready_ || !running_ || videoPaused_) return;
  if (outgoingNeedsKeyframe_ && !frame.keyframe) { if (onKeyframe) onKeyframe(); return; }
  if (frame.keyframe) outgoingNeedsKeyframe_ = false;
  const auto& bytes=frame.payload;
  if (bytes.size()<=8 || bytes.size()>maxFrame) return;
  const auto id=++frame_;
  offeredBytes_+=bytes.size();
  videoCacheBytes_+=bytes.size();
  videoCache_.emplace(id,CachedVideo{std::move(prepared),Clock::now(),{}, {}});
  while(videoCache_.size()>64 || videoCacheBytes_>8*1024*1024) {
    videoCacheBytes_-=videoCache_.begin()->second.frame->payload.size(); videoCache_.erase(videoCache_.begin());
  }
  const auto count=(bytes.size()+chunkSize-1)/chunkSize;
  Clock::time_point deadline;
  {
    std::lock_guard sendLock(sendMutex_);
    deadline=Clock::now()+videoSendBudget(bytes.size()+tx_.queuedVideoBytes(),options_.bitrate,options_.ultraLowLatency);
  }
  for (std::size_t i=0; i<count && running_; ++i) {
    sendVideoFragment(frame,id,i,false,deadline);
    if (outgoingNeedsKeyframe_) { if (onKeyframe) onKeyframe(); break; }
  }
}
void Peer::pauseVideo(bool paused) {
  videoPaused_=paused;
  std::lock_guard lock(sendMutex_);
  // Preserve priority lanes and the repair lane/cache. No fresh P-frame may
  // follow a pause without a new IDR. Never take videoMutex_ from Delivery.
  tx_.clearRealtimeVideo();
  outgoingNeedsKeyframe_=true;
  offeredBytes_=0;
  sendReady_.notify_one();
}
Peer::VideoFeedback Peer::videoFeedback() const {
  std::lock_guard lock(feedbackMutex_);
  auto result=feedback_; result.sentFrame=frame_.load(); return result;
}
void Peer::sendVideoFragment(const PreparedFrame& frame,std::uint64_t id,std::size_t index,bool repair,Clock::time_point deadline) {
  const auto count=(frame.payload.size()+chunkSize-1)/chunkSize;
  Bytes fragment; u16(fragment,static_cast<std::uint16_t>(index)); u16(fragment,static_cast<std::uint16_t>(count)); u32(fragment,static_cast<std::uint32_t>(frame.payload.size()));
  const auto offset=index*chunkSize;
  fragment.insert(fragment.end(),frame.payload.begin()+static_cast<std::ptrdiff_t>(offset),frame.payload.begin()+static_cast<std::ptrdiff_t>(std::min(offset+chunkSize,frame.payload.size())));
  transmit(snv2::PacketType::Video,fragment,1,id,static_cast<std::uint16_t>((frame.keyframe ? snv2::kFlagKeyFrame : 0) | (repair ? snv2::kFlagRetransmitted : 0)),false,deadline);
}
void Peer::queueVideoRepair(const Bytes& bytes) {
  if(bytes.size()<16 || std::memcmp(bytes.data(),"SVN1",4)!=0) return;
  std::uint64_t id=0; for(std::size_t i=4;i<12;++i) id=(id<<8)|bytes[i];
  const auto count=r16(bytes.data()+12);
  if(count==0 || count>32 || bytes.size()!=14+static_cast<std::size_t>(count)*2) return;
  bool missing=false;
  {
    std::lock_guard lock(videoMutex_);
    const auto found=videoCache_.find(id);
    if(found==videoCache_.end() || Clock::now()-found->second.sent>std::chrono::seconds(1)) missing=true;
    else {
      auto& cached=found->second;
      if(Clock::now()-cached.lastRequest<std::chrono::milliseconds(10)) return;
      const auto fragments=(cached.frame->payload.size()+chunkSize-1)/chunkSize;
      std::set<std::uint16_t> requested;
      for(std::size_t i=0;i<count;++i) {
        const auto index=r16(bytes.data()+14+i*2);
        if(index>=fragments || !requested.insert(index).second) return;
      }
      if(cached.repair.size()+requested.size()>64) return;
      cached.lastRequest=Clock::now();
      for(auto index:requested) if(std::find(cached.repair.begin(),cached.repair.end(),index)==cached.repair.end()) cached.repair.push_back(index);
    }
  }
  if(missing) { ++repairCacheMisses_; if(onKeyframe) onKeyframe(); }
}
void Peer::serviceVideoRepairs() {
  std::lock_guard lock(videoMutex_);
  unsigned budget=8;
  for(auto it=videoCache_.begin();it!=videoCache_.end();) {
    if(Clock::now()-it->second.sent>std::chrono::seconds(1)) {
      videoCacheBytes_-=it->second.frame->payload.size(); it=videoCache_.erase(it); continue;
    }
    auto& cached=it->second;
    while(budget && !cached.repair.empty()) {
      const auto index=cached.repair.front(); cached.repair.pop_front();
      sendVideoFragment(*cached.frame,it->first,index,true); ++repairedDatagrams_; --budget;
    }
    ++it;
  }
}

void Peer::input(const Input& input) {
  if (!ready_ || !options_.input || options_.host) return;
  if(input.kind==Input::Key) ++queuedKeys_;
  if(input.kind==Input::Button) ++queuedButtons_;
  const bool reliable=input.kind!=Input::Move;
  transmit(reliable ? snv2::PacketType::Keyboard : snv2::PacketType::MouseMove,encodeInput(input),reliable ? 2 : 0,0,reliable ? snv2::kFlagAcknowledgementRequired : 0,reliable);
}
void Peer::audio(const AudioSamples& samples) {
  if(audioReady()) transmit(snv2::PacketType::Audio,encodeAudio(samples));
}
void Peer::requestKeyframe() {
  std::lock_guard lock(keyframeRequestMutex_);
  if(!running_) return;
  const auto now=Clock::now(); if (now-lastKeyframe_<std::chrono::milliseconds(options_.ultraLowLatency ? 100 : 300)) return;
  lastKeyframe_=now; transmit(snv2::PacketType::KeyframeRequest,{});
}
void Peer::process(const Bytes& bytes) {
  ++rawDatagrams_;
  const auto packet=codec_.open(bytes); if (!packet) { ++authRejected_; return; }
  const auto& [h,b]=*packet; if (h.streamId>2 || h.keyId==0) return;
  const bool generationChanged=h.keyId!=peerGeneration_;
  if(generationChanged) {
    if(previousGenerations_.contains(h.keyId)) return;
    if(previousGenerations_.size()>=16) { running_=false; return; }
    if(peerGeneration_) previousGenerations_.insert(peerGeneration_);
    peerGeneration_=h.keyId; peerSack_=false; peerAudio_=false; peerVideoProgress_=false; lastAudio_=0; controlProbe_.reset(); for(auto& window:replay_) window=ReplayWindow{};
    reassembler_=Reassembler{options_.bitrate,options_.ultraLowLatency}; lastReportedBytes_=lastReportedLoss_=0; needKeyframe_=true; lastVideo_=0; lastMouse_=0; deliveredInput_=0; receivedInputs_.clear();
    controlAppRtt_=-1; inputRttUs_=inputRttSamples_=inputWorkUs_=inputWorkSamples_=0;
    if(options_.host) {
      { std::lock_guard lock(videoMutex_); videoCache_.clear(); videoCacheBytes_=0; outgoingNeedsKeyframe_=true; }
      { std::lock_guard lock(feedbackMutex_); feedback_={}; }
      busyWindows_=congestionWindows_=recoveryWindows_=0;
    }
    if(options_.host && onInput) onInput(Input{});
  }
  if (h.streamId==2 && h.sequence<=deliveredInput_) { acknowledgeInput(); return; }
  if (!replay_[h.streamId].accept(h.sequence)) { if (h.streamId==2) acknowledgeInput(); return; }
  lastReceived_=Clock::now();
  const bool becameReady=!ready_.exchange(true);
  if((becameReady || generationChanged) && h.packetType!=snv2::PacketType::Disconnect && onReady) onReady();
  if (h.packetType==snv2::PacketType::Keepalive && b==Bytes({'S','A','K','1'})) peerSack_=true;
  // Optional authenticated control RTT on the SAME media transport. Old peers
  // ignore this keepalive extension; no response means unknown, never zero.
  if(h.packetType==snv2::PacketType::Keepalive && h.streamId==0 && b.size()==12) {
    if(options_.host && std::memcmp(b.data(),"SRQ1",4)==0) {
      auto pong=b; pong[2]='P'; transmit(snv2::PacketType::Keepalive,pong); return;
    }
    if(!options_.host && std::memcmp(b.data(),"SRP1",4)==0) {
      std::uint64_t id=0; for(std::size_t i=4;i<12;++i) id=(id<<8)|b[i];
      if(const auto rtt=controlProbe_.received(id,Clock::now())) {
        controlAppRtt_=*rtt;
        if(std::getenv("SANSER_TELEMETRY")) { std::ostringstream line; line<<"SNV2_CONTROL_TIMING appRttMs="<<*rtt; writeTelemetry(line); }
      }
      return;
    }
  }
  if (options_.host && h.streamId==0 && h.packetType==snv2::PacketType::Keepalive && b==Bytes({'S','A','P','1'})) peerAudio_=true;
  if(h.packetType==snv2::PacketType::Audio) {
    if(!options_.host && options_.audio && h.streamId==0 && h.sequence>lastAudio_ && onAudio) {
      if(auto samples=decodeAudio(b)) { lastAudio_=h.sequence; onAudio(std::move(*samples)); }
    }
    return;
  }
  if (h.packetType==snv2::PacketType::Disconnect) { stopReason_=1; running_=false; return; }
  if(options_.host && h.streamId==0 && h.packetType==snv2::PacketType::NetworkFeedback && b.size()==12 && std::memcmp(b.data(),"SVP1",4)==0) {
    std::uint64_t completed=0; for(std::size_t i=4;i<12;++i) completed=(completed<<8)|b[i];
    if(completed<=frame_.load()) {
      std::lock_guard lock(feedbackMutex_);
      if(!peerVideoProgress_ || completed>feedback_.completedFrame) lastVideoProgress_=Clock::now();
      peerVideoProgress_=true;
      feedback_.completedFrame=std::max(feedback_.completedFrame,completed);
    }
    return;
  }
  if(options_.host && h.streamId==0 && h.packetType==snv2::PacketType::NetworkFeedback && b.size()==24 && std::memcmp(b.data(),"SVF1",4)==0) {
    if(Clock::now()-lastVideoFeedback_<std::chrono::milliseconds(250)) return;
    const auto elapsed=r32(b.data()+4); if(elapsed<250 || elapsed>2000) return;
    lastVideoFeedback_=Clock::now();
    std::uint64_t delivered=0; for(std::size_t i=8;i<16;++i) delivered=(delivered<<8)|b[i];
    const auto packets=r32(b.data()+16), missing=r32(b.data()+20);
    const auto offered=offeredBytes_.exchange(0), drops=videoQueueDrops_.load();
    // Compare actual offered encoded bytes, never the configured target. An
    // idle/compressible desktop must not be mistaken for insufficient bandwidth.
    const bool busy=offered>=32000;
    busyWindows_=busy ? std::min(4U,busyWindows_+1) : 0;
    const bool loss=packets>=16 && static_cast<double>(missing)/packets>0.03;
    const bool congested=busy && (static_cast<double>(delivered)<static_cast<double>(offered)*0.85 || loss || drops>lastFeedbackDrops_);
    bool deliveryStalled=false, liveStalled=false;
    {
      std::lock_guard lock(feedbackMutex_);
      ++feedback_.serial; feedback_.deliveredBytes=delivered;
      feedback_.congested=congested; feedback_.received=Clock::now();
      liveStalled=peerVideoProgress_ && feedback_.completedFrame>0 && busyWindows_>=2 &&
        Clock::now()-lastVideoProgress_>=std::chrono::milliseconds(750);
      deliveryStalled=peerVideoProgress_ && (feedback_.completedFrame==0 || liveStalled);
    }
    lastFeedbackDrops_=drops;
    if(congested) {
      recoveryWindows_=0;
      if(++congestionWindows_>=(liveStalled ? 1U : 2U)) {
        auto target=static_cast<std::uint32_t>(videoBudget_.load()*0.85);
        if(deliveryStalled) {
          // No picture progress despite sustained source activity: recover at
          // a rate this peer actually receives, with repair headroom.
          // Only peers explicitly reporting SVP1 progress can enter this path.
          const auto deliveredBps=std::clamp(static_cast<double>(delivered)*8000.0/elapsed,0.0,static_cast<double>(options_.bitrate));
          target=std::min(target,static_cast<std::uint32_t>(deliveredBps*0.8));
        }
        const auto previous=videoBudget_.load();
        videoBudget_=std::max(std::min(options_.bitrate,5000000U),target);
        if(deliveryStalled && videoBudget_<previous && onKeyframe) onKeyframe();
        congestionWindows_=0;
      }
    } else {
      congestionWindows_=0;
      if(busy && ++recoveryWindows_>=10) {
        videoBudget_=std::min(options_.bitrate,videoBudget_.load()+std::max(500000U,videoBudget_.load()/20));
        recoveryWindows_=0;
      }
    }
    return;
  }
  if (h.packetType==snv2::PacketType::NetworkFeedback && (b.size()==8 || b.size()==16)) {
    std::uint64_t ack=0, mask=0;
    for (std::size_t i=0;i<8;++i) ack=(ack<<8)|b[i];
    for (std::size_t i=8;i<b.size();++i) mask=(mask<<8)|b[i];
    std::lock_guard lock(sendMutex_);
    if (ack > sequences_[2]) return;
    const auto now=Clock::now();
    const auto recordRtt=[&](const Pending& p) {
      if(p.sacked) return;
      inputRttUs_+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(now-p.created).count()); ++inputRttSamples_;
      inputDelivery_.acknowledged(std::chrono::duration_cast<std::chrono::milliseconds>(now-p.created));
    };
    for(auto it=pending_.begin();it!=pending_.upper_bound(ack);++it)
      recordRtt(it->second);
    pending_.erase(pending_.begin(),pending_.upper_bound(ack));
    for (auto& [sequence,p] : pending_) {
      const auto offset=sequence-ack-1;
      if (offset<64 && (mask & (std::uint64_t{1}<<offset)) && !p.sacked) {
        recordRtt(p);
        p.sacked=true;
      }
    }
    return;
  }
  if (h.packetType==snv2::PacketType::Nack && options_.host) { queueVideoRepair(b); return; }
  if (h.packetType==snv2::PacketType::KeyframeRequest && options_.host) { if(onKeyframe) onKeyframe(); return; }
  if (h.packetType==snv2::PacketType::Video && !options_.host && h.streamId==1) {
    lastVideoReceived_=Clock::now();
    ++videoDatagrams_;
    ++receivedVideoDatagrams_;
    deliverVideo(reassembler_.push(h,b));
    if(needKeyframe_ && !reassembler_.assemblingKeyframe()) requestKeyframe();
    return;
  }
  if (options_.host && (h.packetType==snv2::PacketType::Keyboard || h.packetType==snv2::PacketType::MouseMove)) {
    auto event=decodeInput(b); if(!event) return;
    if(h.streamId==2) {
      if(h.sequence>deliveredInput_+64) { running_=false; return; }
      receivedInputs_.emplace(h.sequence,*event);
      while(receivedInputs_.contains(deliveredInput_+1)) {
        const auto next=receivedInputs_.at(++deliveredInput_);
        // ACK receipt even if the host denies injection. Otherwise a stale
        // client input preference kills an otherwise healthy video session.
        if(options_.input && onInput) {
          const auto began=Clock::now(); onInput(next);
          inputWorkUs_+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(Clock::now()-began).count()); ++inputWorkSamples_;
          if(next.kind==Input::Key) ++deliveredKeys_; if(next.kind==Input::Button) ++deliveredButtons_;
        }
        receivedInputs_.erase(deliveredInput_);
      }
      acknowledgeInput();
    } else if(event->kind==Input::Move && h.sequence>lastMouse_) {
      lastMouse_=h.sequence;
      if(options_.input && onInput) onInput(*event);
    }
  }
}
void Peer::deliverVideo(std::optional<Frame> frame) {
  if(!frame) frame=reassembler_.take();
  while(frame) {
    ++completedFrames_;
    const auto id=frame->id;
    if(lastVideo_ && id!=lastVideo_+1 && !frame->keyframe) needKeyframe_=true;
    lastVideo_=id;
    if(!needKeyframe_ || frame->keyframe) {
      needKeyframe_=false;
      if(onFrame) onFrame(std::move(*frame));
      completedVideoFrame_=id;
    } else ++keyframeWaitDropped_;
    frame=reassembler_.take();
  }
  if(reassembler_.awaitingReference()) needKeyframe_=true;
}
void Peer::acknowledgeInput() {
  Bytes ack;
  for(int i=7;i>=0;--i) ack.push_back(static_cast<std::uint8_t>(deliveredInput_>>(i*8)));
  if (peerSack_) {
    std::uint64_t mask=0;
    for (const auto& [sequence,event] : receivedInputs_) {
      (void)event;
      const auto offset=sequence-deliveredInput_-1;
      if(offset<64) mask|=std::uint64_t{1}<<offset;
    }
    for(int i=7;i>=0;--i) ack.push_back(static_cast<std::uint8_t>(mask>>(i*8)));
  }
  transmit(snv2::PacketType::NetworkFeedback,ack);
}
void Peer::sendLoop() {
  auto creditAt=Clock::now();
  constexpr double maxBurstBytes=8*1500;
  double credit=maxBurstBytes;
  // Bitrate describes encoded video. Budget framing overhead separately.
  const double bytesPerSecond=static_cast<double>(options_.bitrate)*1.2/8.0;
  try {
    while(running_) {
      std::optional<TxScheduler::Packet> packet;
      {
        std::unique_lock lock(sendMutex_);
        const auto now=Clock::now();
        credit=std::min(maxBurstBytes, credit+std::chrono::duration<double>(now-creditAt).count()*bytesPerSecond);
        creditAt=now;
        const auto videoBytes=tx_.nextVideoBytes();
        packet=tx_.pop(credit>=static_cast<double>(videoBytes));
        if(!packet) {
          const auto nextVideo=now+std::chrono::microseconds(static_cast<std::uint64_t>(
            std::max(1.0,(static_cast<double>(videoBytes)-credit)*1000000.0/bytesPerSecond)));
          sendReady_.wait_until(lock, tx_.hasVideo() ? nextVideo : now+std::chrono::milliseconds(20),
            [&] { return !running_ || tx_.hasPriority() || (tx_.hasVideo() && Clock::now()>=nextVideo); });
          continue;
        }
        if(packet->audio && now-packet->queued>std::chrono::milliseconds(40)) continue;
        if (packet->frame && ((packet->deadline!=Clock::time_point{} && now>packet->deadline) ||
            (packet->deadline==Clock::time_point{} && now-packet->queued>std::chrono::milliseconds(100)))) {
          if(!packet->repair) { tx_.clearVideo(); outgoingNeedsKeyframe_=true; ++videoQueueDrops_; }
          continue;
        }
      }
      datagram(packet->bytes);
      if (packet->frame) {
        credit-=static_cast<double>(packet->bytes.size());
      }
    }
  } catch (const std::exception& error) { std::cerr<<"SNV2_SEND_ERROR "<<error.what()<<std::endl; stopReason_=5; running_=false; }
}
void Peer::logHealth() {
  if(!std::getenv("SANSER_TELEMETRY")) return;
  std::size_t pending;
  { std::lock_guard lock(sendMutex_); pending=pending_.size(); }
  std::ostringstream line;
  line<<"SNV2_TRANSPORT_HEALTH ready="<<ready_.load()<<" running="<<running_.load()
    <<" rawDatagrams="<<rawDatagrams_<<" authRejected="<<authRejected_
    <<" videoDatagrams="<<videoDatagrams_<<" completed="<<completedFrames_
    <<" keyframeWaitDropped="<<keyframeWaitDropped_<<" nackSent="<<nackSent_
    <<" repaired="<<repairedDatagrams_.load()<<" repairCacheMisses="<<repairCacheMisses_.load()
    <<" sendQueueDrops="<<videoQueueDrops_.load()<<" pending="<<pending<<" stopReason="<<stopReason_.load(); writeTelemetry(line);
  if(inputRttSamples_ || inputWorkSamples_) {
    line.str(""); line.clear();
    line<<"SNV2_INPUT_TIMING";
    if(inputRttSamples_) line<<" avgRttMs="<<inputRttUs_/1000.0/inputRttSamples_;
    if(inputWorkSamples_) line<<" hostAvgWorkMs="<<inputWorkUs_/1000.0/inputWorkSamples_;
    writeTelemetry(line);
    inputRttUs_=inputRttSamples_=inputWorkUs_=inputWorkSamples_=0;
  }
}
void Peer::loop() {
  auto lastHello=Clock::time_point{};
  auto lastControlProbe=Clock::time_point{};
  auto lastInputLog=Clock::now(); std::uint64_t lastInputCount=0;
  auto lastFeedback=Clock::now();
  auto lastHealth=Clock::now();
  try {
    while(running_ && !stopRequested) {
      const auto now=Clock::now();
      if(!options_.host && ready_ && now-lastControlProbe>=std::chrono::seconds(1)) {
        const auto id=controlProbe_.sent(now); Bytes probe{'S','R','Q','1'};
        for(int i=7;i>=0;--i) probe.push_back(static_cast<std::uint8_t>(id>>(i*8)));
        transmit(snv2::PacketType::Keepalive,probe); lastControlProbe=now;
      }
      if(now-lastHealth>=std::chrono::seconds(1)) { logHealth(); lastHealth=now; }
      if(std::getenv("SANSER_TELEMETRY") && now-lastInputLog>=std::chrono::seconds(1)) {
        const auto queuedKeys=queuedKeys_.load(),queuedButtons=queuedButtons_.load();
        const auto total=queuedKeys+queuedButtons+deliveredKeys_+deliveredButtons_;
        if(total!=lastInputCount) {
          std::ostringstream line;
          line<<"SNV2_INPUT_HEALTH role="<<(options_.host ? "host" : "client")
            <<" queuedKeys="<<queuedKeys<<" queuedButtons="<<queuedButtons
            <<" deliveredKeys="<<deliveredKeys_<<" deliveredButtons="<<deliveredButtons_; writeTelemetry(line);
          lastInputCount=total;
        }
        lastInputLog=now;
      }
      if(now-lastHello>std::chrono::milliseconds(options_.ultraLowLatency ? 100 : 500)) {
        transmit(snv2::PacketType::Keepalive,Bytes{'S','A','K','1'});
        // Optional authenticated capability. Old engines safely ignore it;
        // hosts never send audio until a receiving client explicitly opts in.
        if(!options_.host && options_.audio && onAudio) transmit(snv2::PacketType::Keepalive,Bytes{'S','A','P','1'});
        lastHello=now;
      }
      // Ultra changes presentation latency, not whether a short Wi-Fi hiccup
      // destroys the window. Startup/NAT handoff keeps its normal 15s budget.
      if(now-lastReceived_>(ready_ && options_.ultraLowLatency ? std::chrono::seconds(5) : std::chrono::seconds(15))) { stopReason_=2; running_=false; break; }
      { std::lock_guard lock(sendMutex_);
        for(auto& [seq,p]:pending_) {
          (void)seq;
          const auto deadline=inputDelivery_.deadline();
          if(now-p.created>deadline) {
            std::fprintf(stderr,"SNV2_INPUT_ACK_TIMEOUT pending=%zu deadlineMs=%lld\n",pending_.size(),static_cast<long long>(deadline.count()));
            stopReason_=3; running_=false; break;
          }
          if(!p.sacked && now-p.sent>inputDelivery_.retry(options_.ultraLowLatency,p.attempts)) {
            if (!tx_.push(TxScheduler::InputLane, {p.bytes, now, 0})) { running_=false; break; }
            ++p.attempts; p.sent=now; sendReady_.notify_one();
          }
        }
      }
      std::array<char,1500> buffer{}; const auto received=recv(static_cast<Socket>(socket_),buffer.data(),static_cast<int>(buffer.size()),0);
      if(received>0) process(Bytes(buffer.begin(),buffer.begin()+received));
      if(options_.host) serviceVideoRepairs();
      else {
        bool expired=false;
        std::chrono::milliseconds repairRetry;
        {
          std::lock_guard lock(sendMutex_);
          // Reuse the observed ACK delivery bound, not a cross-machine clock.
          // The first NACK remains prompt; avoid spending all three retries
          // before one WAN repair could possibly make a round trip.
          repairRetry=inputDelivery_.repairRetry();
        }
        for(const auto& repair:reassembler_.repairs(expired,Clock::now(),repairRetry)) {
          Bytes request{'S','V','N','1'};
          for(int i=7;i>=0;--i) request.push_back(static_cast<std::uint8_t>(repair.frame>>(i*8)));
          u16(request,static_cast<std::uint16_t>(repair.missing.size()));
          for(auto index:repair.missing) u16(request,index);
          transmit(snv2::PacketType::Nack,request);
          ++nackSent_;
        }
        deliverVideo();
        if(!reassembler_.assemblingKeyframe() && (expired || (ready_ && needKeyframe_ && now-lastVideoReceived_>std::chrono::seconds(1)))) requestKeyframe();
        if(ready_ && now-lastFeedback>=std::chrono::milliseconds(500)) {
          Bytes feedback{'S','V','F','1'};
          u32(feedback,static_cast<std::uint32_t>(std::chrono::duration_cast<std::chrono::milliseconds>(now-lastFeedback).count()));
          const auto receivedBytes=reassembler_.receivedBytes();
          const auto delivered=receivedBytes-lastReportedBytes_;
          for(int i=7;i>=0;--i) feedback.push_back(static_cast<std::uint8_t>(delivered>>(i*8)));
          lastReportedBytes_=receivedBytes;
          u32(feedback,static_cast<std::uint32_t>(std::min<std::uint64_t>(receivedVideoDatagrams_,UINT32_MAX)));
          const auto lost=reassembler_.lostFragments();
          u32(feedback,static_cast<std::uint32_t>(std::min<std::uint64_t>(lost-lastReportedLoss_,UINT32_MAX)));
          lastReportedLoss_=lost;
          transmit(snv2::PacketType::NetworkFeedback,feedback);
          // Separate optional control message keeps the existing SVF1 wire
          // format compatible. Completion is not a claim of display scan-out.
          Bytes progress{'S','V','P','1'};
          for(int i=7;i>=0;--i) progress.push_back(static_cast<std::uint8_t>(completedVideoFrame_>>(i*8)));
          transmit(snv2::PacketType::NetworkFeedback,progress);
          receivedVideoDatagrams_=0; lastFeedback=now;
        }
      }
    }
  } catch (const std::exception& error) { std::cerr<<"SNV2_RECEIVE_ERROR "<<error.what()<<std::endl; stopReason_=5; running_=false; }
  try { transmit(snv2::PacketType::Disconnect, {}); } catch (...) {}
  running_=false; ready_=false; sendReady_.notify_all();
  logHealth();
  if(stopReason_>=2) std::cerr<<"SNV2_STOP reason="<<stopReason_.load()
    <<" (2=peer-timeout 3=input-ack-timeout 4=priority-queue-overflow 5=transport-error)"<<std::endl;
  if(options_.host && onInput) onInput(Input{});
  if(onStopped) onStopped();
}
}
