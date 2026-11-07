#include "udp_io_policy.h"
#include "telemetry.h"
#include "transport.h"
#include "media_thread.h"
#include "text_input.h"
#include "user_ipc.h"
#include "session_shortcuts.h"
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
#if defined(__ANDROID__)
#include "../mobile/android_crypto.h"
#else
#include <CommonCrypto/CommonCryptor.h>
#endif
#endif

namespace sanser::desktop {
namespace {
std::atomic<bool> stopRequested{false};
std::atomic<bool> externalVoice{false};
std::mutex nativeCommandMutex;
std::vector<std::string> nativeCommands;
std::vector<std::string> voiceCommands;
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
#elif defined(__ANDROID__)
  const auto encoded=sanser_android_aes(key.data(),iv,input.data(),input.size(),encrypt?1:0,out.data(),out.size());
  if(encoded<0) throw std::runtime_error("Android AES operation failed");
  const auto size=static_cast<std::size_t>(encoded);
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
int socketError() {
#ifdef _WIN32
  return WSAGetLastError();
#else
  return errno;
#endif
}
DatagramIo::Result sendDatagram(std::intptr_t socket,const std::uint8_t* bytes,std::size_t size) {
  const auto sent=::send(static_cast<Socket>(socket),reinterpret_cast<const char*>(bytes),static_cast<int>(size),0);
  return {sent,sent<0 ? socketError() : 0};
}
DatagramIo::Result receiveDatagram(std::intptr_t socket,std::uint8_t* bytes,std::size_t size) {
  const auto received=::recv(static_cast<Socket>(socket),reinterpret_cast<char*>(bytes),static_cast<int>(size),0);
  return {received,received<0 ? socketError() : 0};
}
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
bool externalVoiceActive() { return externalVoice.load(std::memory_order_acquire); }
void externalVoiceActive(bool active) { externalVoice.store(active,std::memory_order_release); }
Options parseOptions(bool host, int argc, char** argv) {
  Options o; o.host = host;
  o.files=std::getenv("SANSER_FILE_TRANSFER") && std::getenv("SANSER_CONTROL_STDIN");
  if (std::getenv("SANSER_CONTROL_STDIN")) {
    std::thread([] {
      std::string line; bool oversized=false; int c;
      while((c=std::fgetc(stdin))!=EOF) {
        if(c!='\n') { if(line.size()<sanser::maxClipboardBytes*2+128) line.push_back(static_cast<char>(c)); else oversized=true; continue; }
        if(!oversized && line=="stop") break;
        if(!oversized) {
          std::lock_guard lock(nativeCommandMutex);
          if(line.starts_with("voice-config\t") || line.starts_with("voice-mute\t") || line.starts_with("voice-external\t")) {
            if(voiceCommands.size()>=16) voiceCommands.erase(voiceCommands.begin());
            voiceCommands.push_back(std::move(line));
          } else {
            if(nativeCommands.size()>=32) {
              if(const auto request=parseUserCommand(line)) { if(request->kind==UserCommand::FileMessage) {if(request->body.size()>=4) {std::uint32_t epoch=0;for(unsigned i=0;i<4;++i)epoch=(epoch<<8)|static_cast<std::uint8_t>(request->body[i]);std::fprintf(stdout,"SNUSER_FILE_FAIL\t%u\n",epoch);std::fflush(stdout);}}else userReply(request->request,2);line.clear();oversized=false;continue; }
              break;
            }
            nativeCommands.push_back(std::move(line));
          }
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
    else if (arg == "--group-voice") o.groupVoice = true;
    else if (arg == "--share-peer-id") o.viewerId = value();
    else if (arg == "--disable-input") o.input = false;
    else if (arg == "--enable-clipboard") o.clipboard = true;
    else if (arg == "--disable-audio") o.audio = false;
    else if (arg == "--mute-audio") o.audioMuted = true;
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
  if(o.groupVoice && (o.input || (host && !o.sharing))) throw std::runtime_error("Group voice requires a view-only sharing session");
  o.width &= ~1U; o.height &= ~1U;
  return o;
}
std::vector<std::string> takeNativeCommands() {
  std::lock_guard lock(nativeCommandMutex); std::vector<std::string> result;
  result.swap(nativeCommands); return result;
}
std::vector<std::string> takeVoiceCommands() {
  std::lock_guard lock(nativeCommandMutex); std::vector<std::string> result;
  result.swap(voiceCommands); return result;
}
bool nativeStopRequested() { return stopRequested.load(); }
Bytes encodeInput(const Input& input) {
  if(input.kind==Input::Text) {
    if(!sanser::textInputUtf16(input.text)) return {};
    Bytes out{Input::Text}; out.insert(out.end(),input.text.begin(),input.text.end()); return out;
  }
  Bytes out{static_cast<std::uint8_t>(input.kind), static_cast<std::uint8_t>(input.down)};
  u16(out, input.x); u16(out, input.y); u16(out, input.code); u16(out, static_cast<std::uint16_t>(input.delta)); return out;
}
std::optional<Input> decodeInput(const Bytes& b) {
  if(!b.empty() && b[0]==Input::Text) {
    if(b.size()>sanser::maxTextInputBytes+1) return std::nullopt;
    Input input; input.kind=Input::Text; input.text.assign(b.begin()+1,b.end());
    return sanser::textInputUtf16(input.text) ? std::optional<Input>{std::move(input)}:std::nullopt;
  }
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
std::chrono::milliseconds videoSendBudget(std::size_t bytes,std::uint32_t bitrate,bool ultra,bool mobileKeyframe) {
  // 1.2 is the same wire-overhead allowance used by sendLoop. Include already
  // queued video when admitting a frame, but never permit a multi-second queue.
  const auto serialization=static_cast<std::int64_t>(std::ceil(static_cast<double>(bytes)*8000.0/(std::max(1U,bitrate)*1.2)));
  // A narrowly paced IDR can take longer than the normal desktop queue cap.
  // This is an expiry bound, never a playout delay. Raw capture admission
  // prevents dependent pictures building up behind it on mobile hosts.
  if(mobileKeyframe) return std::chrono::milliseconds(1500);
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
    if (now-i->second.started > videoSendBudget(i->second.data.size(),bitrate_,ultra_,mobile_ && i->second.keyframe)+std::chrono::milliseconds(350)) {
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
    if(now-partial.started>videoSendBudget(partial.data.size(),bitrate_,ultra_,mobile_ && partial.keyframe)+std::chrono::milliseconds(350)) { expired=true; haveReference_=false; lostFragments_+=partial.seen.size()-partial.received; bufferedBytes_-=partial.data.size(); it=frames_.erase(it); continue; }
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

Peer::Peer(const Options& o,DatagramIo io) : options_(o), io_(io), codec_(o.token, o.host), reassembler_(o.bitrate,o.ultraLowLatency) {
  if(!io_.send) io_.send=sendDatagram;
  if(!io_.receive) io_.receive=receiveDatagram;
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
  userMailbox_->alive=true;
  audioMuted_=options_.audioMuted;
  voiceSlot_=options_.voiceSlot;
  lastReceived_=Clock::now();
  try {
    if(options_.audio && options_.audioOpus) {
      if(options_.host && !options_.sharedAudioEncoder) audioEncoder_=std::make_unique<sanser::audio::Encoder>();
      else if(onAudio) audioReceiver_=std::make_unique<sanser::audio::ReceiveWorker>([this](sanser::audio::Decoded decoded) { if(!audioMuted_) onAudio(std::move(decoded.samples)); });
    }
    if(options_.host && options_.input && !options_.sharing) inputWorker_=std::make_unique<InputWorker>(
      [this](Input event) {
        if(onInput) {
          onInput(event);
          if(event.kind==Input::Key) ++deliveredKeys_;
          if(event.kind==Input::Button) ++deliveredButtons_;
        }
      },
      [this] {
        std::cerr<<"SNV2_INPUT_ERROR reason=os-injection-failed action=stop-and-reset"<<std::endl;
        stopReason_=5; running_=false; sendReady_.notify_all();
      });
    sender_=std::thread([this] { sendLoop(); }); worker_=std::thread([this] { loop(); });
  }
  catch(...) { running_=false; sendReady_.notify_all(); if(sender_.joinable()) sender_.join(); if(inputWorker_) inputWorker_->stop(); throw; }
}
void Peer::stop() { userMailbox_->alive=false; if (running_) { try { transmit(snv2::PacketType::Disconnect, {}); } catch (...) {} } running_=false; sendReady_.notify_all(); if (worker_.joinable()) worker_.join(); if (sender_.joinable()) sender_.join(); audioReceiver_.reset(); if(inputWorker_) inputWorker_->stop(); }
void Peer::datagram(const Bytes& b) {
  const auto result=io_.send(socket_,b.data(),b.size());
  if(result.bytes==static_cast<std::ptrdiff_t>(b.size())) return;
  if(result.bytes<0 && udp::transientSendError(result.error)) {
    ++sendTransientErrors_;
    // Reliable input keeps its pending sequence/retry deadline. Missing video
    // fragments use NACK/cache repair; audio expires instead of accumulating.
    return;
  }
  throw std::runtime_error("UDP send failed, socket error "+std::to_string(result.error));
}
bool Peer::transmit(snv2::PacketType type,const Bytes& body,std::uint32_t stream,std::uint64_t frame,std::uint16_t flags,bool reliable,Clock::time_point deadline,std::shared_ptr<std::atomic<std::uint32_t>> originalSent) {
  const auto queuedAt=Clock::now(); // Include sender mutex/encryption time in application input latency.
  std::lock_guard lock(sendMutex_);
  if(!options_.host && inputSuspended_ && (type==snv2::PacketType::Keyboard || type==snv2::PacketType::MouseMove)) return false;
  if(!options_.host && type==snv2::PacketType::Keyboard && !body.empty() && body[0]==Input::Text && !textAvailable()) return false;
  // A text edit can be declined before taking a sequence. It must not tear
  // down a healthy video session merely because the input window is full.
  if(reliable && !body.empty() && body[0]==Input::Text && pending_.size()>=64) return false;
  if(type==snv2::PacketType::Video && !(flags & snv2::kFlagRetransmitted) && videoPaused_) return false;
  if (stream > 2 || (reliable && pending_.size() >= 64)) { stopReason_=4; running_=false; return false; }
  snv2::Header h; h.packetType=type; h.streamId=stream; h.sequence=++sequences_[stream]; h.frameId=frame; h.flags=flags; h.keyId=generation_;
  auto b=codec_.seal(h,body);
  if (type == snv2::PacketType::Disconnect) { datagram(b); return true; }
  const auto lane = type == snv2::PacketType::Video ? ((flags & snv2::kFlagRetransmitted) ? TxScheduler::Repair : TxScheduler::Video)
    : reliable ? TxScheduler::InputLane
    : type == snv2::PacketType::Audio ? (voice::isPacket(body) ? TxScheduler::Voice : TxScheduler::Audio)
    : type == snv2::PacketType::Clipboard ? TxScheduler::UserData : TxScheduler::Control;
  TxScheduler::Packet queued{b,queuedAt,frame,false,false,deadline};
  queued.inputSequence=reliable ? h.sequence : 0;
  queued.input=reliable || type==snv2::PacketType::MouseMove;
  if(lane==TxScheduler::UserData) { queued.userMessage=UserMessages::key(body);queued.userFragment=UserMessages::fragment(body); }
  if(originalSent && type==snv2::PacketType::Video && !(flags&snv2::kFlagRetransmitted) && body.size()>=8) {
    queued.originalSent=std::move(originalSent);queued.originalOrdinal=r16(body.data())+1;
  }
  if (type == snv2::PacketType::MouseMove) tx_.latestInput(std::move(queued));
  else if (!tx_.push(lane, std::move(queued))) {
    if (lane == TxScheduler::Video) { tx_.clearRealtimeVideo(); outgoingNeedsKeyframe_ = true; ++videoQueueDrops_; }
    else if(lane!=TxScheduler::Repair && lane!=TxScheduler::UserData) { stopReason_=4; running_ = false; } // Stop/reset rather than silently losing key-up.
    return false;
  }
  if (reliable) pending_.emplace(h.sequence, Pending{std::move(b),{},queuedAt,0,false,true,static_cast<Input::Kind>(body[0])});
  sendReady_.notify_one();
  return true;
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
  const auto originalSent=std::make_shared<std::atomic<std::uint32_t>>(0);
  videoCache_.emplace(id,CachedVideo{std::move(prepared),Clock::now(),{}, {},originalSent});
  while(videoCache_.size()>64 || videoCacheBytes_>8*1024*1024) {
    videoCacheBytes_-=videoCache_.begin()->second.frame->payload.size(); videoCache_.erase(videoCache_.begin());
  }
  const auto count=(bytes.size()+chunkSize-1)/chunkSize;
  Clock::time_point deadline;
  {
    std::lock_guard sendLock(sendMutex_);
    deadline=Clock::now()+videoSendBudget(bytes.size()+tx_.queuedVideoBytes(),peerVideoLimit_ ? videoBudget_.load():options_.bitrate,options_.ultraLowLatency,peerVideoLimit_ && frame.keyframe);
  }
  for (std::size_t i=0; i<count && running_; ++i) {
    sendVideoFragment(frame,id,i,false,deadline,originalSent);
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
void Peer::sendVideoFragment(const PreparedFrame& frame,std::uint64_t id,std::size_t index,bool repair,Clock::time_point deadline,std::shared_ptr<std::atomic<std::uint32_t>> originalSent) {
  const auto count=(frame.payload.size()+chunkSize-1)/chunkSize;
  Bytes fragment; u16(fragment,static_cast<std::uint16_t>(index)); u16(fragment,static_cast<std::uint16_t>(count)); u32(fragment,static_cast<std::uint32_t>(frame.payload.size()));
  const auto offset=index*chunkSize;
  fragment.insert(fragment.end(),frame.payload.begin()+static_cast<std::ptrdiff_t>(offset),frame.payload.begin()+static_cast<std::ptrdiff_t>(std::min(offset+chunkSize,frame.payload.size())));
  transmit(snv2::PacketType::Video,fragment,1,id,static_cast<std::uint16_t>((frame.keyframe ? snv2::kFlagKeyFrame : 0) | (repair ? snv2::kFlagRetransmitted : 0)),false,deadline,std::move(originalSent));
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
    if(found==videoCache_.end() || Clock::now()-found->second.sent>std::chrono::seconds(peerVideoLimit_ && found->second.frame->keyframe ? 2:1)) missing=true;
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
      const auto sent=cached.originalSent->load(std::memory_order_acquire);
      for(auto index:requested) {
        // A receiver may still use the pre-adaptation serialization estimate.
        // Never let an early NACK duplicate data waiting in our own pacer.
        if(index>=sent){++repairUnsentIgnored_;continue;}
        if(std::find(cached.repair.begin(),cached.repair.end(),index)==cached.repair.end())cached.repair.push_back(index);
      }
    }
  }
  if(missing) { ++repairCacheMisses_; if(onKeyframe) onKeyframe(); }
}
void Peer::serviceVideoRepairs() {
  // Publishing a large frame can be sealing fragments under this mutex. UDP
  // receive/control must not wait for that work; repair resumes next iteration.
  std::unique_lock lock(videoMutex_,std::try_to_lock);
  if(!lock.owns_lock()) return;
  unsigned budget=8;
  for(auto it=videoCache_.begin();it!=videoCache_.end();) {
    if(Clock::now()-it->second.sent>std::chrono::seconds(peerVideoLimit_ && it->second.frame->keyframe ? 2:1)) {
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
void Peer::announceVideoRate() {
  if(!options_.host || !peerVideoLimit_)return;
  Bytes rate{'S','V','R','1'};u32(rate,std::min(videoBudget_.load(),videoCeiling()));
  transmit(snv2::PacketType::Keepalive,rate);
}

void Peer::input(const Input& input) {
  (void)queueInput(input);
}
bool Peer::gamepadAvailable() const {
  const auto at=peerGamepadAt_.load();
  const auto now=std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now().time_since_epoch()).count();
  return !options_.host && !options_.sharing && options_.input && ready_ && running_ && peerGamepad_ &&
      at>0 && now>=at && now-at<3000;
}
bool Peer::gamepad(const GamepadState& state,std::uint32_t expectedGeneration) {
  const auto bytes=encodeGamepad(state);if(bytes.empty() || !expectedGeneration)return false;
  std::lock_guard userLock(userMutex_);std::lock_guard sendLock(sendMutex_);
  if(!gamepadAvailable() || !inputAvailable() || remoteGeneration()!=expectedGeneration || sequences_[0]==UINT64_MAX)return false;
  snv2::Header header;header.packetType=snv2::PacketType::Gamepad;header.streamId=0;
  header.sequence=++sequences_[0];header.keyId=generation_;
  // Fence snapshots behind the latest reliable reset/key action. A late
  // pre-reset neutral sample must not rearm the next capture generation.
  header.frameId=sequences_[2];
  TxScheduler::Packet packet;packet.bytes=codec_.seal(header,bytes);packet.queued=Clock::now();packet.input=true;
  packet.deadline=packet.queued+std::chrono::milliseconds(100);
  tx_.latestGamepad(std::move(packet));sendReady_.notify_one();return true;
}
bool Peer::shortcut(std::span<const std::uint16_t> keys,std::uint32_t expectedGeneration) {
  if(!validShortcutKeys(keys) || !expectedGeneration)return false;
  return queueShortcut(keys,expectedGeneration);
}
bool Peer::releaseForShortcuts(std::uint32_t expectedGeneration) {
  return expectedGeneration && queueShortcut({},expectedGeneration);
}
bool Peer::releaseForController(std::uint32_t expectedGeneration) {
  return expectedGeneration && queueShortcut({},expectedGeneration,true);
}
bool Peer::queueShortcut(std::span<const std::uint16_t> keys,std::uint32_t expectedGeneration,bool controller) {
  std::lock_guard userLock(userMutex_);
  std::lock_guard sendLock(sendMutex_);
  const auto current=[&] {return (controller ? gamepadAvailable():textAvailable()) && inputAvailable() && remoteGeneration()==expectedGeneration;};
  const auto count=keys.empty() ? 1:keys.size()*2;
  if(!current() || pending_.size()+count>64 || sequences_[2]>UINT64_MAX-count)return false;
  std::vector<Input> events;
  if(keys.empty())events.push_back(Input{});
  for(auto key:keys)events.push_back(Input{Input::Key,0,0,key,0,true});
  for(auto it=keys.rbegin();it!=keys.rend();++it)events.push_back(Input{Input::Key,0,0,*it,0,false});
  // The panel releases input once on entry. Reset is a host queue fence, not
  // an ordinary key-up: resetting per action would erase an earlier queued
  // shortcut before a slow OS injector can execute it. Each action only has
  // balanced edges, so rapid Copy followed by Paste retains both operations.
  auto nextPending=pending_;std::vector<TxScheduler::Packet> packets;
  const auto now=Clock::now();auto sequence=sequences_[2];
  for(const auto& event:events) {
    snv2::Header header;header.packetType=snv2::PacketType::Keyboard;header.streamId=2;
    header.sequence=++sequence;header.flags=snv2::kFlagAcknowledgementRequired;header.keyId=generation_;
    auto bytes=codec_.seal(header,encodeInput(event));
    TxScheduler::Packet packet{bytes,now};packet.input=true;packet.inputSequence=header.sequence;
    packets.push_back(std::move(packet));
    nextPending.emplace(sequence,Pending{std::move(bytes),{},now,0,false,true,event.kind});
  }
  if(!current() || !tx_.pushInputBatch(std::move(packets)))return false;
  pending_.swap(nextPending);sequences_[2]=sequence;queuedKeys_+=keys.size()*2;
  sendReady_.notify_one();return true;
}
bool Peer::queueInput(const Input& input) {
  if (!running_ || !ready_ || !inputAvailable() || options_.host) return false;
  if(input.kind==Input::Gamepad)return false; // Only negotiated latest-state path.
  if(input.kind==Input::Text && !textAvailable()) return false;
  // Never reserve a reliable sequence for an unsupported OS key/event. A
  // rejected payload would otherwise leave an unfillable ordered input gap.
  const auto encoded=encodeInput(input);
  if(!decodeInput(encoded)) return false;
  if(input.kind==Input::Key) ++queuedKeys_;
  if(input.kind==Input::Button) ++queuedButtons_;
  const bool reliable=input.kind!=Input::Move;
  return transmit(reliable ? snv2::PacketType::Keyboard : snv2::PacketType::MouseMove,encoded,reliable ? 2 : 0,0,reliable ? snv2::kFlagAcknowledgementRequired : 0,reliable);
}
bool Peer::text(const std::string& utf8,std::uint32_t expectedGeneration) {
  std::lock_guard lock(userMutex_);
  if((expectedGeneration && expectedGeneration!=remoteGeneration()) || !textAvailable() || !sanser::textInputUtf16(utf8)) return false;
  Input edit; edit.kind=Input::Text; edit.text=utf8; return queueInput(edit);
}
bool Peer::clipboard(bool write,const std::string& utf8,ClipboardResult completed) {
  if(!clipboardAvailable() || !inputAvailable() || !completed || !sanser::clipboardUtf16(utf8) || (!write && !utf8.empty())) return false;
  std::lock_guard lock(userMutex_);
  if(clipboardPending_.size()>=4 || clipboardSerial_==UINT64_MAX) return false;
  const auto request=++clipboardSerial_;
  Bytes body{'S','C','B','1',static_cast<std::uint8_t>(write ? 1:2)};
  for(int i=7;i>=0;--i) body.push_back(static_cast<std::uint8_t>(request>>(i*8)));
  body.push_back(0); body.insert(body.end(),utf8.begin(),utf8.end());
  if(!userMessages_.queue(std::move(body))) return false;
  clipboardPending_.emplace(request,ClipboardPending{Clock::now(),std::move(completed)});return true;
}
void Peer::localClipboard(bool write,const std::string& utf8,ClipboardResult completed) {
  if(!completed)return;
  if(!localClipboardAvailable() || !sanser::clipboardUtf16(utf8) || (!write && !utf8.empty())) {completed(3,{});return;}
  std::uint64_t epoch;
  {std::unique_lock lock(userMailbox_->mutex);if(userMailbox_->localJobs>=4) {lock.unlock();completed(2,{});return;}
    ++userMailbox_->localJobs;epoch=userMailbox_->epoch;}
  const std::weak_ptr<UserMailbox> weak=userMailbox_;
  const auto current=[weak,epoch,deadline=Clock::now()+std::chrono::seconds(5)]{auto state=weak.lock();if(!state || Clock::now()>deadline)return false;std::lock_guard lock(state->mutex);return state->alive && state->epoch==epoch;};
  auto once=std::make_shared<std::atomic<bool>>(false);
  auto done=[weak,epoch,once,completed=std::move(completed)](unsigned status,std::string text) {
    if(once->exchange(true))return;
    auto state=weak.lock();bool valid=false;
    if(state){std::lock_guard lock(state->mutex);if(state->epoch==epoch){--state->localJobs;valid=state->alive;}}
    if(!valid) {completed(5,{});return;}
    if(status>5 || !sanser::clipboardUtf16(text)) {completed(4,{});return;}
    completed(status,status==0 ? std::move(text):std::string{});
  };
  try {onClipboard(write,utf8,current,done);}catch(...){done(3,{});}
}
bool Peer::fileMessage(const Bytes& body,std::uint32_t expectedGeneration) {
  if(!filesAvailable() || body.size()<21 || body.size()>8192 || !std::equal(body.begin(),body.begin()+4,"SFT1"))return false;
  std::lock_guard lock(userMutex_);
  if(!filesAvailable() || (expectedGeneration && expectedGeneration!=fileGeneration_))return false;
  return userMessages_.queue(body);
}
void Peer::processUserMessage(Bytes body) {
  if(body.size()>=21 && body.size()<=8192 && std::equal(body.begin(),body.begin()+4,"SFT1")) {
    if(filesAvailable()) {
      if(onFile)onFile(std::move(body));
      else if(std::getenv("SANSER_CONTROL_STDIN")) {const auto encoded=userHex(std::string_view(reinterpret_cast<const char*>(body.data()),body.size()));std::fprintf(stdout,"SNUSER_FILE_DATA\t%u\t%s\n",peerGeneration_,encoded.c_str());std::fflush(stdout);}
    }return;
  }
  if(body.size()<14 || body.size()>sanser::maxClipboardBytes+14 || !std::equal(body.begin(),body.begin()+4,"SCB1") || body[4]<1 || body[4]>3 || body[13]>5) return;
  std::uint64_t request=0;for(unsigned i=5;i<13;++i)request=(request<<8)|body[i];if(!request)return;
  std::string text(body.begin()+14,body.end());
  if(body[4]==3 && !options_.host) {
    ClipboardResult callback;
    { std::lock_guard lock(userMutex_);const auto found=clipboardPending_.find(request);if(found==clipboardPending_.end())return;
      callback=std::move(found->second.result);clipboardPending_.erase(found); }
    const bool valid=sanser::clipboardUtf16(text).has_value();
    callback(valid ? body[13]:4,valid && body[13]==0 ? std::move(text):std::string{});return;
  }
  if(!options_.host || body[4]==3 || body[13] || (body[4]==2 && !text.empty())) return;
  Bytes response(body.begin(),body.begin()+14);response[4]=3;
  const auto reject=[&](unsigned code) {
    response[13]=static_cast<std::uint8_t>(code);std::lock_guard lock(userMailbox_->mutex);
    if(userMailbox_->alive && userMailbox_->replies.size()+userMailbox_->hostJobs+userMailbox_->servicing<8)userMailbox_->replies.push_back(std::move(response));
  };
  if(!options_.clipboard || !options_.input || options_.sharing || localCapturePaused_ || !onClipboard) {reject(1);return;}
  if(!sanser::clipboardUtf16(text)) {reject(4);return;}
  const auto now=Clock::now();
  if(now-lastClipboardRequest_<std::chrono::milliseconds(100)) {reject(2);return;}
  std::uint64_t epoch;
  { std::unique_lock lock(userMailbox_->mutex);if(userMailbox_->hostJobs>=4 || userMailbox_->hostJobs+userMailbox_->replies.size()+userMailbox_->servicing>=8 || !userMailbox_->alive) {lock.unlock();reject(2);return;}
    epoch=userMailbox_->epoch;++userMailbox_->hostJobs; }
  lastClipboardRequest_=now;
  const std::weak_ptr<UserMailbox> weak=userMailbox_;
  const auto current=[weak,epoch,deadline=now+std::chrono::seconds(5)] { const auto state=weak.lock();if(!state || Clock::now()>deadline)return false;std::lock_guard lock(state->mutex);return state->alive && state->epoch==epoch; };
  auto once=std::make_shared<std::atomic<bool>>(false);
  auto done=[weak,epoch,response=std::move(response),once](unsigned status,std::string text) mutable {
    if(once->exchange(true))return;
    const auto state=weak.lock();if(!state)return;
    std::lock_guard lock(state->mutex);if(!state->alive || state->epoch!=epoch)return;
    --state->hostJobs;
    response[13]=static_cast<std::uint8_t>(status<=5 ? status:3);
    if(status==0 && !sanser::clipboardUtf16(text)) response[13]=4;
    if(response[13]==0) response.insert(response.end(),text.begin(),text.end());
    if(state->replies.size()<8)state->replies.push_back(std::move(response));
  };
  try { onClipboard(body[4]==1,std::move(text),current,done); } catch(...) {done(3,{});}
}
void Peer::serviceUserMessages(Clock::time_point now) {
  if(now-lastUserPoll_<std::chrono::milliseconds(16))return;lastUserPoll_=now;
  std::deque<Bytes> replies;
  { std::lock_guard lock(userMailbox_->mutex);replies.swap(userMailbox_->replies);userMailbox_->servicing=static_cast<unsigned>(replies.size()); }
  std::deque<Bytes> deferred;
  std::vector<Bytes> packets;std::vector<ClipboardResult> expired;
  { std::lock_guard lock(userMutex_);
    for(auto& body:replies)if(!userMessages_.queue(body,now))deferred.push_back(std::move(body));
    packets=userMessages_.poll(now,std::chrono::milliseconds(static_cast<unsigned>(std::clamp(controlRoundTripMs().value_or(30)*2+10,60.0,2000.0))));
    for(auto it=clipboardPending_.begin();it!=clipboardPending_.end();) {
      if(now-it->second.created>std::chrono::seconds(10)) {expired.push_back(std::move(it->second.result));it=clipboardPending_.erase(it);}else ++it;
    }
  }
  { std::lock_guard lock(userMailbox_->mutex);userMailbox_->servicing=0;
    if(userMailbox_->alive)while(!deferred.empty() && userMailbox_->replies.size()<8) {userMailbox_->replies.push_front(std::move(deferred.back()));deferred.pop_back();}
  }
  for(auto& packet:packets) {
    Clock::time_point deadline;
    {std::lock_guard lock(userMutex_);deadline=userMessages_.expires(packet);}
    transmit(snv2::PacketType::Clipboard,packet,0,0,0,false,deadline);
  }
  for(auto& callback:expired)callback(5,{});
}
void Peer::setCapturePaused(bool paused) {
  if(!options_.host || localCapturePaused_.exchange(paused)==paused) return;
  // Retry release on resume too: Secure Desktop may reject an earlier SendInput.
  if(inputWorker_ && !inputWorker_->submit(Input{})) { stopReason_=4; cancel(); }
}
void Peer::audio(const AudioSamples& samples) {
  if(externalVoiceActive() || !audioReady()) return;
  std::lock_guard lock(audioMutex_);
  if(peerAudioOpus_ && audioEncoder_) {
    const auto sequence=++audioSequence_;
    const auto packet=sanser::audio::pack({sequence,(sequence-1)*audioFrames,0,audioEncoder_->encode(samples)});
    transmit(snv2::PacketType::Audio,packet); ++opusTxPackets_; opusTxBytes_+=packet.size();
  } else transmit(snv2::PacketType::Audio,encodeAudio(samples));
}
void Peer::audioPrepared(const Bytes& packet,bool opus) {
  if(externalVoiceActive() || !audioReady() || opus!=opusAudioReady()) return;
  transmit(snv2::PacketType::Audio,packet);
  if(opus) { ++opusTxPackets_; opusTxBytes_+=packet.size(); }
}
void Peer::setAudioMuted(bool muted) {
  if(options_.host || !options_.audio) return;
  audioMuted_=muted;
  if(audioReceiver_) audioReceiver_->clear();
  if(onAudioReset) onAudioReset();
  transmit(snv2::PacketType::Keepalive,Bytes{'S','A','M','1',static_cast<std::uint8_t>(muted)});
}
void Peer::setVoiceState(voice::State state) {
  if(!options_.groupVoice) return;
  if(!state.joined) state.microphone=state.deafened=false;
  if(state.deafened) state.microphone=false;
  voiceJoined_=state.joined; voiceMic_=state.microphone; voiceDeaf_=state.deafened;
  { std::lock_guard lock(sendMutex_); if(!state.joined || !state.microphone) tx_.clearVoice(); }
  if(running_) transmit(snv2::PacketType::Keepalive,Bytes{'S','V','C','1',static_cast<std::uint8_t>(state.joined),static_cast<std::uint8_t>(state.microphone),static_cast<std::uint8_t>(state.deafened)});
}
bool Peer::voiceListening() const {
  const auto now=std::chrono::duration_cast<std::chrono::microseconds>(Clock::now().time_since_epoch()).count();
  return options_.groupVoice && running_ && ready_ && peerVoiceJoined_ && !peerVoiceDeaf_
    && peerVoiceAt_>0 && now-peerVoiceAt_.load()<2000000;
}
void Peer::voicePacket(voice::Packet packet) {
  if(!options_.groupVoice || !voiceJoined_ || !voiceMic_ || voiceDeaf_ || !ready_ || !running_) return;
  const auto now=std::chrono::duration_cast<std::chrono::microseconds>(Clock::now().time_since_epoch()).count();
  if(peerVoiceAt_==0 || now-peerVoiceAt_.load()>2000000) return;
  // A sharing host is an SFU: its own join preference is not a requirement for
  // forwarding viewer microphones. A viewer can only transmit its assigned ID.
  packet.speaker=voiceSlot_.load();
  if(packet.speaker>=voice::maxSpeakers) return;
  transmit(snv2::PacketType::Audio,voice::pack(packet));
}
void Peer::forwardVoice(const voice::Packet& packet) {
  if(!options_.host || !voiceListening()) return;
  transmit(snv2::PacketType::Audio,voice::pack(packet));
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
    peerGeneration_=h.keyId; peerFiles_=false;announcedFiles_=-1;
    if(options_.files && std::getenv("SANSER_CONTROL_STDIN")) {std::fprintf(stdout,"SNUSER_FILE_RESET\n");std::fflush(stdout);}
    peerSack_=false; peerAudio_=false; peerVideoProgress_=false; peerInputRecovery_=false; peerUnicode_=false; peerClipboard_=false;
    peerGamepad_=false;peerGamepadAt_=0;lastGamepad_=lastGamepadCapability_=0;
    { std::lock_guard lock(userMailbox_->mutex);++userMailbox_->epoch;userMailbox_->hostJobs=0;userMailbox_->localJobs=0;userMailbox_->servicing=0;userMailbox_->replies.clear(); }
    std::vector<ClipboardResult> cancelled;
    { std::lock_guard lock(userMutex_);fileGeneration_=h.keyId;userMessages_=UserMessages{};
      for(auto& [id,pending]:clipboardPending_) { (void)id;cancelled.push_back(std::move(pending.result)); }clipboardPending_.clear(); }
    { std::lock_guard lock(sendMutex_);tx_.clearUserData(); }
    for(auto& callback:cancelled)callback(5,{});
    receivedRecoverySerial_=receivedRecoveryFence_=0; inputLeaseArmed_=false;
    lastAudio_=0; lastAudioPreference_=0; receivingOpus_=false; peerAudioOpus_=false; peerAudioMuted_=false;
    peerVoiceJoined_=peerVoiceMic_=peerVoiceDeaf_=false; peerVoiceAt_=0; lastVoicePreference_=0;
    if(!options_.host) voiceSlot_=255;
    if(audioReceiver_) audioReceiver_->reset(h.keyId);
    if(onAudioReset) onAudioReset();
    { std::lock_guard lock(audioMutex_); audioSequence_=0; if(audioEncoder_) audioEncoder_->reset(); }
    controlProbe_.reset(); for(auto& window:replay_) window=ReplayWindow{};
    const auto receiveLimit=receiveVideoLimit_.load();
    reassembler_=Reassembler{receiveLimit ? std::min({options_.bitrate,receiveLimit,4000000U}):options_.bitrate,options_.ultraLowLatency,receiveLimit!=0};
    lastVideoRateSequence_=0;lastReportedBytes_=lastReportedLoss_=0; needKeyframe_=true; lastVideo_=0; lastMouse_=0; deliveredInput_=0; receivedInputs_.clear();
    controlAppRtt_=-1; minimumControlRtt_=0; inputRttUs_=inputRttSamples_=0;
    if(options_.host) {
      { std::lock_guard lock(videoMutex_); videoCache_.clear(); videoCacheBytes_=0; outgoingNeedsKeyframe_=true; }
      { std::lock_guard lock(feedbackMutex_); feedback_={}; }
      busyWindows_=congestionWindows_=recoveryWindows_=0;peerVideoLimit_=0;lastDelayFeedbackSequence_=0;videoBudget_=options_.bitrate;delayRate_=DelayRatePolicy{};
    }
    if(inputWorker_) inputWorker_->submit(Input{});
  }
  if (h.streamId==2 && h.sequence<=deliveredInput_) { acknowledgeInput(); return; }
  if (!replay_[h.streamId].accept(h.sequence)) { if (h.streamId==2) acknowledgeInput(); return; }
  lastReceived_=Clock::now();
  const bool becameReady=!ready_.exchange(true);
  if((becameReady || generationChanged) && h.packetType!=snv2::PacketType::Disconnect && onReady) onReady();
  if (h.packetType==snv2::PacketType::Keepalive && b==Bytes({'S','A','K','1'})) peerSack_=true;
  if (h.packetType==snv2::PacketType::Keepalive && b==Bytes({'S','F','T','1'})) peerFiles_=true;
  if(options_.groupVoice && h.packetType==snv2::PacketType::Keepalive && h.streamId==0) {
    if(b.size()==7 && !std::memcmp(b.data(),"SVC1",4) && b[4]<=1 && b[5]<=1 && b[6]<=1 && h.sequence>lastVoicePreference_) {
      lastVoicePreference_=h.sequence;
      peerVoiceJoined_=b[4]!=0; peerVoiceDeaf_=b[6]!=0;
      peerVoiceMic_=b[4] && b[5] && !b[6];
      peerVoiceAt_=std::chrono::duration_cast<std::chrono::microseconds>(Clock::now().time_since_epoch()).count();
      return;
    }
    if(!options_.host && b.size()==5 && !std::memcmp(b.data(),"SVI1",4) && b[4]>0 && b[4]<voice::maxSpeakers) { voiceSlot_=b[4]; return; }
  }
  if(h.packetType==snv2::PacketType::Keepalive && h.streamId==0) {
    if(!options_.host && b.size()==5 && std::memcmp(b.data(),"SGC1",4)==0 && b[4]<=1 && h.sequence>lastGamepadCapability_) {
      lastGamepadCapability_=h.sequence;peerGamepad_=b[4]!=0;
      peerGamepadAt_=std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now().time_since_epoch()).count();return;
    }
    if(!options_.host && b==Bytes({'S','U','T','1'})) { peerUnicode_=true; return; }
    if(!options_.host && b==Bytes({'S','C','B','1'})) { peerClipboard_=true; return; }
    if(b==Bytes({'S','I','C','1'})) { peerInputRecovery_=true; return; }
    if(!options_.host && b.size()==5 && std::memcmp(b.data(),"SCP1",4)==0 && b[4]<=1) {
      remoteCapturePaused_=b[4]!=0; return;
    }
    if(options_.host && peerInputRecovery_ && b==Bytes({'S','I','E','1'})) {
      inputLeaseAt_=Clock::now(); return;
    }
    if(peerInputRecovery_ && b.size()==20 &&
       ((options_.host && std::memcmp(b.data(),"SIR1",4)==0) || (!options_.host && std::memcmp(b.data(),"SIA1",4)==0))) {
      std::uint64_t serial=0,fence=0;
      for(std::size_t i=4;i<12;++i) serial=(serial<<8)|b[i];
      for(std::size_t i=12;i<20;++i) fence=(fence<<8)|b[i];
      if(options_.host) {
        if(serial<receivedRecoverySerial_ || !serial) return;
        if(serial!=receivedRecoverySerial_) {
          if(fence<deliveredInput_ || fence-deliveredInput_>64) return;
          if(inputWorker_ && !inputWorker_->submit(Input{})) { stopReason_=4; running_=false; return; }
          deliveredInput_=fence; receivedInputs_.clear();
          inputLeaseArmed_=false; receivedRecoverySerial_=serial; receivedRecoveryFence_=fence;
        } else if(fence!=receivedRecoveryFence_) return;
        auto response=b; response[2]='A'; transmit(snv2::PacketType::Keepalive,response);
      } else {
        std::lock_guard lock(sendMutex_);
        if(inputRecoveryPending_ && serial==inputRecoverySerial_ && fence==inputRecoveryFence_ &&
           Clock::now()-inputRecoveryStarted_<=std::chrono::seconds(5)) {
          inputRecoveryPending_=false; inputSuspended_=false;
          std::cerr<<"SNV2_INPUT_RECOVERED resetAcknowledged=1"<<std::endl;
        }
      }
      return;
    }
  }
  if(options_.host && h.streamId==0 && h.packetType==snv2::PacketType::Keepalive &&
     b.size()==8 && std::memcmp(b.data(),"SVB1",4)==0) {
    const auto limit=r32(b.data()+4);
    if(limit>=1000000 && limit<=50000000 && limit!=peerVideoLimit_) {
      peerVideoLimit_=limit;
      // Start below the ceiling and probe upward using real delivery feedback.
      videoBudget_=std::min({videoBudget_.load(),videoCeiling(),4000000U});
      announceVideoRate();
    }
    return;
  }
  if(!options_.host && receiveVideoLimit_ && h.streamId==0 && h.packetType==snv2::PacketType::Keepalive &&
     b.size()==8 && std::memcmp(b.data(),"SVR1",4)==0) {
    const auto rate=r32(b.data()+4);
    if(rate>=1000000 && rate<=receiveVideoLimit_ && h.sequence>lastVideoRateSequence_) {
      lastVideoRateSequence_=h.sequence;reassembler_.wireBitrate(rate);
    }
    return;
  }
  if(options_.host && peerVideoLimit_ && h.streamId==0 && h.packetType==snv2::PacketType::NetworkFeedback &&
     b.size()==12 && std::memcmp(b.data(),"SVL1",4)==0) {
    if(h.sequence<=lastDelayFeedbackSequence_) return;
    lastDelayFeedbackSequence_=h.sequence;
    const auto now=Clock::now();
    const double rtt=r32(b.data()+4)/1000.0,baseline=r32(b.data()+8)/1000.0;
    if(const auto target=delayRate_.observe(rtt,baseline,videoBudget_.load(),videoCeiling(),now)) {
      videoBudget_=*target;recoveryWindows_=0;announceVideoRate();
      if(std::getenv("SANSER_TELEMETRY")) { std::ostringstream line;line<<"SNV2_RATE_CONTROL rttMs="<<rtt<<" baselineMs="<<baseline<<" targetBps="<<*target;writeTelemetry(line); }
    }
    return;
  }
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
        minimumControlRtt_=minimumControlRtt_>0 ? std::min(minimumControlRtt_,*rtt):*rtt;
        const auto nominated=receiveBaselineMs_.load();
        const auto baseline=nominated>0 ? std::min(nominated,minimumControlRtt_):minimumControlRtt_;
        if(receiveVideoLimit_) {
          Bytes delay{'S','V','L','1'};u32(delay,static_cast<std::uint32_t>(std::clamp(*rtt*1000.0,1.0,3000000.0)));
          u32(delay,static_cast<std::uint32_t>(std::clamp(baseline*1000.0,1.0,3000000.0)));
          transmit(snv2::PacketType::NetworkFeedback,delay);
        }
        if(std::getenv("SANSER_TELEMETRY")) { std::ostringstream line; line<<"SNV2_CONTROL_TIMING appRttMs="<<*rtt; writeTelemetry(line); }
      }
      return;
    }
  }
  if (options_.host && h.streamId==0 && h.packetType==snv2::PacketType::Keepalive && b==Bytes({'S','A','P','1'})) peerAudio_=true;
  if(options_.host && options_.audio && options_.audioOpus && (audioEncoder_ || options_.sharedAudioEncoder) && h.streamId==0 && h.packetType==snv2::PacketType::Keepalive && b==Bytes({'S','A','O','1'})) { peerAudio_=true; peerAudioOpus_=true; }
  if(options_.host && h.streamId==0 && h.packetType==snv2::PacketType::Keepalive && b.size()==5 && std::equal(b.begin(),b.begin()+4,"SAM1") && b[4]<=1 && h.sequence>lastAudioPreference_) { lastAudioPreference_=h.sequence; peerAudioMuted_=b[4]!=0; }
  if(h.packetType==snv2::PacketType::Audio) {
    if(voice::isPacket(b)) {
      if(!options_.groupVoice || h.streamId!=0 || !onVoice) return;
      const auto now=Clock::now();
      if(now-voiceRateAt_>=std::chrono::seconds(1)) { voiceRateAt_=now; voiceRateCount_=0; }
      if(++voiceRateCount_>(options_.host ? 60u : 160u)) return; // Three 50pps sources at a viewer.
      if(auto packet=voice::unpack(b)) {
        if(options_.host) {
          if(!peerVoiceJoined_ || !peerVoiceMic_ || peerVoiceDeaf_ || packet->speaker!=options_.voiceSlot
              || std::chrono::duration_cast<std::chrono::microseconds>(now.time_since_epoch()).count()-peerVoiceAt_.load()>2000000) return;
        } else if(!voiceJoined_ || voiceDeaf_ || packet->speaker==voiceSlot_) return;
        onVoice(std::move(*packet));
      }
      return;
    }
    if(!options_.host && options_.audio && !audioMuted_ && h.streamId==0 && onAudio) {
      if(audioReceiver_ && b.size()>4 && std::equal(b.begin(),b.begin()+4,"SOA1")) {
        if(auto packet=sanser::audio::unpack(b)) {
          if(!receivingOpus_) { receivingOpus_=true; if(onAudioReset) onAudioReset(); }
          audioReceiver_->push(std::move(*packet),h.keyId);
        }
      } else if(!receivingOpus_ && h.sequence>lastAudio_) {
        if(auto samples=decodeAudio(b)) { lastAudio_=h.sequence; onAudio(std::move(*samples)); }
      }
    }
    return;
  }
  if (h.packetType==snv2::PacketType::Disconnect) {
    // Empty Disconnect remains a deliberate close for older peers. Optional
    // authenticated error reasons preserve reconnect instead of ending a share.
    stopReason_=(b.size()==1 && b[0]>=2 && b[0]<=5) ? b[0] : 1;
    running_=false; return;
  }
  if(h.packetType==snv2::PacketType::Clipboard && h.streamId==0) {
    UserMessages::Received received;
    { std::lock_guard lock(userMutex_);received=userMessages_.receive(b); }
    if(received.ack)transmit(snv2::PacketType::Clipboard,*received.ack);
    if(received.body)processUserMessage(std::move(*received.body));return;
  }
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
        videoBudget_=std::clamp(target,std::min(videoCeiling(),peerVideoLimit_ ? 1000000U:5000000U),videoCeiling());
        if(deliveryStalled && videoBudget_<previous && onKeyframe) onKeyframe();
        congestionWindows_=0;
      }
    } else {
      congestionWindows_=0;
      if(busy && !delayRate_.recovering(Clock::now()) && ++recoveryWindows_>=10) {
        videoBudget_=std::min(videoCeiling(),videoBudget_.load()+std::max(peerVideoLimit_ ? 200000U:500000U,videoBudget_.load()/20));
        recoveryWindows_=0;
      }
    }
    announceVideoRate();return;
  }
  // Optional host input timings. Older clients read its large prefix as an
  // impossible ACK and ignore it; the existing 8/16-byte ACK wire format stays.
  if(!options_.host && h.streamId==0 && h.packetType==snv2::PacketType::NetworkFeedback && b.size()==16 && std::memcmp(b.data(),"SIT1",4)==0) {
    if(std::getenv("SANSER_TELEMETRY")) {
      std::ostringstream line; line<<"SNV2_INPUT_HOST_TIMING hostQueueAvgMs="<<r32(b.data()+4)/1000.0
        <<" hostAvgWorkMs="<<r32(b.data()+8)/1000.0<<" hostQueueMaxMs="<<r32(b.data()+12)/1000.0;
      writeTelemetry(line);
    }
    return;
  }
  if (h.packetType==snv2::PacketType::NetworkFeedback && (b.size()==8 || b.size()==16)) {
    std::uint64_t ack=0, mask=0;
    for (std::size_t i=0;i<8;++i) ack=(ack<<8)|b[i];
    for (std::size_t i=8;i<b.size();++i) mask=(mask<<8)|b[i];
    std::lock_guard lock(sendMutex_);
    if (ack > sequences_[2]) return;
    lastInputAck_=std::max(lastInputAck_,ack);
    const auto now=Clock::now();
    const auto recordRtt=[&](const Pending& p) {
      if(p.sacked) return;
      inputRttUs_+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(now-p.created).count()); ++inputRttSamples_;
      // Karn's rule: a retransmitted ACK cannot identify which send it answers.
      if(p.attempts==1) inputDelivery_.acknowledged(std::chrono::duration_cast<std::chrono::milliseconds>(now-p.sent));
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
    tx_.acknowledgeInput(ack,mask);
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
        if(inputWorker_ && (!localCapturePaused_ || next.kind==Input::Reset)) {
          if(!inputWorker_->submit(next)) { stopReason_=4; running_=false; return; }
          inputLeaseArmed_=next.kind!=Input::Reset; inputLeaseAt_=Clock::now();
        }
        receivedInputs_.erase(deliveredInput_);
      }
      acknowledgeInput();
    } else if(event->kind==Input::Move && h.sequence>lastMouse_) {
      lastMouse_=h.sequence;
      if(inputWorker_ && !localCapturePaused_) inputWorker_->submit(*event);
    }
  }
  if(options_.host && h.packetType==snv2::PacketType::Gamepad && h.streamId==0 && h.sequence>lastGamepad_) {
    lastGamepad_=h.sequence;
    if(h.frameId!=deliveredInput_ || localCapturePaused_ || !inputWorker_ || !onGamepadAvailable || !onGamepadAvailable())return;
    if(const auto state=decodeGamepad(b)) {
      Input event;event.kind=Input::Gamepad;event.gamepad=*state;
      (void)inputWorker_->submit(std::move(event));
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
  (void)sanser::prioritizeMediaThread();
  auto creditAt=Clock::now();
  auto dataCreditAt=creditAt;
  double credit=8*1500;
  double dataCredit=1000;
  // Bitrate describes encoded video. Budget framing overhead separately.
  try {
    while(running_) {
      std::optional<TxScheduler::Packet> packet;
      {
        std::unique_lock lock(sendMutex_);
        const auto now=Clock::now();
        // The tighter mobile pacer is an explicitly negotiated policy. Preserve
        // desktop serialization/quality bounds until a receiver opts in.
        const double maxBurstBytes=(peerVideoLimit_ ? 2:8)*1500.0;
        const auto wireRate=peerVideoLimit_ ? videoBudget_.load():options_.bitrate;
        const double bytesPerSecond=static_cast<double>(std::max(1000000U,wireRate))*1.2/8.0;
        credit=std::min(maxBurstBytes, credit+std::chrono::duration<double>(now-creditAt).count()*bytesPerSecond);
        creditAt=now;
        const double dataRate=std::min(65536.0,bytesPerSecond*0.02);
        dataCredit=std::min(2000.0,dataCredit+std::chrono::duration<double>(now-dataCreditAt).count()*dataRate);
        dataCreditAt=now;
        const auto videoBytes=tx_.nextVideoBytes();
        const auto dataBytes=tx_.nextUserBytes();
        packet=tx_.pop(credit>=static_cast<double>(videoBytes),credit>=dataBytes && dataCredit>=dataBytes);
        if(!packet) {
          const auto nextVideo=now+std::chrono::microseconds(static_cast<std::uint64_t>(
            std::max(1.0,(static_cast<double>(videoBytes)-credit)*1000000.0/bytesPerSecond)));
          const auto nextData=now+std::chrono::microseconds(static_cast<std::uint64_t>(std::max({1.0,(dataBytes-credit)*1000000.0/bytesPerSecond,(dataBytes-dataCredit)*1000000.0/dataRate})));
          auto wake=now+std::chrono::milliseconds(20);if(tx_.hasVideo())wake=std::min(wake,nextVideo);if(tx_.hasUserData())wake=std::min(wake,nextData);
          sendReady_.wait_until(lock,wake,
            [&] { return !running_ || tx_.hasPriority() || (tx_.hasVideo() && Clock::now()>=nextVideo) || (tx_.hasUserData() && Clock::now()>=nextData); });
          continue;
        }
        if(packet->audio && now-packet->queued>std::chrono::milliseconds(40)) continue;
        if(packet->gamepad && now>packet->deadline)continue;
        if(packet->userData && packet->deadline!=Clock::time_point{} && now>packet->deadline) continue;
        if(packet->inputSequence) {
          const auto pending=pending_.find(packet->inputSequence);
          if(pending==pending_.end() || pending->second.sacked) continue;
          auto& input=pending->second;
          input.sent=now; input.queued=false;
          if(input.attempts++) ++inputRetryCount_;
        }
        if(packet->input) {
          inputSendQueueUs_+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(now-packet->queued).count());
          ++inputSendSamples_;
        }
        if (packet->frame && ((packet->deadline!=Clock::time_point{} && now>packet->deadline) ||
            (packet->deadline==Clock::time_point{} && now-packet->queued>std::chrono::milliseconds(100)))) {
          if(!packet->repair) { tx_.clearRealtimeVideo(); outgoingNeedsKeyframe_=true; ++videoQueueDrops_; }
          continue;
        }
      }
      datagram(packet->bytes);
      if(packet->originalSent)packet->originalSent->store(packet->originalOrdinal,std::memory_order_release);
      if (packet->frame || packet->userData) {
        credit-=static_cast<double>(packet->bytes.size());
      }
      if(packet->userData)dataCredit-=static_cast<double>(packet->bytes.size());
    }
  } catch (const std::exception& error) { std::cerr<<"SNV2_SEND_ERROR "<<error.what()<<std::endl; stopReason_=5; running_=false; }
}
void Peer::logHealth() {
  // Numeric snapshot for mobile diagnostics even when stdout tracing is off.
  // Update on the receive worker; callers never read reassembler state across
  // threads or block a realtime audio callback.
  const auto audioStats=audioReceiver_ ? audioReceiver_->stats():sanser::audio::Stats{};
  {
    std::lock_guard lock(healthMutex_);
    receiveHealth_={videoDatagrams_,completedFrames_,nackSent_,reassembler_.lostFragments(),keyframeWaitDropped_,
      audioStats.decoded,audioStats.concealed,audioStats.late};
  }
  if(!std::getenv("SANSER_TELEMETRY")) return;
  if(audioReceiver_ || opusTxPackets_) {
    const auto stats=audioReceiver_ ? audioReceiver_->stats() : sanser::audio::Stats{};
    std::ostringstream audio;
    audio<<"SNV2_AUDIO_HEALTH opusDecoded="<<stats.decoded<<" opusPlc="<<stats.concealed
      <<" opusLate="<<stats.late<<" opusReordered="<<stats.reordered<<" opusDropped="<<stats.dropped<<" opusErrors="<<stats.errors
      <<" opusTxPackets="<<opusTxPackets_.load()<<" opusTxBytes="<<opusTxBytes_.load(); writeTelemetry(audio);
  }
  std::size_t pending;
  std::uint64_t sendQueueUs,sendSamples,retries;
  { std::lock_guard lock(sendMutex_); pending=pending_.size(); sendQueueUs=inputSendQueueUs_; sendSamples=inputSendSamples_; retries=inputRetryCount_;
    inputSendQueueUs_=inputSendSamples_=inputRetryCount_=0; }
  const auto work=inputWorker_ ? inputWorker_->takeStats() : InputWorker::Stats{};
  if(work.samples && ready_ && running_) {
    Bytes timing{'S','I','T','1'};
    for(const auto us : {work.queueUs/work.samples,work.workUs/work.samples,work.queueMaxUs})
      u32(timing,static_cast<std::uint32_t>(std::min<std::uint64_t>(us,UINT32_MAX)));
    transmit(snv2::PacketType::NetworkFeedback,timing);
  }
  std::ostringstream line;
  line<<"SNV2_TRANSPORT_HEALTH ready="<<ready_.load()<<" running="<<running_.load()
    <<" rawDatagrams="<<rawDatagrams_<<" authRejected="<<authRejected_
    <<" videoDatagrams="<<videoDatagrams_<<" completed="<<completedFrames_
    <<" keyframeWaitDropped="<<keyframeWaitDropped_<<" nackSent="<<nackSent_
    <<" repaired="<<repairedDatagrams_.load()<<" repairCacheMisses="<<repairCacheMisses_.load()
    <<" repairUnsentIgnored="<<repairUnsentIgnored_.load()<<" videoBudgetBps="<<videoBudget_.load()<<" peerVideoLimitBps="<<peerVideoLimit_.load()
    <<" sendQueueDrops="<<videoQueueDrops_.load()<<" pending="<<pending<<" stopReason="<<stopReason_.load()
    <<" sendTransientErrors="<<sendTransientErrors_.load()<<" receiveTransientErrors="<<receiveTransientErrors_.load(); writeTelemetry(line);
  if(inputRttSamples_ || work.samples || sendSamples) {
    line.str(""); line.clear();
    line<<"SNV2_INPUT_TIMING";
    if(inputRttSamples_) line<<" avgRttMs="<<inputRttUs_/1000.0/inputRttSamples_;
    if(work.samples) line<<" hostAvgWorkMs="<<work.workUs/1000.0/work.samples<<" hostQueueAvgMs="<<work.queueUs/1000.0/work.samples
      <<" hostQueueMaxMs="<<work.queueMaxUs/1000.0;
    if(sendSamples) line<<" inputSendQueueAvgMs="<<sendQueueUs/1000.0/sendSamples;
    line<<" inputRetries="<<retries<<" coalescedMoves="<<work.coalescedMoves;
    writeTelemetry(line);
    inputRttUs_=inputRttSamples_=0;
  }
}
void Peer::loop() {
  (void)sanser::prioritizeMediaThread();
  auto lastHello=Clock::time_point{};
  auto lastControlProbe=Clock::time_point{};
  auto lastInputLog=Clock::now(); std::uint64_t lastInputCount=0;
  auto lastFeedback=Clock::now();
  auto lastHealth=Clock::now();
  try {
    while(running_ && !stopRequested) {
      // A sharing host's process-wide commands belong to HostOutput, never to
      // one of its per-viewer peers. A single host owns its file commands here.
      if(!options_.host || !options_.sharing) for(const auto& command:takeNativeCommands()) {
        if(command=="audio-muted\t1" || command=="audio-muted\t0") setAudioMuted(command.back()=='1');
        else if(const auto request=parseUserCommand(command)) {
          if(request->kind==UserCommand::FileMessage) {
            const auto bytes=userFileBody(request->body,peerGeneration_);
            if(bytes && !fileMessage(*bytes)) {std::fprintf(stdout,"SNUSER_FILE_FAIL\t%u\n",peerGeneration_);std::fflush(stdout);}
          }
          else if(request->kind==UserCommand::Text)userReply(request->request,text(request->body) ? 0:3);
          else if(request->kind==UserCommand::ReadLocalClipboard || request->kind==UserCommand::WriteLocalClipboard) {
            const auto id=request->request;
            localClipboard(request->kind==UserCommand::WriteLocalClipboard,request->body,[id](unsigned status,std::string body){userReply(id,status,body);});
          }
          else {
            const auto id=request->request;
            if(!clipboard(request->kind==UserCommand::WriteClipboard,request->body,[id](unsigned status,std::string body) {userReply(id,status,body);}))userReply(id,3);
          }
        }
      }
      const auto now=Clock::now();
      serviceUserMessages(now);
      if(options_.files && std::getenv("SANSER_CONTROL_STDIN")) {const int available=filesAvailable()?1:0;
        if(available!=announcedFiles_) {announcedFiles_=available;std::fprintf(stdout,"SNUSER_FILE_CAP\t%d\t%u\n",available,peerGeneration_);std::fflush(stdout);}
      }
      if(!options_.host && std::getenv("SANSER_CONTROL_STDIN")) {
        const int caps=(textAvailable()?1:0)|(clipboardAvailable()?2:0)|(localClipboardAvailable()?4:0);
        if(caps!=announcedUserCapabilities_) {announcedUserCapabilities_=caps;std::fprintf(stdout,"SNUSER_CAP\t%d\t%d\t%d\n",caps&1?1:0,caps&2?1:0,caps&4?1:0);std::fflush(stdout);}
      }
      if(!options_.host && ready_ && now-lastControlProbe>=std::chrono::milliseconds(receiveVideoLimit_ ? 250:1000)) {
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
        announceVideoRate();
        if(!options_.host && receiveVideoLimit_) {
          Bytes limit{'S','V','B','1'};u32(limit,receiveVideoLimit_.load());
          transmit(snv2::PacketType::Keepalive,limit);
        }
        if(options_.groupVoice) {
          transmit(snv2::PacketType::Keepalive,Bytes{'S','V','C','1',static_cast<std::uint8_t>(voiceJoined_.load()),static_cast<std::uint8_t>(voiceMic_.load()),static_cast<std::uint8_t>(voiceDeaf_.load())});
          if(options_.host && options_.voiceSlot>0 && options_.voiceSlot<voice::maxSpeakers)
            transmit(snv2::PacketType::Keepalive,Bytes{'S','V','I','1',static_cast<std::uint8_t>(options_.voiceSlot)});
        }
        if(options_.files && options_.input && !options_.sharing)transmit(snv2::PacketType::Keepalive,Bytes{'S','F','T','1'});
        transmit(snv2::PacketType::Keepalive,Bytes{'S','A','K','1'});
        transmit(snv2::PacketType::Keepalive,Bytes{'S','I','C','1'});
        if(options_.host && options_.input && !options_.sharing && onInput)
          transmit(snv2::PacketType::Keepalive,Bytes{'S','U','T','1'});
        if(options_.host && options_.input && !options_.sharing && options_.clipboard && onClipboard)
          transmit(snv2::PacketType::Keepalive,Bytes{'S','C','B','1'});
        if(options_.host)transmit(snv2::PacketType::Keepalive,Bytes{'S','G','C','1',
          static_cast<std::uint8_t>(options_.input && !options_.sharing && onGamepadAvailable && onGamepadAvailable())});
        if(options_.host) transmit(snv2::PacketType::Keepalive,Bytes{'S','C','P','1',static_cast<std::uint8_t>(localCapturePaused_.load())});
        if(!options_.host && inputAvailable()) transmit(snv2::PacketType::Keepalive,Bytes{'S','I','E','1'});
        // Optional authenticated capability. Old engines safely ignore it;
        // hosts never send audio until a receiving client explicitly opts in.
        if(!options_.host && options_.audio && onAudio) {
          transmit(snv2::PacketType::Keepalive,Bytes{'S','A','M','1',static_cast<std::uint8_t>(audioMuted_.load())});
          if(!audioMuted_) {
            transmit(snv2::PacketType::Keepalive,Bytes{'S','A','P','1'});
            if(audioReceiver_) transmit(snv2::PacketType::Keepalive,Bytes{'S','A','O','1'});
          }
        }
        lastHello=now;
      }
      if(options_.host && peerInputRecovery_ && inputLeaseArmed_ && now-inputLeaseAt_>std::chrono::seconds(2)) {
        // Even if every reset request is lost, a client that suspended input
        // stops renewing this lease. Release held actions without killing video.
        inputLeaseArmed_=false;
        if(inputWorker_ && !inputWorker_->submit(Input{})) { stopReason_=4; running_=false; break; }
        std::cerr<<"SNV2_INPUT_LEASE_EXPIRED action=release-held-input"<<std::endl;
      }
      if(inputRecoveryPending_) {
        if(now-inputRecoveryStarted_>std::chrono::seconds(5)) {
          inputRecoveryPending_=false;
          std::cerr<<"SNV2_INPUT_SUSPENDED resetAcknowledged=0 action=reconnect-to-restore-input"<<std::endl;
        } else if(now-inputRecoverySent_>=std::chrono::milliseconds(100)) {
          Bytes reset{'S','I','R','1'};
          for(auto value : {inputRecoverySerial_,inputRecoveryFence_})
            for(int i=7;i>=0;--i) reset.push_back(static_cast<std::uint8_t>(value>>(i*8)));
          transmit(snv2::PacketType::Keepalive,reset); inputRecoverySent_=now;
        }
      }
      // Ultra changes presentation latency, not whether a short Wi-Fi hiccup
      // destroys the window. Startup/NAT handoff keeps its normal 15s budget.
      if(now-lastReceived_>(ready_ && options_.ultraLowLatency ? std::chrono::seconds(5) : std::chrono::seconds(15))) { stopReason_=2; running_=false; break; }
      { std::lock_guard lock(sendMutex_);
        for(auto& [seq,p]:pending_) {
          (void)seq;
          const auto deadline=inputDelivery_.deadline();
          if(now-p.created>deadline) {
            std::fprintf(stderr,"SNV2_INPUT_ACK_TIMEOUT pending=%zu deadlineMs=%lld sequence=%llu kind=%u attempts=%u queued=%u lastAck=%llu ageMs=%lld\n",
              pending_.size(),static_cast<long long>(deadline.count()),static_cast<unsigned long long>(seq),static_cast<unsigned>(p.kind),p.attempts,static_cast<unsigned>(p.queued),
              static_cast<unsigned long long>(lastInputAck_),static_cast<long long>(std::chrono::duration_cast<std::chrono::milliseconds>(now-p.created).count()));
            if(peerInputRecovery_) {
              inputSuspended_=true; inputRecoveryPending_=true; inputRecoveryStarted_=now; inputRecoverySent_={};
              inputRecoveryFence_=sequences_[2]; ++inputRecoverySerial_;
              tx_.clearInput(); pending_.clear();
              std::cerr<<"SNV2_INPUT_SUSPENDED resetAcknowledged=0 action=authenticated-input-reset"<<std::endl;
            } else { stopReason_=3; running_=false; }
            break;
          }
          if(!p.sacked && !p.queued && now-p.sent>inputDelivery_.retry(options_.ultraLowLatency,p.attempts)) {
            TxScheduler::Packet retry{p.bytes,now,0}; retry.inputSequence=seq; retry.input=true;
            if (!tx_.push(TxScheduler::InputLane, std::move(retry))) { stopReason_=4; running_=false; break; }
            p.queued=true; sendReady_.notify_one();
          }
        }
      }
      std::array<std::uint8_t,1500> buffer{};
      const auto result=io_.receive(socket_,buffer.data(),buffer.size());
      const auto received=result.bytes;
      if(received<0) {
        const auto action=udp::receiveFailure(result.error);
        if(action==udp::ReceiveFailure::Fatal)
          throw std::runtime_error("UDP receive failed, socket error "+std::to_string(result.error));
        if(action==udp::ReceiveFailure::Retry) {
          ++receiveTransientErrors_;
          // Network errors can return immediately. Avoid a CPU spin while
          // retaining heartbeat, repair and authenticated peer-timeout budgets.
          std::this_thread::sleep_for(std::chrono::milliseconds(1));
        }
      }
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
  try {
    const auto reason=stopReason_.load();
    transmit(snv2::PacketType::Disconnect,reason>=2 ? Bytes{static_cast<std::uint8_t>(reason)} : Bytes{});
  } catch (...) {}
  running_=false; ready_=false; userMailbox_->alive=false; sendReady_.notify_all();
  std::vector<ClipboardResult> pendingUser;
  { std::lock_guard lock(userMutex_);for(auto& [id,pending]:clipboardPending_) {(void)id;pendingUser.push_back(std::move(pending.result));}clipboardPending_.clear(); }
  for(auto& callback:pendingUser)callback(5,{});
  logHealth();
  if(stopReason_>=2) std::cerr<<"SNV2_STOP reason="<<stopReason_.load()
    <<" (2=peer-timeout 3=input-ack-timeout 4=priority-queue-overflow 5=transport-error)"<<std::endl;
  if(inputWorker_) inputWorker_->stop();
  if(onStopped) onStopped();
}
}
