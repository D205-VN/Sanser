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
#ifdef _WIN32
#define NOMINMAX
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
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
constexpr std::size_t chunkSize = 1000, maxFrame = 4 * 1024 * 1024;
void u16(Bytes& b, std::uint16_t n) { b.push_back(static_cast<std::uint8_t>(n >> 8)); b.push_back(static_cast<std::uint8_t>(n)); }
void u32(Bytes& b, std::uint32_t n) { u16(b, static_cast<std::uint16_t>(n >> 16)); u16(b, static_cast<std::uint16_t>(n)); }
std::uint16_t r16(const std::uint8_t* p) { return static_cast<std::uint16_t>((p[0] << 8) | p[1]); }
std::uint32_t r32(const std::uint8_t* p) { return (static_cast<std::uint32_t>(r16(p)) << 16) | r16(p + 2); }
snv2::DirectionKey derive(const snv2::DirectionKey& key, std::string_view label) {
  return snv2::deriveKey(key, {reinterpret_cast<const std::uint8_t*>(label.data()), label.size()});
}
Bytes crypt(const snv2::DirectionKey& key, const std::uint8_t* iv, const Bytes& input, bool encrypt) {
  Bytes out(input.size() + 16);
#ifdef _WIN32
  BCRYPT_ALG_HANDLE alg = nullptr;
  BCRYPT_KEY_HANDLE handle = nullptr;
  if (BCryptOpenAlgorithmProvider(&alg, BCRYPT_AES_ALGORITHM, nullptr, 0) < 0) throw std::runtime_error("AES unavailable");
  auto status = BCryptSetProperty(alg, BCRYPT_CHAINING_MODE, reinterpret_cast<PUCHAR>(const_cast<wchar_t*>(BCRYPT_CHAIN_MODE_CBC)), sizeof(BCRYPT_CHAIN_MODE_CBC), 0);
  if (status >= 0) status = BCryptGenerateSymmetricKey(alg, &handle, nullptr, 0, const_cast<PUCHAR>(key.data()), static_cast<ULONG>(key.size()), 0);
  std::array<std::uint8_t, 16> mutableIv{}; std::copy_n(iv, 16, mutableIv.begin());
  ULONG size = 0;
  if (status >= 0) {
    const auto operation = encrypt ? BCryptEncrypt : BCryptDecrypt;
    status = operation(handle, const_cast<PUCHAR>(input.data()), static_cast<ULONG>(input.size()), nullptr, mutableIv.data(), 16, out.data(), static_cast<ULONG>(out.size()), &size, BCRYPT_BLOCK_PADDING);
  }
  if (handle) BCryptDestroyKey(handle);
  BCryptCloseAlgorithmProvider(alg, 0);
  if (status < 0) throw std::runtime_error("AES operation failed");
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
    std::thread([] { char line[16]; while(std::fgets(line,sizeof(line),stdin)) { if(std::string_view(line)=="stop\n") break; } stopRequested=true; }).detach();
  }
  const char* secret = std::getenv("SANSER_NATIVE_SESSION_TOKEN");
  if (secret) o.token = secret;
  if (o.token.size() < 32 || o.token.size() > 256) throw std::runtime_error("An authenticated session credential is required");
  for (int i = 1; i < argc; ++i) {
    const std::string arg = argv[i];
    auto value = [&]() -> std::string { if (++i >= argc) throw std::runtime_error("Missing engine argument"); return argv[i]; };
    auto number = [&]() -> std::uint32_t { const auto v = value(); std::size_t used = 0; const auto n = std::stoul(v, &used); if (used != v.size() || n > std::numeric_limits<std::uint32_t>::max()) throw std::runtime_error("Invalid numeric engine argument"); return static_cast<std::uint32_t>(n); };
    if (arg == "--snv2" || arg == "--udp-video" || arg == "--low-latency-encoder" || arg == "--udp-pacing") continue;
    if (arg == "--disable-input") o.input = false;
    else if (arg == "--disable-audio") continue;
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
  o.width &= ~1U; o.height &= ~1U;
  return o;
}
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
std::optional<Frame> Reassembler::push(const snv2::Header& h, const Bytes& b) {
  if (b.size() < 8 || h.frameId <= lastFrame_) return std::nullopt;
  const auto index = r16(b.data()), count = r16(b.data()+2); const auto size = r32(b.data()+4);
  if (size < 8 || size > maxFrame || count != (size+chunkSize-1)/chunkSize || index >= count) return std::nullopt;
  const auto offset = static_cast<std::size_t>(index)*chunkSize;
  if (b.size()-8 != std::min(chunkSize, size-offset)) return std::nullopt;
  const auto now = Clock::now();
  for (auto i = frames_.begin(); i != frames_.end();) { if (now-i->second.started > std::chrono::milliseconds(250)) i=frames_.erase(i); else ++i; }
  if (!frames_.contains(h.frameId)) { if (frames_.size() >= 4) frames_.erase(frames_.begin()); frames_.emplace(h.frameId, Partial{Bytes(size), std::vector<bool>(count), 0, now, (h.flags & snv2::kFlagKeyFrame) != 0}); }
  auto& p = frames_.at(h.frameId);
  if (p.data.size() != size || p.seen.size() != count || p.seen[index]) return std::nullopt;
  std::copy(b.begin()+8, b.end(), p.data.begin()+static_cast<std::ptrdiff_t>(offset)); p.seen[index]=true; ++p.received;
  if (p.received != count) return std::nullopt;
  Frame frame{{p.data.begin()+8, p.data.end()}, r32(p.data.data()), r32(p.data.data()+4), p.keyframe};
  frames_.erase(frames_.begin(), frames_.upper_bound(h.frameId)); lastFrame_=h.frameId;
  if (frame.width < 64 || frame.height < 64 || frame.width > 3840 || frame.height > 2160) return std::nullopt;
  return frame;
}
Peer::Peer(const Options& o) : options_(o), codec_(o.token, o.host) {
#ifdef _WIN32
  WSADATA data{}; if (WSAStartup(MAKEWORD(2,2), &data) != 0) throw std::runtime_error("Winsock unavailable");
#endif
  std::random_device random; do { generation_=random(); } while(generation_==0);
  const auto remote = endpoint(o.peer);
  const auto fd = ::socket(AF_INET, SOCK_DGRAM, 0); socket_ = static_cast<std::intptr_t>(fd);
  if (socket_ == -1) throw std::runtime_error("Unable to create media socket");
  sockaddr_in local{}; local.sin_family = AF_INET; local.sin_port = htons(o.port); local.sin_addr.s_addr = htonl(INADDR_ANY);
  if (::bind(fd, reinterpret_cast<const sockaddr*>(&local), sizeof(local)) != 0 || ::connect(fd, reinterpret_cast<const sockaddr*>(&remote), sizeof(remote)) != 0) { closeSocket(fd); socket_=-1; throw std::runtime_error("Unable to bind/connect native UDP socket"); }
#ifdef _WIN32
  DWORD timeout=100; setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,reinterpret_cast<const char*>(&timeout),sizeof(timeout));
#else
  timeval timeout{0,100000}; setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&timeout,sizeof(timeout));
#endif
  int buffer = 4*1024*1024; setsockopt(fd,SOL_SOCKET,SO_RCVBUF,reinterpret_cast<const char*>(&buffer),sizeof(buffer));
}
Peer::~Peer() { stop(); if (socket_ != -1) closeSocket(static_cast<Socket>(socket_));
#ifdef _WIN32
  WSACleanup();
#endif
}
void Peer::start() { if (running_.exchange(true)) return; lastReceived_=Clock::now(); worker_=std::thread([this] { loop(); }); }
void Peer::stop() { if (running_) { try { transmit(snv2::PacketType::Disconnect, {}); } catch (...) {} } running_=false; if (worker_.joinable()) worker_.join(); }
void Peer::datagram(const Bytes& b) { ::send(static_cast<Socket>(socket_), reinterpret_cast<const char*>(b.data()), static_cast<int>(b.size()), 0); }
void Peer::transmit(snv2::PacketType type,const Bytes& body,std::uint32_t stream,std::uint64_t frame,std::uint16_t flags,bool reliable) {
  std::lock_guard lock(sendMutex_);
  if (stream > 2 || (reliable && pending_.size() >= 64)) { running_=false; return; }
  snv2::Header h; h.packetType=type; h.streamId=stream; h.sequence=++sequences_[stream]; h.frameId=frame; h.flags=flags; h.keyId=generation_;
  auto b=codec_.seal(h,body); datagram(b);
  if (reliable) pending_.emplace(h.sequence, Pending{std::move(b),Clock::now(),1});
}
void Peer::video(const Frame& frame) {
  std::lock_guard lock(videoMutex_);
  if (!ready_ || !running_) return;
  Bytes bytes; u32(bytes,frame.width); u32(bytes,frame.height); bytes.insert(bytes.end(),frame.data.begin(),frame.data.end());
  if (bytes.size()>maxFrame) return;
  const auto id=++frame_; const auto count=(bytes.size()+chunkSize-1)/chunkSize;
  for (std::size_t i=0; i<count && running_; ++i) {
    Bytes fragment; u16(fragment,static_cast<std::uint16_t>(i)); u16(fragment,static_cast<std::uint16_t>(count)); u32(fragment,static_cast<std::uint32_t>(bytes.size()));
    const auto offset=i*chunkSize; fragment.insert(fragment.end(),bytes.begin()+static_cast<std::ptrdiff_t>(offset),bytes.begin()+static_cast<std::ptrdiff_t>(std::min(offset+chunkSize,bytes.size())));
    transmit(snv2::PacketType::Video,fragment,1,id,frame.keyframe ? snv2::kFlagKeyFrame : 0);
    if (i%8==7) std::this_thread::sleep_for(std::chrono::microseconds(150));
  }
}
void Peer::input(const Input& input) {
  if (!ready_ || !options_.input || options_.host) return;
  const bool reliable=input.kind!=Input::Move;
  transmit(reliable ? snv2::PacketType::Keyboard : snv2::PacketType::MouseMove,encodeInput(input),reliable ? 2 : 0,0,reliable ? snv2::kFlagAcknowledgementRequired : 0,reliable);
}
void Peer::requestKeyframe() {
  const auto now=Clock::now(); if (now-lastKeyframe_<std::chrono::milliseconds(300)) return;
  lastKeyframe_=now; transmit(snv2::PacketType::KeyframeRequest,{});
}
void Peer::process(const Bytes& bytes) {
  const auto packet=codec_.open(bytes); if (!packet) return;
  const auto& [h,b]=*packet; if (h.streamId>2 || h.keyId==0) return;
  if(h.keyId!=peerGeneration_) {
    if(previousGenerations_.contains(h.keyId)) return;
    if(previousGenerations_.size()>=16) { running_=false; return; }
    if(peerGeneration_) previousGenerations_.insert(peerGeneration_);
    peerGeneration_=h.keyId; for(auto& window:replay_) window=ReplayWindow{};
    reassembler_=Reassembler{}; needKeyframe_=true; lastVideo_=0; lastMouse_=0; deliveredInput_=0; receivedInputs_.clear();
    if(options_.host && onInput) onInput(Input{});
  }
  if (h.streamId==2 && h.sequence<=deliveredInput_) { Bytes ack; for(int i=7;i>=0;--i) ack.push_back(static_cast<std::uint8_t>(deliveredInput_>>(i*8))); transmit(snv2::PacketType::NetworkFeedback,ack); return; }
  if (!replay_[h.streamId].accept(h.sequence)) return;
  lastReceived_=Clock::now(); ready_=true;
  if (h.packetType==snv2::PacketType::Disconnect) { running_=false; return; }
  if (h.packetType==snv2::PacketType::NetworkFeedback && b.size()==8) { std::uint64_t ack=0; for(auto n:b) ack=(ack<<8)|n; std::lock_guard lock(sendMutex_); pending_.erase(pending_.begin(),pending_.upper_bound(ack)); return; }
  if (h.packetType==snv2::PacketType::KeyframeRequest && options_.host) { if(onKeyframe) onKeyframe(); return; }
  if (h.packetType==snv2::PacketType::Video && !options_.host && h.streamId==1) {
    auto frame=reassembler_.push(h,b);
    if(frame) {
      if(lastVideo_ && h.frameId!=lastVideo_+1 && !frame->keyframe) needKeyframe_=true;
      lastVideo_=h.frameId;
      if(!needKeyframe_ || frame->keyframe) { needKeyframe_=false; if(onFrame) onFrame(std::move(*frame)); }
    }
    if (needKeyframe_) requestKeyframe(); return;
  }
  if (options_.host && options_.input && (h.packetType==snv2::PacketType::Keyboard || h.packetType==snv2::PacketType::MouseMove)) {
    auto event=decodeInput(b); if(!event) return;
    if(h.streamId==2) {
      if(h.sequence>deliveredInput_+64) { running_=false; return; }
      receivedInputs_.emplace(h.sequence,*event);
      while(receivedInputs_.contains(deliveredInput_+1)) { const auto next=receivedInputs_.at(++deliveredInput_); if(onInput) onInput(next); receivedInputs_.erase(deliveredInput_); }
      Bytes ack; for(int i=7;i>=0;--i) ack.push_back(static_cast<std::uint8_t>(deliveredInput_>>(i*8))); transmit(snv2::PacketType::NetworkFeedback,ack);
    } else if(event->kind==Input::Move && h.sequence>lastMouse_) {
      lastMouse_=h.sequence;
      if(onInput) onInput(*event);
    }
  }
}
void Peer::loop() {
  auto lastHello=Clock::time_point{};
  try {
    while(running_ && !stopRequested) {
      const auto now=Clock::now();
      if(now-lastHello>std::chrono::milliseconds(500)) { transmit(snv2::PacketType::Keepalive,{}); lastHello=now; }
      if(now-lastReceived_>std::chrono::seconds(15)) { running_=false; break; }
      { std::lock_guard lock(sendMutex_); for(auto& [seq,p]:pending_) { (void)seq; if(now-p.sent>std::chrono::milliseconds(80)) { if(++p.attempts>20) { running_=false; break; } datagram(p.bytes); p.sent=now; } } }
      std::array<char,1500> buffer{}; const auto received=recv(static_cast<Socket>(socket_),buffer.data(),static_cast<int>(buffer.size()),0);
      if(received>0) process(Bytes(buffer.begin(),buffer.begin()+received));
    }
  } catch (...) { running_=false; }
  running_=false; ready_=false;
  if(options_.host && onInput) onInput(Input{});
  if(onStopped) onStopped();
}
}
