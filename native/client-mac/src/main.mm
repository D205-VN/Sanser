#import <Cocoa/Cocoa.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <CoreGraphics/CoreGraphics.h>
#import <Metal/Metal.h>
#import <MetalKit/MetalKit.h>
#import <VideoToolbox/VideoToolbox.h>
#import <AudioToolbox/AudioToolbox.h>
#import <CommonCrypto/CommonDigest.h>
#import <CommonCrypto/CommonHMAC.h>
#import <dispatch/dispatch.h>

#include <chrono>
#include <algorithm>
#include <array>
#include <atomic>
#include <cerrno>
#include <condition_variable>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <cmath>
#include <deque>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <map>
#include <memory>
#include <mutex>
#include <sstream>
#include <stdexcept>
#include <string>
#include <thread>
#include <utility>
#include <vector>

#include <arpa/inet.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <unistd.h>

namespace {

CMVideoCodecType fourcc(char a, char b, char c, char d) {
  return (static_cast<CMVideoCodecType>(a) << 24)
       | (static_cast<CMVideoCodecType>(b) << 16)
       | (static_cast<CMVideoCodecType>(c) << 8)
       | static_cast<CMVideoCodecType>(d);
}

std::string jsonEscape(const char* value) {
  std::ostringstream escaped;
  for (const char* cursor = value ? value : ""; *cursor; ++cursor) {
    const char ch = *cursor;
    switch (ch) {
      case '\\': escaped << "\\\\"; break;
      case '"': escaped << "\\\""; break;
      case '\n': escaped << "\\n"; break;
      case '\r': escaped << "\\r"; break;
      case '\t': escaped << "\\t"; break;
      default: escaped << ch; break;
    }
  }
  return escaped.str();
}

std::string nsStringToUtf8(NSString* value) {
  return value ? std::string([value UTF8String] ?: "") : "";
}

std::string boolText(bool value) {
  return value ? "yes" : "no";
}

id<MTLDevice> defaultMetalDevice() {
  id<MTLDevice> device = MTLCreateSystemDefaultDevice();
  if (device) return device;

  NSArray<id<MTLDevice>>* devices = MTLCopyAllDevices();
  if ([devices count] > 0) return [devices objectAtIndex:0];
  return nil;
}

void printMetalDevices() {
  NSArray<id<MTLDevice>>* devices = MTLCopyAllDevices();
  std::cout << "Metal devices found: " << [devices count] << "\n";
  for (id<MTLDevice> device in devices) {
    std::cout << "  - " << nsStringToUtf8([device name]) << "\n";
  }
}

void printHelp() {
  std::cout
    << "sanser-native-client --probe\n"
    << "sanser-native-client --decode-snv capture_h264.snv [--max-packets N]\n"
    << "sanser-native-client --listen-snv PORT [--max-packets N]\n"
    << "sanser-native-client --listen-render-snv PORT [--control-port PORT] [--audio-port PORT] [--audio-jitter-ms N] [--max-packets N] [--session-token TOKEN] [--log-input] [--fullscreen] [--hide-cursor] [--relative-mouse]\n"
    << "sanser-native-client --metal-test [--seconds 5]\n"
    << "sanser-native-client --clipboard-read\n"
    << "sanser-native-client --clipboard-write \"text\"\n"
    << "\n"
    << "Phase 5 prototype:\n"
    << "  --probe           Print VideoToolbox hardware decode and Metal availability\n"
    << "  --decode-snv P    Decode an SNV1 H.264 packet file with VideoToolbox\n"
    << "  --listen-snv P    Listen for SNV1 H.264 packets over TCP and decode them\n"
    << "  --listen-render-snv P Listen over TCP, render with Metal, send native input back\n"
    << "  --udp-video      Listen for SNU1/SNU2 UDP video instead of TCP video\n"
    << "  --session-token T Require host control proof and HMAC-authenticated control packets\n"
    << "  --control-port P  Dedicated native input/stats TCP port; defaults to render port + 1, 0 disables\n"
    << "  --audio-port P   Listen for SNA1/SNA2 UDP float PCM audio; defaults to render port + 2, 0 disables\n"
    << "  --audio-jitter-ms N Target SNA1 audio jitter buffer, default 24 ms\n"
    << "  --max-packets N   Stop SNV decode after N packets\n"
    << "  --log-input       Print each native input event for debugging\n"
    << "  --fullscreen      Open the Metal renderer fullscreen\n"
    << "  --hide-cursor     Hide the local cursor while the renderer is open\n"
    << "  --relative-mouse  Grab macOS mouse movement and send relative dx/dy input\n"
    << "  --metal-test      Open a Metal window and log native input events to stdout\n"
    << "  --seconds N       Auto-close Metal test after N seconds; 0 keeps it open\n"
    << "  --clipboard-read  Print macOS pasteboard text\n"
    << "  --clipboard-write Write text into macOS pasteboard\n";
}

void printCodecSupport(const char* label, CMVideoCodecType codec) {
  bool supported = false;
  if (@available(macOS 10.13, *)) {
    supported = VTIsHardwareDecodeSupported(codec);
  }
  std::cout << "VideoToolbox " << label << " hardware decode: " << boolText(supported) << "\n";
}

int runProbe() {
  @autoreleasepool {
    id<MTLDevice> device = defaultMetalDevice();
    std::cout << "Sanser native mac client probe\n";
    std::cout << "Metal device: " << (device ? nsStringToUtf8([device name]) : "unavailable") << "\n";
    printMetalDevices();
    printCodecSupport("H.264", kCMVideoCodecType_H264);
    printCodecSupport("H.265/HEVC", kCMVideoCodecType_HEVC);
    printCodecSupport("AV1", fourcc('a', 'v', '0', '1'));
  }
  return 0;
}

int clipboardRead() {
  @autoreleasepool {
    NSString* value = [[NSPasteboard generalPasteboard] stringForType:NSPasteboardTypeString];
    if (value) std::cout << [value UTF8String] << "\n";
  }
  return 0;
}

int clipboardWrite(const std::string& value) {
  @autoreleasepool {
    NSPasteboard* pasteboard = [NSPasteboard generalPasteboard];
    [pasteboard clearContents];
    NSString* text = [NSString stringWithUTF8String:value.c_str()];
    if (![pasteboard setString:text forType:NSPasteboardTypeString]) {
      throw std::runtime_error("Failed to write text to macOS pasteboard.");
    }
  }
  return 0;
}

double secondsSince(std::chrono::steady_clock::time_point start) {
  const auto now = std::chrono::steady_clock::now();
  return std::chrono::duration<double>(now - start).count();
}

std::uint64_t steadyMicros() {
  const auto now = std::chrono::steady_clock::now().time_since_epoch();
  return static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(now).count());
}

std::uint64_t unixMicros() {
  const auto now = std::chrono::system_clock::now().time_since_epoch();
  return static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(now).count());
}

void writeLe32Raw(std::uint8_t* target, std::uint32_t value) {
  target[0] = static_cast<std::uint8_t>(value & 0xff);
  target[1] = static_cast<std::uint8_t>((value >> 8) & 0xff);
  target[2] = static_cast<std::uint8_t>((value >> 16) & 0xff);
  target[3] = static_cast<std::uint8_t>((value >> 24) & 0xff);
}

std::string bytesToHex(const unsigned char* bytes, std::size_t size) {
  static constexpr char digits[] = "0123456789abcdef";
  std::string out;
  out.resize(size * 2);
  for (std::size_t i = 0; i < size; ++i) {
    out[i * 2] = digits[(bytes[i] >> 4) & 0x0f];
    out[i * 2 + 1] = digits[bytes[i] & 0x0f];
  }
  return out;
}

bool constantTimeEqual(const std::string& left, const std::string& right) {
  if (left.size() != right.size()) return false;
  unsigned char diff = 0;
  for (std::size_t i = 0; i < left.size(); ++i) {
    diff |= static_cast<unsigned char>(left[i] ^ right[i]);
  }
  return diff == 0;
}

std::string hmacSha256Hex(const std::string& key, const std::string& message) {
  unsigned char digest[CC_SHA256_DIGEST_LENGTH]{};
  CCHmac(kCCHmacAlgSHA256,
         key.data(),
         key.size(),
         message.data(),
         message.size(),
         digest);
  return bytesToHex(digest, sizeof(digest));
}

std::string randomHex(std::size_t bytes) {
  std::vector<unsigned char> data(bytes);
  if (!data.empty()) arc4random_buf(data.data(), data.size());
  return bytesToHex(data.data(), data.size());
}

std::string controlAuthMac(const std::string& token,
                           const char* direction,
                           std::uint64_t sequence,
                           const std::string& payload) {
  std::ostringstream message;
  message << "sanser-control-v1|" << direction << "|" << sequence << "|" << payload;
  return hmacSha256Hex(token, message.str());
}

std::string controlSessionProof(const std::string& token, const std::string& nonce) {
  return hmacSha256Hex(token, "sanser-control-proof-v1|" + nonce + "|2");
}

std::string makeSecureControlEnvelope(const std::string& token,
                                      const char* direction,
                                      std::uint64_t sequence,
                                      const std::string& payload) {
  const std::string mac = controlAuthMac(token, direction, sequence, payload);
  std::ostringstream out;
  out << "{\"type\":\"secure-control\""
      << ",\"authVersion\":1"
      << ",\"authSeq\":" << sequence
      << ",\"payload\":\"" << jsonEscape(payload.c_str()) << "\""
      << ",\"authMac\":\"" << mac << "\""
      << "}";
  return out.str();
}

static constexpr std::size_t kUdpAuthTagHexSize = 32;

std::string udpDatagramMacHex(const std::string& token,
                              const char* media,
                              const std::uint8_t* datagram,
                              std::size_t datagramSize) {
  std::string message = std::string("sanser-udp-v1|") + media + "|";
  message.append(reinterpret_cast<const char*>(datagram), datagramSize);
  return hmacSha256Hex(token, message).substr(0, kUdpAuthTagHexSize);
}

bool verifyUdpDatagramMac(const std::string& token,
                          const char* media,
                          std::vector<std::uint8_t> datagram,
                          std::size_t tagOffset) {
  if (token.empty() || tagOffset + kUdpAuthTagHexSize > datagram.size()) return false;
  const std::string received(reinterpret_cast<const char*>(datagram.data() + tagOffset), kUdpAuthTagHexSize);
  std::memset(datagram.data() + tagOffset, 0, kUdpAuthTagHexSize);
  const std::string expected = udpDatagramMacHex(token, media, datagram.data(), datagram.size());
  return constantTimeEqual(received, expected);
}

std::uint8_t hexNibble(char ch) {
  if (ch >= '0' && ch <= '9') return static_cast<std::uint8_t>(ch - '0');
  if (ch >= 'a' && ch <= 'f') return static_cast<std::uint8_t>(ch - 'a' + 10);
  if (ch >= 'A' && ch <= 'F') return static_cast<std::uint8_t>(ch - 'A' + 10);
  return 0;
}

std::array<std::uint8_t, 32> hmacSha256Bytes(const std::string& key, const std::string& message) {
  const std::string hex = hmacSha256Hex(key, message);
  std::array<std::uint8_t, 32> bytes{};
  for (std::size_t i = 0; i < bytes.size() && i * 2 + 1 < hex.size(); ++i) {
    bytes[i] = static_cast<std::uint8_t>((hexNibble(hex[i * 2]) << 4) | hexNibble(hex[i * 2 + 1]));
  }
  return bytes;
}

std::string mediaEpochMaterial(std::uint64_t epoch, const std::string& nonce) {
  std::ostringstream out;
  out << epoch << "|" << nonce;
  return out.str();
}

std::array<std::uint8_t, 32> deriveMediaCryptoKey(const std::string& token,
                                                  const char* media,
                                                  std::uint64_t epoch,
                                                  const std::string& nonce) {
  return hmacSha256Bytes(token, std::string("sanser-media-chacha20-key-v2|") + media + "|" + mediaEpochMaterial(epoch, nonce));
}

std::string deriveMediaAuthKey(const std::string& token,
                               const char* media,
                               std::uint64_t epoch,
                               const std::string& nonce) {
  return hmacSha256Hex(token, std::string("sanser-media-auth-key-v2|") + media + "|" + mediaEpochMaterial(epoch, nonce));
}

std::uint32_t loadLe32(const std::uint8_t* data) {
  return static_cast<std::uint32_t>(data[0])
       | (static_cast<std::uint32_t>(data[1]) << 8)
       | (static_cast<std::uint32_t>(data[2]) << 16)
       | (static_cast<std::uint32_t>(data[3]) << 24);
}

void storeLe32(std::uint8_t* target, std::uint32_t value) {
  target[0] = static_cast<std::uint8_t>(value & 0xff);
  target[1] = static_cast<std::uint8_t>((value >> 8) & 0xff);
  target[2] = static_cast<std::uint8_t>((value >> 16) & 0xff);
  target[3] = static_cast<std::uint8_t>((value >> 24) & 0xff);
}

std::uint32_t rotateLeft32(std::uint32_t value, int bits) {
  return (value << bits) | (value >> (32 - bits));
}

void chachaQuarterRound(std::uint32_t& a,
                        std::uint32_t& b,
                        std::uint32_t& c,
                        std::uint32_t& d) {
  a += b; d ^= a; d = rotateLeft32(d, 16);
  c += d; b ^= c; b = rotateLeft32(b, 12);
  a += b; d ^= a; d = rotateLeft32(d, 8);
  c += d; b ^= c; b = rotateLeft32(b, 7);
}

void chacha20Block(const std::array<std::uint8_t, 32>& key,
                   std::uint32_t counter,
                   const std::array<std::uint8_t, 12>& nonce,
                   std::uint8_t out[64]) {
  const std::uint32_t constants[4] = {
    0x61707865u, 0x3320646eu, 0x79622d32u, 0x6b206574u
  };
  std::uint32_t state[16]{};
  state[0] = constants[0];
  state[1] = constants[1];
  state[2] = constants[2];
  state[3] = constants[3];
  for (int i = 0; i < 8; ++i) {
    state[4 + i] = loadLe32(key.data() + i * 4);
  }
  state[12] = counter;
  state[13] = loadLe32(nonce.data());
  state[14] = loadLe32(nonce.data() + 4);
  state[15] = loadLe32(nonce.data() + 8);

  std::uint32_t working[16]{};
  std::copy(std::begin(state), std::end(state), std::begin(working));
  for (int round = 0; round < 10; ++round) {
    chachaQuarterRound(working[0], working[4], working[8], working[12]);
    chachaQuarterRound(working[1], working[5], working[9], working[13]);
    chachaQuarterRound(working[2], working[6], working[10], working[14]);
    chachaQuarterRound(working[3], working[7], working[11], working[15]);
    chachaQuarterRound(working[0], working[5], working[10], working[15]);
    chachaQuarterRound(working[1], working[6], working[11], working[12]);
    chachaQuarterRound(working[2], working[7], working[8], working[13]);
    chachaQuarterRound(working[3], working[4], working[9], working[14]);
  }
  for (int i = 0; i < 16; ++i) {
    storeLe32(out + i * 4, working[i] + state[i]);
  }
}

std::array<std::uint8_t, 12> mediaNonce(const char* media, std::uint64_t sequence) {
  std::array<std::uint8_t, 12> nonce{};
  const std::uint32_t domain = std::string(media) == "audio" ? 0x32414e53u : 0x32554e53u;
  storeLe32(nonce.data(), domain);
  for (int i = 0; i < 8; ++i) {
    nonce[4 + i] = static_cast<std::uint8_t>((sequence >> (8 * i)) & 0xff);
  }
  return nonce;
}

void chacha20Xor(std::uint8_t* data,
                 std::size_t size,
                 const std::array<std::uint8_t, 32>& key,
                 const char* media,
                 std::uint64_t sequence) {
  if (!data || size == 0) return;
  const auto nonce = mediaNonce(media, sequence);
  std::uint32_t counter = 1;
  std::size_t offset = 0;
  while (offset < size) {
    std::uint8_t block[64]{};
    chacha20Block(key, counter++, nonce, block);
    const std::size_t chunk = std::min<std::size_t>(sizeof(block), size - offset);
    for (std::size_t i = 0; i < chunk; ++i) {
      data[offset + i] ^= block[i];
    }
    offset += chunk;
  }
}

class NativeInputSender {
public:
  NativeInputSender() : inputSessionId_(randomHex(8)), worker_(&NativeInputSender::workerLoop, this) {}

  ~NativeInputSender() {
    {
      std::lock_guard<std::mutex> lock(mutex_);
      stopped_ = true;
      queue_.clear();
      pendingBatches_.clear();
    }
    condition_.notify_all();
    if (worker_.joinable()) {
      worker_.join();
    }
  }

  void setFd(int fd) {
    std::lock_guard<std::mutex> lock(mutex_);
    fd_ = fd;
    fdGeneration_ += 1;
    queue_.clear();
    pausePendingBatchesLocked("fd-set");
    batchEnabled_ = false;
    controlAuthenticated_ = !authRequired_;
    packetAuthEnabled_ = false;
    packetAuthSequence_ = 0;
    condition_.notify_all();
  }

  void clearFd(int fd) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (fd_ == fd) {
      fd_ = -1;
      fdGeneration_ += 1;
      queue_.clear();
      pausePendingBatchesLocked("fd-clear");
      controlAuthenticated_ = !authRequired_;
      packetAuthEnabled_ = false;
      packetAuthSequence_ = 0;
    }
    condition_.notify_all();
  }

  void setAuthRequired(bool required, const std::string& packetAuthToken = {}) {
    std::lock_guard<std::mutex> lock(mutex_);
    authRequired_ = required;
    packetAuthToken_ = packetAuthToken;
    controlAuthenticated_ = !authRequired_;
    packetAuthEnabled_ = false;
    packetAuthSequence_ = 0;
    batchEnabled_ = false;
    queue_.clear();
    pendingBatches_.clear();
  }

  void setControlAuthenticated(bool authenticated, bool preservePending = true) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (controlAuthenticated_ == authenticated) {
      if (!authenticated && !preservePending) {
        pendingBatches_.clear();
      }
      return;
    }
    controlAuthenticated_ = authenticated;
    if (!controlAuthenticated_) {
      batchEnabled_ = false;
      packetAuthEnabled_ = false;
      queue_.clear();
      if (preservePending) {
        pausePendingBatchesLocked("auth-blocked");
      } else {
        pendingBatches_.clear();
      }
    }
    std::cout << "SNCONTROL sessionAuth=" << (controlAuthenticated_ ? "authenticated" : "blocked") << "\n";
    condition_.notify_all();
  }

  void setPacketAuthEnabled(bool enabled) {
    std::lock_guard<std::mutex> lock(mutex_);
    const bool next = enabled && !packetAuthToken_.empty();
    if (packetAuthEnabled_ == next) return;
    packetAuthEnabled_ = next;
    packetAuthSequence_ = 0;
    std::cout << "SNCONTROL packetAuth=" << (packetAuthEnabled_ ? "enabled" : "disabled") << "\n";
  }

  void setBatchEnabled(bool enabled) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (authRequired_ && !controlAuthenticated_) enabled = false;
    if (batchEnabled_ == enabled) return;
    batchEnabled_ = enabled;
    std::cout << "SNCONTROL inputBatch=" << (enabled ? "enabled" : "disabled") << "\n";
    condition_.notify_all();
  }

  std::string inputSessionId() const {
    return inputSessionId_;
  }

  bool sendJson(const std::string& json) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (fd_ < 0 || stopped_) return false;
    if (authRequired_ && !controlAuthenticated_ && !isAllowedBeforeAuth(json)) return false;
    const bool isMove = isPointerMove(json);
    if (isMove && !queue_.empty() && isPointerMove(queue_.back())) {
      queue_.back() = json;
    } else {
      while (queue_.size() >= maxQueueSize_) {
        if (!dropOldestPointerMoveLocked()) {
          queue_.pop_front();
        }
      }
      queue_.push_back(json);
    }
    condition_.notify_one();
    return true;
  }

  void observeInputAck(std::uint64_t sequence, std::uint64_t applied) {
    if (sequence == 0) return;
    std::lock_guard<std::mutex> lock(mutex_);
    auto it = pendingBatches_.find(sequence);
    if (it == pendingBatches_.end()) return;
    const std::uint32_t attempts = it->second.attempts;
    pendingBatches_.erase(it);
    if (attempts > 1 || sequence == 1 || sequence % 120 == 0) {
      std::cout << "SNINPUT_ACKED sequence=" << sequence
                << " applied=" << applied
                << " attempts=" << attempts
                << " pending=" << pendingBatches_.size()
                << "\n";
    }
    condition_.notify_one();
  }

private:
  struct PendingBatch {
    std::string json;
    std::size_t events = 0;
    std::uint32_t attempts = 0;
    std::chrono::steady_clock::time_point lastSentAt;
  };

  static bool isPointerMove(const std::string& json) {
    return json.find("\"type\":\"pointer-move\"") != std::string::npos;
  }

  static bool isControlPing(const std::string& json) {
    return json.find("\"type\":\"control-ping\"") != std::string::npos;
  }

  static bool isStreamStats(const std::string& json) {
    return json.find("\"type\":\"stream-stats\"") != std::string::npos;
  }

  static bool isUdpRepairStats(const std::string& json) {
    return json.find("\"type\":\"udp-repair-stats\"") != std::string::npos;
  }

  static bool isRenderStats(const std::string& json) {
    return json.find("\"source\":\"render\"") != std::string::npos;
  }

  static bool isKeyframeRequest(const std::string& json) {
    return json.find("\"type\":\"keyframe-request\"") != std::string::npos;
  }

  static bool isVideoNack(const std::string& json) {
    return json.find("\"type\":\"video-nack\"") != std::string::npos;
  }

  static bool isControlHello(const std::string& json) {
    return json.find("\"type\":\"control-hello\"") != std::string::npos;
  }

  static bool isAllowedBeforeAuth(const std::string& json) {
    return isControlHello(json) || isControlPing(json);
  }

  static bool isBatchable(const std::string& json) {
    return !isControlPing(json) && !isStreamStats(json) && !isUdpRepairStats(json) && !isRenderStats(json) &&
           !isKeyframeRequest(json) && !isVideoNack(json) && !isControlHello(json);
  }

  bool dropOldestPointerMoveLocked() {
    for (auto it = queue_.begin(); it != queue_.end(); ++it) {
      if (isPointerMove(*it)) {
        queue_.erase(it);
        droppedPointerMoves_ += 1;
        if (droppedPointerMoves_ == 1 || droppedPointerMoves_ % 120 == 0) {
          std::cout << "SNINPUT_QUEUE droppedPointerMoves=" << droppedPointerMoves_
                    << " pending=" << queue_.size()
                    << "\n";
        }
        return true;
      }
    }
    return false;
  }

  void workerLoop() {
    while (true) {
      std::string json;
      std::uint64_t batchSequence = 0;
      std::size_t batchEvents = 0;
      std::uint32_t retryAttempt = 0;
      bool retry = false;
      std::uint64_t generation = 0;
      int sendFd = -1;
      {
        std::unique_lock<std::mutex> lock(mutex_);
        condition_.wait_for(lock, retryPollInterval_, [&] {
          const auto now = std::chrono::steady_clock::now();
          return stopped_ || (fd_ >= 0 && (!queue_.empty() || hasRetryWorkLocked(now)));
        });
        if (stopped_) return;
        if (fd_ < 0) continue;
        const auto wakeNow = std::chrono::steady_clock::now();
        if (queue_.empty() && !hasRetryWorkLocked(wakeNow)) continue;
        generation = fdGeneration_;
        sendFd = dup(fd_);
        if (sendFd < 0) {
          fd_ = -1;
          fdGeneration_ += 1;
          queue_.clear();
          pausePendingBatchesLocked("dup-failed");
          std::cerr << "SNINPUT async dup failed; waiting for reconnect.\n";
          continue;
        }

        const auto now = std::chrono::steady_clock::now();
        if (prepareRetryLocked(now, json, batchSequence, batchEvents, retryAttempt)) {
          retry = true;
        } else if (!queue_.empty()) {
          std::vector<std::string> batch;
          const bool batchEnabled = batchEnabled_;
          batch.push_back(std::move(queue_.front()));
          queue_.pop_front();
          if (batchEnabled && isBatchable(batch.front())) {
            while (batch.size() < maxBatchEvents_ && !queue_.empty() && isBatchable(queue_.front())) {
              batch.push_back(std::move(queue_.front()));
              queue_.pop_front();
            }
          }
          const bool shouldBatch = batchEnabled &&
            isBatchable(batch.front()) &&
            (batch.size() > 1 || !isPointerMove(batch.front()));
          if (shouldBatch) {
            batchEvents = batch.size();
            json = makeInputBatch(batch, batchSequence);
            trackPendingBatchLocked(batchSequence, json, batchEvents, now);
          } else {
            json = std::move(batch.front());
          }
        } else {
          close(sendFd);
          continue;
        }
      }

      if (retry) {
        std::cout << "SNINPUT_RETRY_TX sequence=" << batchSequence
                  << " attempt=" << retryAttempt
                  << " events=" << batchEvents
                  << "\n";
      }

      std::string outbound = json;
      {
        std::lock_guard<std::mutex> lock(mutex_);
        if (packetAuthEnabled_) {
          outbound = makeSecureControlEnvelope(packetAuthToken_, "c2h", ++packetAuthSequence_, json);
        }
      }
      const bool sent = sendJsonToFd(sendFd, outbound);
      close(sendFd);
      if (!sent) {
        std::lock_guard<std::mutex> lock(mutex_);
        if (generation == fdGeneration_) {
          fd_ = -1;
          fdGeneration_ += 1;
          queue_.clear();
          pausePendingBatchesLocked("send-failed");
        }
        std::cerr << "SNINPUT async send failed; waiting for reconnect.\n";
      }
    }
  }

  bool canRetryPendingLocked() const {
    return batchEnabled_ && (!authRequired_ || controlAuthenticated_);
  }

  bool hasRetryWorkLocked(std::chrono::steady_clock::time_point now) const {
    if (!canRetryPendingLocked()) return false;
    for (const auto& entry : pendingBatches_) {
      if (now - entry.second.lastSentAt >= retryDelay_) return true;
    }
    return false;
  }

  bool prepareRetryLocked(std::chrono::steady_clock::time_point now,
                          std::string& json,
                          std::uint64_t& sequence,
                          std::size_t& events,
                          std::uint32_t& attempt) {
    if (!canRetryPendingLocked()) return false;
    for (auto it = pendingBatches_.begin(); it != pendingBatches_.end();) {
      if (now - it->second.lastSentAt < retryDelay_) {
        ++it;
        continue;
      }
      if (it->second.attempts >= maxBatchRetryAttempts_) {
        std::cout << "SNINPUT_RETRY_DROP sequence=" << it->first
                  << " attempts=" << it->second.attempts
                  << " events=" << it->second.events
                  << " pending=" << (pendingBatches_.size() - 1)
                  << "\n";
        it = pendingBatches_.erase(it);
        continue;
      }
      sequence = it->first;
      json = it->second.json;
      events = it->second.events;
      it->second.attempts += 1;
      attempt = it->second.attempts;
      it->second.lastSentAt = now;
      return true;
    }
    return false;
  }

  void trackPendingBatchLocked(std::uint64_t sequence,
                               const std::string& json,
                               std::size_t events,
                               std::chrono::steady_clock::time_point now) {
    while (pendingBatches_.size() >= maxPendingBatches_) {
      const auto dropped = pendingBatches_.begin();
      std::cout << "SNINPUT_RETRY_DROP sequence=" << dropped->first
                << " attempts=" << dropped->second.attempts
                << " events=" << dropped->second.events
                << " reason=pending-limit"
                << "\n";
      pendingBatches_.erase(dropped);
    }
    pendingBatches_[sequence] = PendingBatch{json, events, 1, now};
  }

  void pausePendingBatchesLocked(const char* reason) {
    if (pendingBatches_.empty()) return;
    const auto now = std::chrono::steady_clock::now();
    for (auto& entry : pendingBatches_) {
      entry.second.lastSentAt = now;
    }
    std::cout << "SNINPUT_PENDING preserved=" << pendingBatches_.size()
              << " reason=" << reason
              << "\n";
  }

  std::string makeInputBatch(const std::vector<std::string>& events, std::uint64_t& sequence) {
    std::ostringstream out;
    sequence = ++batchSequence_;
    out << "{\"type\":\"input-batch\""
        << ",\"inputSessionId\":\"" << inputSessionId_ << "\""
        << ",\"sequence\":" << sequence
        << ",\"sentSteadyMicros\":" << steadyMicros()
        << ",\"events\":[";
    for (std::size_t i = 0; i < events.size(); ++i) {
      if (i > 0) out << ",";
      out << events[i];
    }
    out << "]}";
    if (events.size() > 1 && (sequence == 1 || sequence % 120 == 0)) {
      std::cout << "SNINPUT_BATCH_TX sequence=" << sequence
                << " events=" << events.size()
                << "\n";
    }
    return out.str();
  }

  bool sendJsonToFd(int fd, const std::string& json) {
    std::array<std::uint8_t, 12> header{};
    std::memcpy(header.data(), "SNI1", 4);
    writeLe32Raw(header.data() + 4, static_cast<std::uint32_t>(header.size()));
    writeLe32Raw(header.data() + 8, static_cast<std::uint32_t>(json.size()));

    if (!sendAllToFd(fd, header.data(), header.size())) return false;
    if (!json.empty() && !sendAllToFd(fd, json.data(), json.size())) return false;
    return true;
  }

  bool sendAllToFd(int fd, const void* data, std::size_t size) {
    const auto* cursor = static_cast<const std::uint8_t*>(data);
    std::size_t remaining = size;
    while (remaining > 0) {
      const ssize_t sent = send(fd, cursor, remaining, 0);
      if (sent < 0) {
        if (errno == EINTR) continue;
        return false;
      }
      if (sent == 0) return false;
      cursor += sent;
      remaining -= static_cast<std::size_t>(sent);
    }
    return true;
  }

  std::mutex mutex_;
  std::condition_variable condition_;
  std::deque<std::string> queue_;
  std::map<std::uint64_t, PendingBatch> pendingBatches_;
  const std::string inputSessionId_;
  std::thread worker_;
  std::uint64_t droppedPointerMoves_ = 0;
  std::uint64_t batchSequence_ = 0;
  std::uint64_t fdGeneration_ = 0;
  int fd_ = -1;
  bool batchEnabled_ = false;
  bool authRequired_ = false;
  bool controlAuthenticated_ = true;
  bool packetAuthEnabled_ = false;
  bool stopped_ = false;
  std::string packetAuthToken_;
  std::uint64_t packetAuthSequence_ = 0;
  static constexpr std::size_t maxQueueSize_ = 512;
  static constexpr std::size_t maxBatchEvents_ = 24;
  static constexpr std::size_t maxPendingBatches_ = 128;
  static constexpr std::uint32_t maxBatchRetryAttempts_ = 4;
  static constexpr auto retryDelay_ = std::chrono::milliseconds(85);
  static constexpr auto retryPollInterval_ = std::chrono::milliseconds(20);
};

class MediaReplayWindow {
public:
  bool accept(std::uint64_t epoch, std::uint64_t sequence) {
    if (sequence == 0) return false;

    std::lock_guard<std::mutex> lock(mutex_);
    if (!hasEpoch_ || epoch != epoch_) {
      resetLocked(epoch);
    }

    if (hasHighest_ && highest_ >= windowSize_ && sequence <= highest_ - windowSize_) {
      return false;
    }

    const std::size_t slot = static_cast<std::size_t>(sequence % windowSize_);
    if (seenSequences_[slot] == sequence) {
      return false;
    }

    seenSequences_[slot] = sequence;
    if (!hasHighest_ || sequence > highest_) {
      highest_ = sequence;
      hasHighest_ = true;
    }
    return true;
  }

  void reset(std::uint64_t epoch) {
    std::lock_guard<std::mutex> lock(mutex_);
    resetLocked(epoch);
  }

private:
  void resetLocked(std::uint64_t epoch) {
    epoch_ = epoch;
    highest_ = 0;
    hasEpoch_ = true;
    hasHighest_ = false;
    seenSequences_.fill(0);
  }

  std::mutex mutex_;
  static constexpr std::size_t windowSize_ = 4096;
  std::array<std::uint64_t, windowSize_> seenSequences_{};
  std::uint64_t epoch_ = 0;
  std::uint64_t highest_ = 0;
  bool hasEpoch_ = false;
  bool hasHighest_ = false;
};

std::string udpPeerString(const sockaddr_in& peer) {
  char ip[INET_ADDRSTRLEN]{};
  inet_ntop(AF_INET, &peer.sin_addr, ip, sizeof(ip));
  std::ostringstream out;
  out << (ip[0] ? ip : "unknown") << ":" << ntohs(peer.sin_port);
  return out.str();
}

bool sameUdpPeer(const sockaddr_in& left, const sockaddr_in& right) {
  return left.sin_family == right.sin_family &&
         left.sin_addr.s_addr == right.sin_addr.s_addr &&
         left.sin_port == right.sin_port;
}

class UdpPeerLock {
public:
  explicit UdpPeerLock(const char* label) : label_(label) {}

  bool isLocked() const {
    return locked_;
  }

  void observeGeneration(std::uint64_t generation) {
    if (generation == generation_) return;
    if (locked_) {
      std::cout << label_ << "_PEER_REBIND old=" << udpPeerString(peer_)
                << " generation=" << generation_
                << " nextGeneration=" << generation
                << "\n";
    }
    generation_ = generation;
    locked_ = false;
    peer_ = {};
  }

  bool acceptKnownPeer(const sockaddr_in& peer) {
    if (!locked_ || sameUdpPeer(peer_, peer)) return true;
    logReject(peer);
    return false;
  }

  bool lockOrAccept(const sockaddr_in& peer) {
    if (locked_) return acceptKnownPeer(peer);
    peer_ = peer;
    locked_ = true;
    std::cout << label_ << "_PEER_LOCK peer=" << udpPeerString(peer_) << "\n";
    return true;
  }

private:
  void logReject(const sockaddr_in& peer) {
    const auto now = std::chrono::steady_clock::now();
    if (now - lastRejectLogAt_ < std::chrono::seconds(1)) return;
    lastRejectLogAt_ = now;
    std::cout << label_ << "_PEER_REJECT peer=" << udpPeerString(peer)
              << " locked=" << udpPeerString(peer_)
              << "\n";
  }

  const char* label_ = "SNU1";
  sockaddr_in peer_{};
  std::uint64_t generation_ = 0;
  bool locked_ = false;
  std::chrono::steady_clock::time_point lastRejectLogAt_{};
};

std::shared_ptr<NativeInputSender> gNativeInputSender;
bool gLogInputEvents = false;
bool gRelativeMouse = false;
bool gMouseGrabbed = false;
std::string gExpectedSessionToken;
std::string gControlSessionNonce;
bool gHostPacketAuthVerified = false;
std::uint64_t gLastHostSecureSequence = 0;
bool gMediaCryptoEnabled = false;
std::uint64_t gMediaCryptoEpoch = 0;
std::atomic<std::uint64_t> gMediaPeerGeneration{0};
std::array<std::uint8_t, 32> gVideoMediaCryptoKey{};
std::array<std::uint8_t, 32> gAudioMediaCryptoKey{};
std::string gVideoMediaAuthKey;
std::string gAudioMediaAuthKey;
MediaReplayWindow gVideoMediaReplay;
MediaReplayWindow gAudioMediaReplay;
MediaReplayWindow gPreviousVideoMediaReplay;
MediaReplayWindow gPreviousAudioMediaReplay;

struct MediaCryptoKeySlot {
  bool enabled = false;
  std::uint64_t epoch = 0;
  std::array<std::uint8_t, 32> videoCryptoKey{};
  std::array<std::uint8_t, 32> audioCryptoKey{};
  std::string videoAuthKey;
  std::string audioAuthKey;
  std::chrono::steady_clock::time_point activatedAt{};
};

struct ResolvedMediaCrypto {
  bool enabled = false;
  bool rekeyGrace = false;
  std::uint64_t epoch = 0;
  std::array<std::uint8_t, 32> cryptoKey{};
  std::string authKey;
};

std::mutex gMediaCryptoMutex;
MediaCryptoKeySlot gCurrentMediaKeys;
MediaCryptoKeySlot gPreviousMediaKeys;
std::chrono::steady_clock::time_point gPreviousMediaKeysExpireAt{};
static constexpr auto kMediaRekeyGraceDuration = std::chrono::seconds(3);

MediaCryptoKeySlot makeMediaCryptoKeySlot(std::uint64_t epoch, const std::string& nonce) {
  MediaCryptoKeySlot slot;
  slot.enabled = true;
  slot.epoch = epoch;
  slot.videoCryptoKey = deriveMediaCryptoKey(gExpectedSessionToken, "video", epoch, nonce);
  slot.audioCryptoKey = deriveMediaCryptoKey(gExpectedSessionToken, "audio", epoch, nonce);
  slot.videoAuthKey = deriveMediaAuthKey(gExpectedSessionToken, "video", epoch, nonce);
  slot.audioAuthKey = deriveMediaAuthKey(gExpectedSessionToken, "audio", epoch, nonce);
  slot.activatedAt = std::chrono::steady_clock::now();
  return slot;
}

ResolvedMediaCrypto resolveMediaCryptoForEpoch(std::uint64_t epoch, bool video) {
  std::lock_guard<std::mutex> lock(gMediaCryptoMutex);
  const auto now = std::chrono::steady_clock::now();
  if (gPreviousMediaKeys.enabled && now > gPreviousMediaKeysExpireAt) {
    gPreviousMediaKeys = {};
    gPreviousMediaKeysExpireAt = {};
  }

  const MediaCryptoKeySlot* slot = nullptr;
  bool rekeyGrace = false;
  if (gCurrentMediaKeys.enabled && gCurrentMediaKeys.epoch == epoch) {
    slot = &gCurrentMediaKeys;
  } else if (gPreviousMediaKeys.enabled && gPreviousMediaKeys.epoch == epoch) {
    slot = &gPreviousMediaKeys;
    rekeyGrace = true;
  }
  if (!slot) return {};

  ResolvedMediaCrypto resolved;
  resolved.enabled = true;
  resolved.rekeyGrace = rekeyGrace;
  resolved.epoch = slot->epoch;
  resolved.cryptoKey = video ? slot->videoCryptoKey : slot->audioCryptoKey;
  resolved.authKey = video ? slot->videoAuthKey : slot->audioAuthKey;
  return resolved;
}

void activateMediaCryptoKeys(std::uint64_t epoch, const std::string& nonce) {
  if (gExpectedSessionToken.empty() || nonce.empty()) {
    std::lock_guard<std::mutex> lock(gMediaCryptoMutex);
    gMediaCryptoEnabled = false;
    gMediaCryptoEpoch = 0;
    gVideoMediaAuthKey.clear();
    gAudioMediaAuthKey.clear();
    gCurrentMediaKeys = {};
    gPreviousMediaKeys = {};
    gPreviousMediaKeysExpireAt = {};
    return;
  }
  const MediaCryptoKeySlot nextKeys = makeMediaCryptoKeySlot(epoch, nonce);
  {
    std::lock_guard<std::mutex> lock(gMediaCryptoMutex);
    if (gCurrentMediaKeys.enabled && gCurrentMediaKeys.epoch != epoch) {
      gPreviousMediaKeys = gCurrentMediaKeys;
      gPreviousMediaKeysExpireAt = nextKeys.activatedAt + kMediaRekeyGraceDuration;
      gPreviousVideoMediaReplay.reset(gPreviousMediaKeys.epoch);
      gPreviousAudioMediaReplay.reset(gPreviousMediaKeys.epoch);
    } else if (!gCurrentMediaKeys.enabled) {
      gPreviousMediaKeys = {};
      gPreviousMediaKeysExpireAt = {};
    }
    gCurrentMediaKeys = nextKeys;
  }
  gMediaCryptoEnabled = true;
  gMediaCryptoEpoch = epoch;
  gVideoMediaCryptoKey = nextKeys.videoCryptoKey;
  gAudioMediaCryptoKey = nextKeys.audioCryptoKey;
  gVideoMediaAuthKey = nextKeys.videoAuthKey;
  gAudioMediaAuthKey = nextKeys.audioAuthKey;
  gVideoMediaReplay.reset(epoch);
  gAudioMediaReplay.reset(epoch);
  const std::uint64_t peerGeneration = gMediaPeerGeneration.fetch_add(1, std::memory_order_relaxed) + 1;
  std::cout << "SNMEDIA_KEY_ROTATE epoch=" << gMediaCryptoEpoch
            << " nonce=" << (nonce == "bootstrap" ? "bootstrap" : "control")
            << " peerGeneration=" << peerGeneration
            << " rekeyGraceMs=" << std::chrono::duration_cast<std::chrono::milliseconds>(kMediaRekeyGraceDuration).count()
            << "\n";
}

const char* jsonBool(bool value) {
  return value ? "true" : "false";
}

std::size_t skipJsonWhitespace(const std::string& json, std::size_t offset) {
  while (offset < json.size()) {
    const char ch = json[offset];
    if (ch != ' ' && ch != '\n' && ch != '\r' && ch != '\t') break;
    ++offset;
  }
  return offset;
}

std::uint64_t jsonUint64Value(const std::string& json, const char* key, std::uint64_t fallback = 0) {
  const std::string needle = std::string("\"") + key + "\"";
  const std::size_t keyOffset = json.find(needle);
  if (keyOffset == std::string::npos) return fallback;
  const std::size_t colon = json.find(':', keyOffset + needle.size());
  if (colon == std::string::npos) return fallback;
  const std::size_t valueOffset = skipJsonWhitespace(json, colon + 1);
  try {
    return static_cast<std::uint64_t>(std::stoull(json.substr(valueOffset)));
  } catch (...) {
    return fallback;
  }
}

bool jsonBoolValue(const std::string& json, const char* key, bool fallback = false) {
  const std::string needle = std::string("\"") + key + "\"";
  const std::size_t keyOffset = json.find(needle);
  if (keyOffset == std::string::npos) return fallback;
  const std::size_t colon = json.find(':', keyOffset + needle.size());
  if (colon == std::string::npos) return fallback;
  const std::size_t valueOffset = skipJsonWhitespace(json, colon + 1);
  if (json.compare(valueOffset, 4, "true") == 0) return true;
  if (json.compare(valueOffset, 5, "false") == 0) return false;
  return fallback;
}

std::string jsonStringValue(const std::string& json, const char* key, const std::string& fallback = {}) {
  const std::string needle = std::string("\"") + key + "\"";
  const std::size_t keyOffset = json.find(needle);
  if (keyOffset == std::string::npos) return fallback;
  const std::size_t colon = json.find(':', keyOffset + needle.size());
  if (colon == std::string::npos) return fallback;
  std::size_t offset = skipJsonWhitespace(json, colon + 1);
  if (offset >= json.size() || json[offset] != '"') return fallback;
  ++offset;

  std::string raw;
  while (offset < json.size()) {
    const char ch = json[offset++];
    if (ch == '"') return raw;
    if (ch == '\\' && offset < json.size()) {
      const char escaped = json[offset++];
      switch (escaped) {
        case 'n': raw.push_back('\n'); break;
        case 'r': raw.push_back('\r'); break;
        case 't': raw.push_back('\t'); break;
        case '\\': raw.push_back('\\'); break;
        case '"': raw.push_back('"'); break;
        default: raw.push_back(escaped); break;
      }
    } else {
      raw.push_back(ch);
    }
  }
  return fallback;
}

bool unwrapSecureControlEnvelope(const std::string& envelope,
                                 const std::string& token,
                                 const char* direction,
                                 std::uint64_t& lastSequence,
                                 std::string& payloadOut) {
  if (jsonStringValue(envelope, "type") != "secure-control") return false;
  if (token.empty()) {
    std::cerr << "SNCONTROL_PACKET_AUTH rejected reason=no-token\n";
    return false;
  }

  const std::uint64_t sequence = jsonUint64Value(envelope, "authSeq");
  const std::string payload = jsonStringValue(envelope, "payload");
  const std::string mac = jsonStringValue(envelope, "authMac");
  if (sequence == 0 || payload.empty() || mac.empty()) {
    std::cerr << "SNCONTROL_PACKET_AUTH rejected reason=malformed\n";
    return false;
  }
  if (sequence <= lastSequence) {
    std::cerr << "SNCONTROL_PACKET_AUTH rejected reason=replay seq=" << sequence
              << " last=" << lastSequence
              << "\n";
    return false;
  }

  const std::string expected = controlAuthMac(token, direction, sequence, payload);
  if (!constantTimeEqual(mac, expected)) {
    std::cerr << "SNCONTROL_PACKET_AUTH rejected reason=bad-mac seq=" << sequence << "\n";
    return false;
  }
  lastSequence = sequence;
  payloadOut = payload;
  return true;
}

double backingScaleForView(NSView* view) {
  NSWindow* window = [view window];
  if (window) return std::max<double>([window backingScaleFactor], 1.0);
  NSScreen* screen = [NSScreen mainScreen];
  return screen ? std::max<double>([screen backingScaleFactor], 1.0) : 1.0;
}

void setRelativeMouseGrab(bool enabled) {
  if (gMouseGrabbed == enabled) return;
  const CGError error = CGAssociateMouseAndMouseCursorPosition(enabled ? false : true);
  if (error == kCGErrorSuccess) {
    gMouseGrabbed = enabled;
    std::cout << "SNINPUT mouse grab " << (enabled ? "enabled" : "disabled") << "\n";
  } else {
    std::cerr << "SNINPUT mouse grab failed: CGError " << error << "\n";
  }
}

void restoreRelativeMouseGrab() {
  if (!gMouseGrabbed) return;
  CGAssociateMouseAndMouseCursorPosition(true);
  gMouseGrabbed = false;
  std::cout << "SNINPUT mouse grab disabled\n";
}

bool sendNativeInputJson(const std::string& json) {
  const bool sent = gNativeInputSender && gNativeInputSender->sendJson(json);
  if (gLogInputEvents) {
    const bool isMove = json.find("\"type\":\"pointer-move\"") != std::string::npos;
    const bool isControlPing = json.find("\"type\":\"control-ping\"") != std::string::npos;
    static auto lastMoveLog = std::chrono::steady_clock::time_point{};
    const auto now = std::chrono::steady_clock::now();
    if (!isControlPing && (!isMove || now - lastMoveLog > std::chrono::milliseconds(250))) {
      if (isMove) lastMoveLog = now;
      std::cout << (sent ? "SNINPUT_TX " : "SNINPUT_LOCAL ") << json << "\n";
    }
  }
  return sent;
}

void sendControlPing(std::uint64_t sequence) {
  std::ostringstream out;
  out << "{\"type\":\"control-ping\""
      << ",\"sequence\":" << sequence
      << ",\"sentSteadyMicros\":" << steadyMicros()
      << "}";
  sendNativeInputJson(out.str());
}

bool sendKeyframeRequest(const std::string& reason,
                         std::uint64_t videoSequence,
                         std::uint64_t dropped,
                         std::uint64_t decodeErrors) {
  static std::mutex mutex;
  static auto lastSentAt = std::chrono::steady_clock::time_point{};
  static std::uint64_t requestSequence = 0;

  const auto now = std::chrono::steady_clock::now();
  std::uint64_t sequence = 0;
  {
    std::lock_guard<std::mutex> lock(mutex);
    if (lastSentAt.time_since_epoch().count() != 0 &&
        now - lastSentAt < std::chrono::milliseconds(500)) {
      return false;
    }
    lastSentAt = now;
    sequence = ++requestSequence;
  }

  std::ostringstream out;
  out << "{\"type\":\"keyframe-request\""
      << ",\"sequence\":" << sequence
      << ",\"reason\":\"" << jsonEscape(reason.c_str()) << "\""
      << ",\"videoSequence\":" << videoSequence
      << ",\"dropped\":" << dropped
      << ",\"decodeErrors\":" << decodeErrors
      << ",\"sentSteadyMicros\":" << steadyMicros()
      << "}";
  const bool sent = sendNativeInputJson(out.str());
  std::cout << "SNV1_KEYFRAME_REQUEST_TX sequence=" << sequence
            << " reason=" << reason
            << " sent=" << boolText(sent)
            << " videoSequence=" << videoSequence
            << " dropped=" << dropped
            << " decodeErrors=" << decodeErrors
            << "\n";
  return sent;
}

bool sendVideoNackRequest(const std::string& reason,
                          const std::vector<std::uint64_t>& packetIds) {
  if (packetIds.empty()) return false;

  static std::mutex mutex;
  static auto lastSentAt = std::chrono::steady_clock::time_point{};
  static std::uint64_t requestSequence = 0;

  const auto now = std::chrono::steady_clock::now();
  std::uint64_t sequence = 0;
  {
    std::lock_guard<std::mutex> lock(mutex);
    if (lastSentAt.time_since_epoch().count() != 0 &&
        now - lastSentAt < std::chrono::milliseconds(35)) {
      return false;
    }
    lastSentAt = now;
    sequence = ++requestSequence;
  }

  std::ostringstream out;
  out << "{\"type\":\"video-nack\""
      << ",\"sequence\":" << sequence
      << ",\"reason\":\"" << jsonEscape(reason.c_str()) << "\""
      << ",\"packetIds\":[";
  const std::size_t count = std::min<std::size_t>(packetIds.size(), 32);
  for (std::size_t i = 0; i < count; ++i) {
    if (i > 0) out << ",";
    out << packetIds[i];
  }
  out << "],\"sentSteadyMicros\":" << steadyMicros()
      << "}";
  const bool sent = sendNativeInputJson(out.str());
  std::cout << "SNU1_NACK_TX sequence=" << sequence
            << " reason=" << reason
            << " sent=" << boolText(sent)
            << " packetIds=" << count
            << " first=" << packetIds.front()
            << "\n";
  return sent;
}

void sendControlHello() {
  if (!gExpectedSessionToken.empty()) {
    gControlSessionNonce = randomHex(16);
    gHostPacketAuthVerified = false;
    gLastHostSecureSequence = 0;
  }
  std::ostringstream out;
  out << "{\"type\":\"control-hello\""
      << ",\"protocolVersion\":2"
      << ",\"sentSteadyMicros\":" << steadyMicros()
      << ",\"inputSessionId\":\""
      << (gNativeInputSender ? gNativeInputSender->inputSessionId() : std::string())
      << "\""
      << ",\"inputBatch\":true"
      << ",\"inputAck\":true"
      << ",\"audioUdp\":true"
      << ",\"videoUdp\":true"
      << ",\"keyframeRequest\":true"
      << ",\"videoNack\":true"
      << ",\"udpRepairStats\":true"
      << ",\"sessionAuth\":" << jsonBool(!gExpectedSessionToken.empty())
      << ",\"packetAuth\":" << jsonBool(!gExpectedSessionToken.empty())
      << ",\"mediaCrypto\":" << jsonBool(!gExpectedSessionToken.empty());
  if (!gControlSessionNonce.empty()) {
    out << ",\"sessionNonce\":\"" << gControlSessionNonce << "\"";
  }
  out << "}";
  sendNativeInputJson(out.str());
  std::cout << "SNCONTROL_HELLO protocol=2 inputBatch=requested sessionAuth="
            << boolText(!gExpectedSessionToken.empty())
            << " packetAuth=" << boolText(!gExpectedSessionToken.empty())
            << " mediaCrypto=" << boolText(!gExpectedSessionToken.empty())
            << " fallback=single-event-until-ack\n";
}

enum class HostControlEvent {
  None,
  HelloAck,
  Pong,
  InputAck,
  Other
};

HostControlEvent handleHostControlPayload(const std::string& rawPayload) {
  std::string payload = rawPayload;
  const std::string rawType = jsonStringValue(rawPayload, "type");
  bool securePayload = false;
  if (rawType == "secure-control") {
    securePayload = unwrapSecureControlEnvelope(rawPayload,
                                                gExpectedSessionToken,
                                                "h2c",
                                                gLastHostSecureSequence,
                                                payload);
    if (!securePayload) return HostControlEvent::None;
  } else if (gHostPacketAuthVerified && rawType != "control-hello-ack") {
    std::cerr << "SNCONTROL_PACKET_AUTH rejected reason=missing-envelope type="
              << (rawType.empty() ? "unknown" : rawType)
              << "\n";
    return HostControlEvent::None;
  }

  if (payload.find("\"type\":\"control-hello-ack\"") != std::string::npos) {
    const bool inputBatch = jsonBoolValue(payload, "inputBatch");
    const bool inputAck = jsonBoolValue(payload, "inputAck");
    const bool keyframeRequest = jsonBoolValue(payload, "keyframeRequest");
    const bool videoNack = jsonBoolValue(payload, "videoNack");
    const bool udpRepairStats = jsonBoolValue(payload, "udpRepairStats");
    const bool sessionAuth = jsonBoolValue(payload, "sessionAuth");
    const bool packetAuth = jsonBoolValue(payload, "packetAuth");
    const bool mediaCrypto = jsonBoolValue(payload, "mediaCrypto");
    const std::uint64_t mediaEpoch = jsonUint64Value(payload, "mediaEpoch");
    const std::string sessionProof = jsonStringValue(payload, "sessionProof");
    const std::string legacyAckToken = jsonStringValue(payload, "sessionToken");
    const bool needsSessionAuth = !gExpectedSessionToken.empty();
    const bool proofAccepted = sessionAuth &&
      !gControlSessionNonce.empty() &&
      constantTimeEqual(sessionProof, controlSessionProof(gExpectedSessionToken, gControlSessionNonce));
    const bool legacyAccepted = sessionAuth && legacyAckToken == gExpectedSessionToken;
    const bool sessionAccepted = !needsSessionAuth || proofAccepted || legacyAccepted;
    gHostPacketAuthVerified = sessionAccepted && packetAuth && proofAccepted;
    gLastHostSecureSequence = 0;
    if (sessionAccepted && proofAccepted && mediaCrypto && mediaEpoch > 0) {
      activateMediaCryptoKeys(mediaEpoch, gControlSessionNonce);
    }
    if (gNativeInputSender) {
      gNativeInputSender->setControlAuthenticated(sessionAccepted, sessionAccepted || !needsSessionAuth);
      gNativeInputSender->setPacketAuthEnabled(gHostPacketAuthVerified);
      gNativeInputSender->setBatchEnabled(sessionAccepted && inputBatch && inputAck);
    }
    if (needsSessionAuth && !sessionAccepted) {
      std::cerr << "SNCONTROL_AUTH failed sessionAuth=" << boolText(sessionAuth)
                << " proofReceived=" << boolText(!sessionProof.empty())
                << " legacyTokenReceived=" << boolText(!legacyAckToken.empty())
                << "; input/control feedback blocked until a matching native host reconnects.\n";
      return HostControlEvent::HelloAck;
    }
    std::cout << "SNCONTROL_HELLO_ACK protocol=" << jsonUint64Value(payload, "protocolVersion")
              << " inputBatch=" << boolText(inputBatch)
              << " inputAck=" << boolText(inputAck)
              << " keyframeRequest=" << boolText(keyframeRequest)
              << " videoNack=" << boolText(videoNack)
              << " udpRepairStats=" << boolText(udpRepairStats)
              << " sessionAuth=" << (needsSessionAuth ? "verified" : "disabled")
              << " packetAuth=" << boolText(gHostPacketAuthVerified)
              << " mediaEpoch=" << gMediaCryptoEpoch
              << "\n";
    return HostControlEvent::HelloAck;
  }

  const std::uint64_t sentSteadyMicros = jsonUint64Value(payload, "sentSteadyMicros");
  if (sentSteadyMicros == 0) return HostControlEvent::Other;
  const double rttMs = static_cast<double>(steadyMicros() - sentSteadyMicros) / 1000.0;
  if (payload.find("\"type\":\"control-pong\"") != std::string::npos) {
    std::cout << "SNINPUT_RTT rttMs=" << std::fixed << std::setprecision(1) << rttMs << "\n";
    return HostControlEvent::Pong;
  }
  if (payload.find("\"type\":\"input-ack\"") != std::string::npos) {
    const std::uint64_t sequence = jsonUint64Value(payload, "sequence");
    const std::uint64_t applied = jsonUint64Value(payload, "applied");
    const bool duplicate = jsonBoolValue(payload, "duplicate");
    const bool stale = jsonBoolValue(payload, "stale");
    const bool sessionMismatch = jsonBoolValue(payload, "sessionMismatch");
    if (gNativeInputSender) {
      gNativeInputSender->observeInputAck(sequence, applied);
    }
    static auto lastAckLog = std::chrono::steady_clock::time_point{};
    const auto now = std::chrono::steady_clock::now();
    if (now - lastAckLog > std::chrono::milliseconds(250)) {
      lastAckLog = now;
      std::cout << "SNINPUT_ACK sequence=" << sequence
                << " applied=" << applied
                << " duplicate=" << boolText(duplicate)
                << " stale=" << boolText(stale)
                << " sessionMismatch=" << boolText(sessionMismatch)
                << " rttMs=" << std::fixed << std::setprecision(1) << rttMs
                << "\n";
    }
    return HostControlEvent::InputAck;
  }
  return HostControlEvent::Other;
}

std::string pointerJsonFromPoint(const char* type,
                                 NSPoint point,
                                 NSView* view,
                                 NSInteger button,
                                 CGFloat dx,
                                 CGFloat dy,
                                 bool relative) {
  const NSRect bounds = [view bounds];
  const double width = std::max<double>(bounds.size.width, 1.0);
  const double height = std::max<double>(bounds.size.height, 1.0);
  const double x = std::clamp(point.x / width, 0.0, 1.0);
  const double y = std::clamp(1.0 - (point.y / height), 0.0, 1.0);
  std::ostringstream out;
  out << "{\"type\":\"" << type
      << "\",\"x\":" << x
      << ",\"y\":" << y
      << ",\"button\":" << button
      << ",\"dx\":" << dx
      << ",\"dy\":" << dy
      << ",\"relative\":" << jsonBool(relative)
      << "}";
  return out.str();
}

std::string pointerEventJson(const char* type, NSEvent* event, NSView* view) {
  const NSPoint point = [view convertPoint:[event locationInWindow] fromView:nil];
  const bool wheel = std::strcmp(type, "wheel") == 0;
  const double scale = backingScaleForView(view);
  const CGFloat dx = wheel ? [event scrollingDeltaX] : [event deltaX] * scale;
  const CGFloat dy = wheel ? [event scrollingDeltaY] : -[event deltaY] * scale;
  return pointerJsonFromPoint(type,
                              point,
                              view,
                              [event buttonNumber],
                              dx,
                              dy,
                              gRelativeMouse);
}

std::string keyEventJson(const char* type, NSEvent* event) {
  const std::string text = jsonEscape([[event charactersIgnoringModifiers] UTF8String]);
  std::ostringstream out;
  out << "{\"type\":\"" << type
      << "\",\"key\":\"" << text
      << "\",\"keyCode\":" << [event keyCode]
      << ",\"modifiers\":" << static_cast<unsigned long long>([event modifierFlags])
      << "}";
  return out.str();
}

std::string pasteboardText() {
  NSString* value = [[NSPasteboard generalPasteboard] stringForType:NSPasteboardTypeString];
  return value ? std::string([value UTF8String] ?: "") : "";
}

bool sendCommandShortcut(NSEvent* event) {
  if (([event modifierFlags] & NSEventModifierFlagCommand) == 0) return false;
  NSString* key = [[[event charactersIgnoringModifiers] lowercaseString] copy];
  if (!key || [key length] == 0) return false;

  if ([key isEqualToString:@"v"]) {
    sendNativeInputJson("{\"type\":\"clipboard\",\"text\":\"" + jsonEscape(pasteboardText().c_str()) + "\"}");
    sendNativeInputJson("{\"type\":\"paste\"}");
    return true;
  }
  if ([key isEqualToString:@"c"]) {
    sendNativeInputJson("{\"type\":\"copy\"}");
    return true;
  }
  if ([key isEqualToString:@"x"]) {
    sendNativeInputJson("{\"type\":\"cut\"}");
    return true;
  }
  if ([key isEqualToString:@"a"]) {
    sendNativeInputJson("{\"type\":\"select-all\"}");
    return true;
  }
  return false;
}

void logPointerEvent(const char* type, NSEvent* event, NSView* view) {
  sendNativeInputJson(pointerEventJson(type, event, view));
}

void logKeyEvent(const char* type, NSEvent* event) {
  sendNativeInputJson(keyEventJson(type, event));
}

std::string osStatusString(OSStatus status) {
  std::ostringstream out;
  out << "OSStatus " << status;
  return out.str();
}

void checkStatus(OSStatus status, const char* label) {
  if (status != noErr) {
    throw std::runtime_error(std::string(label) + ": " + osStatusString(status));
  }
}

std::uint32_t readLe32(const std::vector<std::uint8_t>& data, std::size_t offset) {
  if (offset + 4 > data.size()) throw std::runtime_error("Unexpected EOF reading uint32.");
  return static_cast<std::uint32_t>(data[offset])
       | (static_cast<std::uint32_t>(data[offset + 1]) << 8)
       | (static_cast<std::uint32_t>(data[offset + 2]) << 16)
       | (static_cast<std::uint32_t>(data[offset + 3]) << 24);
}

std::uint32_t readLe32Raw(const std::uint8_t* data) {
  return static_cast<std::uint32_t>(data[0])
       | (static_cast<std::uint32_t>(data[1]) << 8)
       | (static_cast<std::uint32_t>(data[2]) << 16)
       | (static_cast<std::uint32_t>(data[3]) << 24);
}

std::uint16_t readLe16Raw(const std::uint8_t* data) {
  return static_cast<std::uint16_t>(data[0])
       | static_cast<std::uint16_t>(static_cast<std::uint16_t>(data[1]) << 8);
}

std::uint64_t readLe64Raw(const std::uint8_t* data) {
  std::uint64_t value = 0;
  for (int i = 0; i < 8; ++i) {
    value |= static_cast<std::uint64_t>(data[i]) << (8 * i);
  }
  return value;
}

std::uint64_t readLe64(const std::vector<std::uint8_t>& data, std::size_t offset) {
  if (offset + 8 > data.size()) throw std::runtime_error("Unexpected EOF reading uint64.");
  std::uint64_t value = 0;
  for (int i = 0; i < 8; ++i) {
    value |= static_cast<std::uint64_t>(data[offset + i]) << (8 * i);
  }
  return value;
}

std::uint32_t readBe32(const std::vector<std::uint8_t>& data, std::size_t offset) {
  if (offset + 4 > data.size()) return 0;
  return (static_cast<std::uint32_t>(data[offset]) << 24)
       | (static_cast<std::uint32_t>(data[offset + 1]) << 16)
       | (static_cast<std::uint32_t>(data[offset + 2]) << 8)
       | static_cast<std::uint32_t>(data[offset + 3]);
}

void appendBe32(std::vector<std::uint8_t>& data, std::uint32_t value) {
  data.push_back(static_cast<std::uint8_t>((value >> 24) & 0xff));
  data.push_back(static_cast<std::uint8_t>((value >> 16) & 0xff));
  data.push_back(static_cast<std::uint8_t>((value >> 8) & 0xff));
  data.push_back(static_cast<std::uint8_t>(value & 0xff));
}

struct SnvPacket {
  std::uint32_t codec = 0;
  std::uint32_t packetFormat = 0;
  std::uint32_t width = 0;
  std::uint32_t height = 0;
  std::uint64_t sequence = 0;
  std::uint64_t timestampMicros = 0;
  std::uint32_t durationMicros = 0;
  std::uint32_t flags = 0;
  std::uint64_t hostUnixMicros = 0;
  std::vector<std::uint8_t> payload;
};

struct DecodedVideoFrameMetadata {
  std::uint64_t sequence = 0;
  std::uint64_t timestampMicros = 0;
  std::uint64_t durationMicros = 0;
  std::uint64_t hostUnixMicros = 0;
  std::uint64_t decodeSubmittedAtMicros = 0;
};

#pragma pack(push, 1)
struct UdpVideoFragmentHeader {
  char magic[4] = {'S', 'N', 'U', '1'};
  std::uint16_t headerSize = sizeof(UdpVideoFragmentHeader);
  std::uint16_t fragmentCount = 0;
  std::uint16_t fragmentIndex = 0;
  std::uint16_t flags = 0;
  std::uint64_t packetId = 0;
  std::uint32_t packetSize = 0;
  std::uint32_t fragmentOffset = 0;
  std::uint32_t payloadSize = 0;
};

struct UdpVideoAuthenticatedFragmentHeader {
  char magic[4] = {'S', 'N', 'U', '2'};
  std::uint16_t headerSize = sizeof(UdpVideoAuthenticatedFragmentHeader);
  std::uint16_t fragmentCount = 0;
  std::uint16_t fragmentIndex = 0;
  std::uint16_t flags = 0;
  std::uint64_t packetId = 0;
  std::uint32_t packetSize = 0;
  std::uint32_t fragmentOffset = 0;
  std::uint32_t payloadSize = 0;
  std::uint64_t authSeq = 0;
  std::uint64_t authEpoch = 0;
  char authTag[kUdpAuthTagHexSize]{};
};

struct AudioPacketHeader {
  char magic[4] = {'S', 'N', 'A', '1'};
  std::uint16_t headerSize = sizeof(AudioPacketHeader);
  std::uint16_t channels = 2;
  std::uint32_t sampleRate = 0;
  std::uint32_t frameCount = 0;
  std::uint64_t sequence = 0;
  std::uint64_t hostUnixMicros = 0;
  std::uint32_t payloadSize = 0;
};

struct AudioAuthenticatedPacketHeader {
  char magic[4] = {'S', 'N', 'A', '2'};
  std::uint16_t headerSize = sizeof(AudioAuthenticatedPacketHeader);
  std::uint16_t channels = 2;
  std::uint32_t sampleRate = 0;
  std::uint32_t frameCount = 0;
  std::uint64_t sequence = 0;
  std::uint64_t hostUnixMicros = 0;
  std::uint32_t payloadSize = 0;
  std::uint64_t authSeq = 0;
  std::uint64_t authEpoch = 0;
  char authTag[kUdpAuthTagHexSize]{};
};
#pragma pack(pop)

struct NalUnit {
  std::vector<std::uint8_t> bytes;
  std::uint8_t type = 0;
};

SnvPacket parseSnvHeader(const std::uint8_t* header, std::size_t headerSize) {
  if (headerSize < 52) {
    throw std::runtime_error("SNV1 header too small.");
  }
  if (std::memcmp(header, "SNV1", 4) != 0) {
    throw std::runtime_error("Invalid SNV1 magic in stream.");
  }

  SnvPacket packet;
  packet.codec = readLe32Raw(header + 8);
  packet.packetFormat = readLe32Raw(header + 12);
  packet.width = readLe32Raw(header + 16);
  packet.height = readLe32Raw(header + 20);
  packet.sequence = readLe64Raw(header + 24);
  packet.timestampMicros = readLe64Raw(header + 32);
  packet.durationMicros = readLe32Raw(header + 40);
  packet.flags = readLe32Raw(header + 44);
  return packet;
}

SnvPacket parseSnvPacketBytes(const std::vector<std::uint8_t>& data) {
  if (data.size() < 52) {
    throw std::runtime_error("Truncated SNV1 packet bytes.");
  }
  const std::uint32_t headerSize = readLe32Raw(data.data() + 4);
  if (headerSize < 52 || headerSize > data.size()) {
    throw std::runtime_error("Invalid SNV1 packet header size.");
  }

  SnvPacket packet = parseSnvHeader(data.data(), headerSize);
  if (headerSize >= 60) {
    packet.hostUnixMicros = readLe64Raw(data.data() + 52);
  }
  const std::uint32_t payloadSize = readLe32Raw(data.data() + 48);
  const std::size_t nextOffset = static_cast<std::size_t>(headerSize) + payloadSize;
  if (nextOffset > data.size()) {
    throw std::runtime_error("Truncated SNV1 packet payload.");
  }
  packet.payload.assign(data.begin() + static_cast<std::ptrdiff_t>(headerSize),
                        data.begin() + static_cast<std::ptrdiff_t>(nextOffset));
  return packet;
}

std::vector<SnvPacket> readSnvPackets(const std::string& file, std::uint64_t maxPackets) {
  std::ifstream input(file, std::ios::binary);
  if (!input.is_open()) {
    throw std::runtime_error("Could not open SNV file: " + file);
  }

  input.seekg(0, std::ios::end);
  const auto fileSize = input.tellg();
  input.seekg(0, std::ios::beg);
  if (fileSize <= 0) {
    throw std::runtime_error("SNV file is empty: " + file);
  }

  std::vector<std::uint8_t> data(static_cast<std::size_t>(fileSize));
  input.read(reinterpret_cast<char*>(data.data()), static_cast<std::streamsize>(data.size()));

  std::vector<SnvPacket> packets;
  std::size_t offset = 0;
  while (offset < data.size() && (maxPackets == 0 || packets.size() < maxPackets)) {
    if (offset + 52 > data.size()) {
      throw std::runtime_error("Truncated SNV header at byte " + std::to_string(offset));
    }
    if (std::memcmp(data.data() + offset, "SNV1", 4) != 0) {
      throw std::runtime_error("Invalid SNV1 magic at byte " + std::to_string(offset));
    }

    const std::uint32_t headerSize = readLe32(data, offset + 4);
    if (headerSize < 52 || offset + headerSize > data.size()) {
      throw std::runtime_error("Invalid SNV1 header size at byte " + std::to_string(offset));
    }

    SnvPacket packet;
    packet.codec = readLe32(data, offset + 8);
    packet.packetFormat = readLe32(data, offset + 12);
    packet.width = readLe32(data, offset + 16);
    packet.height = readLe32(data, offset + 20);
    packet.sequence = readLe64(data, offset + 24);
    packet.timestampMicros = readLe64(data, offset + 32);
    packet.durationMicros = readLe32(data, offset + 40);
    packet.flags = readLe32(data, offset + 44);
    if (headerSize >= 60) {
      packet.hostUnixMicros = readLe64(data, offset + 52);
    }
    const std::uint32_t payloadSize = readLe32(data, offset + 48);
    const std::size_t payloadOffset = offset + headerSize;
    const std::size_t nextOffset = payloadOffset + payloadSize;
    if (nextOffset > data.size()) {
      throw std::runtime_error("SNV1 packet payload exceeds file length.");
    }

    packet.payload.assign(data.begin() + static_cast<std::ptrdiff_t>(payloadOffset),
                          data.begin() + static_cast<std::ptrdiff_t>(nextOffset));
    packets.push_back(std::move(packet));
    offset = nextOffset;
  }

  return packets;
}

std::size_t annexBStartCodeSize(const std::vector<std::uint8_t>& data, std::size_t offset) {
  if (offset + 3 <= data.size() && data[offset] == 0 && data[offset + 1] == 0 && data[offset + 2] == 1) {
    return 3;
  }
  if (offset + 4 <= data.size() && data[offset] == 0 && data[offset + 1] == 0 && data[offset + 2] == 0 && data[offset + 3] == 1) {
    return 4;
  }
  return 0;
}

std::vector<NalUnit> parseAnnexBNalUnits(const std::vector<std::uint8_t>& payload) {
  std::vector<NalUnit> nals;
  std::size_t offset = 0;
  while (offset < payload.size()) {
    while (offset < payload.size() && annexBStartCodeSize(payload, offset) == 0) {
      ++offset;
    }
    if (offset >= payload.size()) break;

    const std::size_t startCode = annexBStartCodeSize(payload, offset);
    std::size_t nalStart = offset + startCode;
    std::size_t next = nalStart;
    while (next < payload.size() && annexBStartCodeSize(payload, next) == 0) {
      ++next;
    }

    if (next > nalStart) {
      NalUnit nal;
      nal.bytes.assign(payload.begin() + static_cast<std::ptrdiff_t>(nalStart),
                       payload.begin() + static_cast<std::ptrdiff_t>(next));
      nal.type = nal.bytes.empty() ? 0 : (nal.bytes[0] & 0x1f);
      nals.push_back(std::move(nal));
    }
    offset = next;
  }
  return nals;
}

std::vector<NalUnit> parseAvccNalUnits(const std::vector<std::uint8_t>& payload) {
  std::vector<NalUnit> nals;
  std::size_t offset = 0;
  while (offset + 4 <= payload.size()) {
    const std::uint32_t size = readBe32(payload, offset);
    offset += 4;
    if (size == 0 || offset + size > payload.size()) {
      nals.clear();
      return nals;
    }
    NalUnit nal;
    nal.bytes.assign(payload.begin() + static_cast<std::ptrdiff_t>(offset),
                     payload.begin() + static_cast<std::ptrdiff_t>(offset + size));
    nal.type = nal.bytes.empty() ? 0 : (nal.bytes[0] & 0x1f);
    nals.push_back(std::move(nal));
    offset += size;
  }
  if (offset != payload.size()) nals.clear();
  return nals;
}

std::vector<NalUnit> parseNalUnits(const std::vector<std::uint8_t>& payload) {
  auto nals = parseAnnexBNalUnits(payload);
  if (!nals.empty()) return nals;
  return parseAvccNalUnits(payload);
}

std::vector<std::uint8_t> buildAvccSample(const std::vector<NalUnit>& nals) {
  std::vector<std::uint8_t> sample;
  for (const auto& nal : nals) {
    if (nal.bytes.empty()) continue;
    if (nal.type == 7 || nal.type == 8 || nal.type == 9) continue;
    appendBe32(sample, static_cast<std::uint32_t>(nal.bytes.size()));
    sample.insert(sample.end(), nal.bytes.begin(), nal.bytes.end());
  }
  return sample;
}

class VtH264Decoder {
public:
  using FrameCallback = void (*)(void* context,
                                 CVImageBufferRef imageBuffer,
                                 const DecodedVideoFrameMetadata& metadata);

  VtH264Decoder(FrameCallback callback = nullptr, void* callbackContext = nullptr)
    : frameCallback_(callback), frameCallbackContext_(callbackContext) {}

  ~VtH264Decoder() {
    if (session_) {
      VTDecompressionSessionWaitForAsynchronousFrames(session_);
      VTDecompressionSessionInvalidate(session_);
      CFRelease(session_);
    }
    if (format_) {
      CFRelease(format_);
    }
  }

  void observeParameterSets(const std::vector<NalUnit>& nals) {
    bool changed = false;
    for (const auto& nal : nals) {
      if (nal.type == 7 && nal.bytes != sps_) {
        sps_ = nal.bytes;
        changed = true;
      } else if (nal.type == 8 && nal.bytes != pps_) {
        pps_ = nal.bytes;
        changed = true;
      }
    }
    if (changed) {
      resetSession();
    }
  }

  bool ready() const {
    return !sps_.empty() && !pps_.empty();
  }

  bool decode(const SnvPacket& packet, const std::vector<std::uint8_t>& avccSample) {
    if (!ready() || avccSample.empty()) return false;
    ensureSession(packet.width, packet.height);

    CMBlockBufferRef blockBuffer = nullptr;
    OSStatus status = CMBlockBufferCreateWithMemoryBlock(kCFAllocatorDefault,
                                                         nullptr,
                                                         avccSample.size(),
                                                         kCFAllocatorDefault,
                                                         nullptr,
                                                         0,
                                                         avccSample.size(),
                                                         0,
                                                         &blockBuffer);
    if (status != noErr) {
      ++decodeErrors_;
      std::cerr << "CMBlockBufferCreate failed: " << osStatusString(status) << "\n";
      return false;
    }

    status = CMBlockBufferReplaceDataBytes(avccSample.data(), blockBuffer, 0, avccSample.size());
    if (status != noErr) {
      CFRelease(blockBuffer);
      ++decodeErrors_;
      std::cerr << "CMBlockBufferReplaceDataBytes failed: " << osStatusString(status) << "\n";
      return false;
    }

    CMSampleTimingInfo timing{};
    timing.presentationTimeStamp = CMTimeMake(static_cast<int64_t>(packet.timestampMicros), 1000000);
    timing.duration = CMTimeMake(static_cast<int64_t>(packet.durationMicros ? packet.durationMicros : 16667), 1000000);
    timing.decodeTimeStamp = kCMTimeInvalid;

    CMSampleBufferRef sampleBuffer = nullptr;
    status = CMSampleBufferCreateReady(kCFAllocatorDefault,
                                       blockBuffer,
                                       format_,
                                       1,
                                       1,
                                       &timing,
                                       0,
                                       nullptr,
                                       &sampleBuffer);
    CFRelease(blockBuffer);
    if (status != noErr) {
      ++decodeErrors_;
      std::cerr << "CMSampleBufferCreateReady failed: " << osStatusString(status) << "\n";
      return false;
    }

    auto* frameMetadata = new DecodedVideoFrameMetadata{
      packet.sequence,
      packet.timestampMicros,
      packet.durationMicros ? packet.durationMicros : 16667,
      packet.hostUnixMicros,
      steadyMicros()
    };

    VTDecodeInfoFlags infoFlags = 0;
    const VTDecodeFrameFlags decodeFlags = kVTDecodeFrame_EnableAsynchronousDecompression
                                         | kVTDecodeFrame_1xRealTimePlayback;
    status = VTDecompressionSessionDecodeFrame(session_, sampleBuffer, decodeFlags, frameMetadata, &infoFlags);
    CFRelease(sampleBuffer);
    if (status != noErr) {
      delete frameMetadata;
      ++decodeErrors_;
      std::cerr << "VTDecompressionSessionDecodeFrame failed at packet "
                << packet.sequence << ": " << osStatusString(status) << "\n";
      return false;
    }
    submittedFrames_ += 1;
    return true;
  }

  void flush() {
    if (session_) {
      VTDecompressionSessionWaitForAsynchronousFrames(session_);
    }
  }

  std::uint64_t decodedFrames() const { return decodedFrames_; }
  std::uint64_t submittedFrames() const { return submittedFrames_; }
  std::uint64_t decodeErrors() const { return decodeErrors_; }

private:
  void resetSession() {
    if (session_) {
      VTDecompressionSessionWaitForAsynchronousFrames(session_);
      VTDecompressionSessionInvalidate(session_);
      CFRelease(session_);
      session_ = nullptr;
    }
    if (format_) {
      CFRelease(format_);
      format_ = nullptr;
    }
  }

  void ensureSession(std::uint32_t width, std::uint32_t height) {
    if (session_) return;

    const uint8_t* parameterSets[] = { sps_.data(), pps_.data() };
    const size_t parameterSetSizes[] = { sps_.size(), pps_.size() };
    checkStatus(CMVideoFormatDescriptionCreateFromH264ParameterSets(kCFAllocatorDefault,
                                                                    2,
                                                                    parameterSets,
                                                                    parameterSetSizes,
                                                                    4,
                                                                    &format_),
                "CMVideoFormatDescriptionCreateFromH264ParameterSets");

    VTDecompressionOutputCallbackRecord callback{};
    callback.decompressionOutputCallback = &VtH264Decoder::outputCallback;
    callback.decompressionOutputRefCon = this;

    NSDictionary* attributes = @{
      (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange),
      (id)kCVPixelBufferWidthKey: @(width),
      (id)kCVPixelBufferHeightKey: @(height)
    };

    NSDictionary* decoderSpecification = @{
      (id)kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder: @YES
    };

    checkStatus(VTDecompressionSessionCreate(kCFAllocatorDefault,
                                             format_,
                                             (__bridge CFDictionaryRef)decoderSpecification,
                                             (__bridge CFDictionaryRef)attributes,
                                             &callback,
                                             &session_),
                "VTDecompressionSessionCreate");
    VTSessionSetProperty(session_, kVTDecompressionPropertyKey_RealTime, kCFBooleanTrue);
  }

  static void outputCallback(void* decompressionOutputRefCon,
                             void* sourceFrameRefCon,
                             OSStatus status,
                             VTDecodeInfoFlags infoFlags,
                             CVImageBufferRef imageBuffer,
                             CMTime presentationTimeStamp,
                             CMTime presentationDuration) {
    (void)infoFlags;
    std::unique_ptr<DecodedVideoFrameMetadata> metadata(
      static_cast<DecodedVideoFrameMetadata*>(sourceFrameRefCon));
    DecodedVideoFrameMetadata fallbackMetadata;
    if (!metadata) {
      if (CMTIME_IS_VALID(presentationTimeStamp)) {
        fallbackMetadata.timestampMicros = static_cast<std::uint64_t>(
          CMTimeGetSeconds(presentationTimeStamp) * 1000000.0);
      }
      if (CMTIME_IS_VALID(presentationDuration)) {
        fallbackMetadata.durationMicros = static_cast<std::uint64_t>(
          CMTimeGetSeconds(presentationDuration) * 1000000.0);
      }
      fallbackMetadata.decodeSubmittedAtMicros = steadyMicros();
    }
    const DecodedVideoFrameMetadata& frameMetadata = metadata ? *metadata : fallbackMetadata;
    auto* self = static_cast<VtH264Decoder*>(decompressionOutputRefCon);
    if (status == noErr && imageBuffer) {
      self->decodedFrames_ += 1;
      if (self->frameCallback_) {
        self->frameCallback_(self->frameCallbackContext_, imageBuffer, frameMetadata);
      }
      if (self->decodedFrames_ == 1) {
        self->decodedWidth_ = static_cast<std::uint32_t>(CVPixelBufferGetWidth(imageBuffer));
        self->decodedHeight_ = static_cast<std::uint32_t>(CVPixelBufferGetHeight(imageBuffer));
      }
    } else {
      self->decodeErrors_ += 1;
    }
  }

  std::vector<std::uint8_t> sps_;
  std::vector<std::uint8_t> pps_;
  CMVideoFormatDescriptionRef format_ = nullptr;
  VTDecompressionSessionRef session_ = nullptr;
  std::uint64_t submittedFrames_ = 0;
  std::uint64_t decodedFrames_ = 0;
  std::uint64_t decodeErrors_ = 0;
  std::uint32_t decodedWidth_ = 0;
  std::uint32_t decodedHeight_ = 0;
  FrameCallback frameCallback_ = nullptr;
  void* frameCallbackContext_ = nullptr;
};

struct DecodeSummary {
  std::uint64_t packets = 0;
  std::uint64_t keyframes = 0;
  std::uint64_t nalUnits = 0;
  std::uint64_t skippedNoParameters = 0;
  std::uint64_t skippedEmptySamples = 0;
  std::uint64_t firstSequence = 0;
  std::uint64_t lastSequence = 0;
  bool hasFirstSequence = false;
};

struct ClientStreamStats {
  std::uint64_t windowPackets = 0;
  std::uint64_t windowDropped = 0;
  std::uint64_t totalDropped = 0;
  std::uint64_t lastSequence = 0;
  std::uint64_t lastArrivalMicros = 0;
  bool hasLastSequence = false;
  double jitterMs = 0.0;
  double ageSumMs = 0.0;
  double ageMaxMs = 0.0;
  std::uint64_t ageSamples = 0;

  void observe(const SnvPacket& packet) {
    ++windowPackets;
    if (hasLastSequence && packet.sequence > lastSequence + 1) {
      const std::uint64_t missed = packet.sequence - lastSequence - 1;
      windowDropped += missed;
      totalDropped += missed;
    }
    lastSequence = packet.sequence;
    hasLastSequence = true;

    const std::uint64_t arrival = steadyMicros();
    if (lastArrivalMicros > 0 && packet.durationMicros > 0) {
      const double actualMs = static_cast<double>(arrival - lastArrivalMicros) / 1000.0;
      const double expectedMs = static_cast<double>(packet.durationMicros) / 1000.0;
      const double sampleJitter = std::abs(actualMs - expectedMs);
      jitterMs = jitterMs == 0.0 ? sampleJitter : (jitterMs * 0.85 + sampleJitter * 0.15);
    }
    lastArrivalMicros = arrival;

    if (packet.hostUnixMicros > 0) {
      const double ageMs = static_cast<double>(unixMicros() - packet.hostUnixMicros) / 1000.0;
      if (ageMs > -5000.0 && ageMs < 600000.0) {
        ageSumMs += ageMs;
        ageMaxMs = std::max(ageMaxMs, ageMs);
        ageSamples += 1;
      }
    }
  }

  void resetWindow() {
    windowPackets = 0;
    windowDropped = 0;
    ageSumMs = 0.0;
    ageMaxMs = 0.0;
    ageSamples = 0;
  }

  void resetSequencing() {
    windowPackets = 0;
    windowDropped = 0;
    lastSequence = 0;
    lastArrivalMicros = 0;
    hasLastSequence = false;
    jitterMs = 0.0;
    ageSumMs = 0.0;
    ageMaxMs = 0.0;
    ageSamples = 0;
  }
};

void processSnvPacket(VtH264Decoder& decoder, DecodeSummary& summary, const SnvPacket& packet) {
  if (packet.codec != 1) {
    throw std::runtime_error("Only H.264 SNV1 codec packets are supported.");
  }
  if (!summary.hasFirstSequence) {
    summary.firstSequence = packet.sequence;
    summary.hasFirstSequence = true;
  }
  summary.lastSequence = packet.sequence;
  summary.packets += 1;
  if (packet.flags & 1) ++summary.keyframes;

  const auto nals = parseNalUnits(packet.payload);
  summary.nalUnits += nals.size();
  decoder.observeParameterSets(nals);
  if (!decoder.ready()) {
    ++summary.skippedNoParameters;
    return;
  }

  const auto sample = buildAvccSample(nals);
  if (sample.empty()) {
    ++summary.skippedEmptySamples;
    return;
  }
  decoder.decode(packet, sample);
}

void printDecodeSummary(const std::string& label,
                        const DecodeSummary& summary,
                        const VtH264Decoder& decoder) {
  std::cout << "{\n"
            << "  \"source\": \"" << jsonEscape(label.c_str()) << "\",\n"
            << "  \"packets\": " << summary.packets << ",\n"
            << "  \"keyframes\": " << summary.keyframes << ",\n"
            << "  \"nalUnits\": " << summary.nalUnits << ",\n"
            << "  \"submittedFrames\": " << decoder.submittedFrames() << ",\n"
            << "  \"decodedFrames\": " << decoder.decodedFrames() << ",\n"
            << "  \"decodeErrors\": " << decoder.decodeErrors() << ",\n"
            << "  \"skippedNoParameters\": " << summary.skippedNoParameters << ",\n"
            << "  \"skippedEmptySamples\": " << summary.skippedEmptySamples << ",\n"
            << "  \"firstSequence\": " << summary.firstSequence << ",\n"
            << "  \"lastSequence\": " << summary.lastSequence << "\n"
            << "}\n";
}

int decodeSnvFile(const std::string& file, std::uint64_t maxPackets) {
  @autoreleasepool {
    const auto packets = readSnvPackets(file, maxPackets);
    VtH264Decoder decoder;
    DecodeSummary summary;

    for (const auto& packet : packets) {
      processSnvPacket(decoder, summary, packet);
    }

    decoder.flush();
    printDecodeSummary(file, summary, decoder);

    if (decoder.decodedFrames() == 0) {
      std::cerr << "No frames decoded. The SNV file may not contain SPS/PPS in H.264 Annex B or AVCC form yet.\n";
      return 2;
    }
  }
  return 0;
}

bool readExactFd(int fd, void* target, std::size_t size) {
  auto* cursor = static_cast<std::uint8_t*>(target);
  std::size_t remaining = size;
  while (remaining > 0) {
    const ssize_t received = recv(fd, cursor, remaining, 0);
    if (received == 0) return false;
    if (received < 0) {
      if (errno == EINTR) continue;
      throw std::runtime_error("Socket receive failed.");
    }
    cursor += received;
    remaining -= static_cast<std::size_t>(received);
  }
  return true;
}

bool readControlPayloadFromFd(int fd, std::string& payload) {
  std::array<std::uint8_t, 12> header{};
  if (!readExactFd(fd, header.data(), header.size())) return false;
  if (std::memcmp(header.data(), "SNI1", 4) != 0) {
    throw std::runtime_error("Invalid SNI1 control magic from host.");
  }

  const std::uint32_t headerSize = readLe32Raw(header.data() + 4);
  const std::uint32_t payloadSize = readLe32Raw(header.data() + 8);
  if (headerSize < header.size()) {
    throw std::runtime_error("Invalid SNI1 control header size from host.");
  }
  if (payloadSize > 4 * 1024 * 1024) {
    throw std::runtime_error("SNI1 control payload too large from host.");
  }
  if (headerSize > header.size()) {
    std::vector<std::uint8_t> extra(headerSize - header.size());
    if (!readExactFd(fd, extra.data(), extra.size())) return false;
  }

  payload.assign(payloadSize, '\0');
  if (!payload.empty() && !readExactFd(fd, payload.data(), payload.size())) return false;
  return true;
}

bool readSnvPacketFromFd(int fd, SnvPacket& packet) {
  std::array<std::uint8_t, 52> header{};
  if (!readExactFd(fd, header.data(), header.size())) {
    return false;
  }
  if (std::memcmp(header.data(), "SNV1", 4) != 0) {
    throw std::runtime_error("Invalid SNV1 magic from TCP stream.");
  }

  const std::uint32_t headerSize = readLe32Raw(header.data() + 4);
  if (headerSize < header.size()) {
    throw std::runtime_error("Invalid SNV1 header size from TCP stream.");
  }
  packet = parseSnvHeader(header.data(), header.size());

  if (headerSize > header.size()) {
    std::vector<std::uint8_t> extraHeader(headerSize - header.size());
    if (!readExactFd(fd, extraHeader.data(), extraHeader.size())) {
      throw std::runtime_error("TCP stream ended inside SNV1 extended header.");
    }
    if (extraHeader.size() >= 8) {
      packet.hostUnixMicros = readLe64Raw(extraHeader.data());
    }
  }

  const std::uint32_t payloadSize = readLe32Raw(header.data() + 48);
  packet.payload.resize(payloadSize);
  if (payloadSize > 0 && !readExactFd(fd, packet.payload.data(), packet.payload.size())) {
    throw std::runtime_error("TCP stream ended inside SNV1 payload.");
  }
  return true;
}

struct UdpVideoStats {
  std::uint64_t datagrams = 0;
  std::uint64_t fragments = 0;
  std::uint64_t retransmitFragments = 0;
  std::uint64_t completedPackets = 0;
  std::uint64_t retransmitCompletedPackets = 0;
  std::uint64_t droppedAssemblies = 0;
  std::uint64_t malformedDatagrams = 0;
  std::uint64_t authDatagrams = 0;
  std::uint64_t authRejectedDatagrams = 0;
  std::uint64_t replayRejectedDatagrams = 0;
  std::uint64_t peerRejectedDatagrams = 0;
  std::uint64_t rekeyGraceDatagrams = 0;
  std::uint64_t rekeyGraceCompletedPackets = 0;
  std::uint64_t epochResetAssemblies = 0;
  std::uint64_t duplicateFragments = 0;
  std::vector<std::uint64_t> nackPacketIds;
  std::vector<std::uint64_t> newNackPacketIds;
};

struct UdpAssemblyKey {
  std::uint64_t mediaEpoch = 0;
  std::uint64_t packetId = 0;

  bool operator<(const UdpAssemblyKey& other) const {
    if (mediaEpoch != other.mediaEpoch) return mediaEpoch < other.mediaEpoch;
    return packetId < other.packetId;
  }
};

struct UdpPacketAssembly {
  std::vector<std::uint8_t> data;
  std::vector<std::uint8_t> received;
  std::chrono::steady_clock::time_point createdAt;
  std::uint32_t receivedBytes = 0;
  std::uint16_t receivedFragments = 0;
  std::uint16_t fragmentCount = 0;
  std::uint64_t mediaEpoch = 0;
  bool rekeyGrace = false;
  bool hadRetransmit = false;
};

class UdpVideoReassembler {
public:
  bool push(std::vector<std::uint8_t> datagram,
            std::vector<std::uint8_t>& packetBytes,
            UdpVideoStats& stats,
            std::uint64_t& completedPacketId,
            std::uint64_t& completedMediaEpoch,
            bool& completedRekeyGrace,
            const sockaddr_in& peerAddress,
            UdpPeerLock& peerLock) {
    completedPacketId = 0;
    completedMediaEpoch = 0;
    completedRekeyGrace = false;
    const std::uint64_t mediaGeneration = gMediaPeerGeneration.load(std::memory_order_relaxed);
    peerLock.observeGeneration(mediaGeneration);
    observeMediaGeneration(mediaGeneration, stats);
    if (peerLock.isLocked() && !peerLock.acceptKnownPeer(peerAddress)) {
      stats.peerRejectedDatagrams += 1;
      return false;
    }
    stats.datagrams += 1;
    if (datagram.size() < sizeof(UdpVideoFragmentHeader)) {
      stats.malformedDatagrams += 1;
      return false;
    }

    UdpVideoFragmentHeader header{};
    UdpVideoAuthenticatedFragmentHeader authenticatedHeader{};
    ResolvedMediaCrypto videoCrypto;
    std::uint64_t fragmentMediaEpoch = 0;
    bool authenticatedVideo = false;
    if (std::memcmp(datagram.data(), "SNU2", 4) == 0) {
      if (datagram.size() < sizeof(UdpVideoAuthenticatedFragmentHeader)) {
        stats.malformedDatagrams += 1;
        return false;
      }
      std::memcpy(&authenticatedHeader, datagram.data(), sizeof(authenticatedHeader));
      fragmentMediaEpoch = authenticatedHeader.authEpoch;
      videoCrypto = resolveMediaCryptoForEpoch(authenticatedHeader.authEpoch, true);
      if (!videoCrypto.enabled) {
        stats.authRejectedDatagrams += 1;
        return false;
      }
      if (!verifyUdpDatagramMac(videoCrypto.authKey,
                                "video",
                                datagram,
                                offsetof(UdpVideoAuthenticatedFragmentHeader, authTag))) {
        stats.authRejectedDatagrams += 1;
        return false;
      }
      authenticatedVideo = true;
      std::memcpy(header.magic, "SNU1", 4);
      header.headerSize = authenticatedHeader.headerSize;
      header.fragmentCount = authenticatedHeader.fragmentCount;
      header.fragmentIndex = authenticatedHeader.fragmentIndex;
      header.flags = authenticatedHeader.flags;
      header.packetId = authenticatedHeader.packetId;
      header.packetSize = authenticatedHeader.packetSize;
      header.fragmentOffset = authenticatedHeader.fragmentOffset;
      header.payloadSize = authenticatedHeader.payloadSize;
      stats.authDatagrams += 1;
      if (videoCrypto.rekeyGrace) {
        stats.rekeyGraceDatagrams += 1;
      }
    } else if (std::memcmp(datagram.data(), "SNU1", 4) == 0) {
      if (!gExpectedSessionToken.empty()) {
        stats.authRejectedDatagrams += 1;
        return false;
      }
      std::memcpy(&header, datagram.data(), sizeof(header));
      fragmentMediaEpoch = 0;
    } else {
      stats.malformedDatagrams += 1;
      return false;
    }

    if (std::memcmp(header.magic, "SNU1", 4) != 0 ||
        header.headerSize < (authenticatedVideo ? sizeof(UdpVideoAuthenticatedFragmentHeader) : sizeof(UdpVideoFragmentHeader)) ||
        header.headerSize > datagram.size() ||
        header.fragmentCount == 0 ||
        header.fragmentIndex >= header.fragmentCount ||
        header.packetSize == 0 ||
        header.payloadSize == 0 ||
        static_cast<std::size_t>(header.headerSize) + header.payloadSize > datagram.size() ||
        static_cast<std::uint64_t>(header.fragmentOffset) + header.payloadSize > header.packetSize) {
      stats.malformedDatagrams += 1;
      return false;
    }
    if (!peerLock.lockOrAccept(peerAddress)) {
      stats.peerRejectedDatagrams += 1;
      return false;
    }
    if (authenticatedVideo) {
      MediaReplayWindow& replayWindow = videoCrypto.rekeyGrace
        ? gPreviousVideoMediaReplay
        : gVideoMediaReplay;
      if (!replayWindow.accept(authenticatedHeader.authEpoch, authenticatedHeader.authSeq)) {
        stats.replayRejectedDatagrams += 1;
        return false;
      }
      chacha20Xor(datagram.data() + header.headerSize,
                  header.payloadSize,
                  videoCrypto.cryptoKey,
                  "video",
                  authenticatedHeader.authSeq);
    }

    prune(stats);
    auto highestIt = highestPacketIdByEpoch_.find(fragmentMediaEpoch);
    const bool hasHighestPacketId = highestIt != highestPacketIdByEpoch_.end();
    const std::uint64_t highestPacketId = hasHighestPacketId ? highestIt->second : 0;
    if (hasHighestPacketId && header.packetId > highestPacketId + 1) {
      for (std::uint64_t missing = highestPacketId + 1;
           missing < header.packetId && missing - highestPacketId <= 64;
           ++missing) {
        recordNackPacket(stats, missing);
      }
    }
    if (!hasHighestPacketId || header.packetId > highestPacketId) {
      highestPacketIdByEpoch_[fragmentMediaEpoch] = header.packetId;
    }
    stats.fragments += 1;
    if (header.flags & 4u) {
      stats.retransmitFragments += 1;
    }
    const UdpAssemblyKey assemblyKey{fragmentMediaEpoch, header.packetId};
    auto& assembly = assemblies_[assemblyKey];
    if (assembly.data.empty() ||
        assembly.data.size() != header.packetSize ||
        assembly.fragmentCount != header.fragmentCount) {
      if (!assembly.data.empty()) {
        stats.droppedAssemblies += 1;
        recordNackPacket(stats, header.packetId);
      }
      assembly.data.assign(header.packetSize, 0);
      assembly.received.assign(header.fragmentCount, 0);
      assembly.createdAt = std::chrono::steady_clock::now();
      assembly.receivedBytes = 0;
      assembly.receivedFragments = 0;
      assembly.fragmentCount = header.fragmentCount;
      assembly.mediaEpoch = fragmentMediaEpoch;
      assembly.rekeyGrace = authenticatedVideo && videoCrypto.rekeyGrace;
      assembly.hadRetransmit = false;
    }
    if (header.flags & 4u) {
      assembly.hadRetransmit = true;
    }

    if (assembly.received[header.fragmentIndex]) {
      stats.duplicateFragments += 1;
      return false;
    }

    const auto* payload = datagram.data() + header.headerSize;
    std::memcpy(assembly.data.data() + header.fragmentOffset, payload, header.payloadSize);
    assembly.received[header.fragmentIndex] = 1;
    assembly.receivedBytes += header.payloadSize;
    assembly.receivedFragments += 1;

    if (assembly.receivedFragments == assembly.fragmentCount &&
        assembly.receivedBytes == assembly.data.size()) {
      packetBytes = std::move(assembly.data);
      if (assembly.hadRetransmit) {
        stats.retransmitCompletedPackets += 1;
      }
      completedPacketId = header.packetId;
      completedMediaEpoch = assembly.mediaEpoch;
      completedRekeyGrace = assembly.rekeyGrace;
      if (completedRekeyGrace) {
        stats.rekeyGraceCompletedPackets += 1;
      }
      assemblies_.erase(assemblyKey);
      stats.completedPackets += 1;
      return true;
    }

    if (assemblies_.size() > 128) {
      recordNackPacket(stats, assemblies_.begin()->first.packetId);
      assemblies_.erase(assemblies_.begin());
      stats.droppedAssemblies += 1;
    }
    return false;
  }

private:
  void observeMediaGeneration(std::uint64_t generation, UdpVideoStats& stats) {
    if (!hasMediaGeneration_) {
      mediaGeneration_ = generation;
      hasMediaGeneration_ = true;
      return;
    }
    if (generation == mediaGeneration_) return;

    const std::size_t dropped = assemblies_.size();
    if (dropped > 0) {
      stats.epochResetAssemblies += dropped;
      stats.droppedAssemblies += dropped;
    }
    assemblies_.clear();
    highestPacketIdByEpoch_.clear();
    std::cout << "SNU1_EPOCH_RESET generation=" << mediaGeneration_
              << " nextGeneration=" << generation
              << " droppedAssemblies=" << dropped
              << "\n";
    mediaGeneration_ = generation;
  }

  void recordNackPacket(UdpVideoStats& stats, std::uint64_t packetId) {
    if (packetId == 0) return;
    if (stats.nackPacketIds.size() < 64 &&
        std::find(stats.nackPacketIds.begin(), stats.nackPacketIds.end(), packetId) == stats.nackPacketIds.end()) {
      stats.nackPacketIds.push_back(packetId);
    }
    if (stats.newNackPacketIds.size() >= 64) {
      return;
    }
    if (std::find(stats.newNackPacketIds.begin(), stats.newNackPacketIds.end(), packetId) != stats.newNackPacketIds.end()) {
      return;
    }
    stats.newNackPacketIds.push_back(packetId);
  }

  void prune(UdpVideoStats& stats) {
    const auto now = std::chrono::steady_clock::now();
    for (auto it = assemblies_.begin(); it != assemblies_.end();) {
      if (now - it->second.createdAt > std::chrono::milliseconds(500)) {
        recordNackPacket(stats, it->first.packetId);
        it = assemblies_.erase(it);
        stats.droppedAssemblies += 1;
      } else {
        ++it;
      }
    }
  }

  std::map<UdpAssemblyKey, UdpPacketAssembly> assemblies_;
  std::map<std::uint64_t, std::uint64_t> highestPacketIdByEpoch_;
  std::uint64_t mediaGeneration_ = 0;
  bool hasMediaGeneration_ = false;
};

struct UdpVideoNackWindowStats {
  std::uint64_t sentBatches = 0;
  std::uint64_t sentPackets = 0;
  std::uint64_t recoveredPackets = 0;
  std::uint64_t timedOutPackets = 0;
};

class UdpVideoNackController {
public:
  bool resetForMediaGeneration(std::uint64_t generation) {
    if (!hasMediaGeneration_) {
      mediaGeneration_ = generation;
      hasMediaGeneration_ = true;
      return false;
    }
    if (generation == mediaGeneration_) return false;

    const std::size_t dropped = pending_.size();
    pending_.clear();
    std::cout << "SNU1_NACK_RESET generation=" << mediaGeneration_
              << " nextGeneration=" << generation
              << " droppedPending=" << dropped
              << "\n";
    mediaGeneration_ = generation;
    return true;
  }

  void observeMissing(const std::vector<std::uint64_t>& packetIds) {
    if (packetIds.empty()) return;
    const auto now = std::chrono::steady_clock::now();
    for (std::uint64_t packetId : packetIds) {
      if (packetId == 0 || pending_.find(packetId) != pending_.end()) continue;
      if (pending_.size() >= maxPendingPackets_) {
        pending_.erase(pending_.begin());
        windowTimedOutPackets_ += 1;
      }
      pending_.emplace(packetId, MissingPacket{now, {}, 0});
    }
  }

  void markRecovered(std::uint64_t packetId) {
    if (packetId == 0) return;
    const auto erased = pending_.erase(packetId);
    if (erased > 0) {
      windowRecoveredPackets_ += 1;
    }
  }

  bool maybeSend(const std::string& reason) {
    if (pending_.empty()) return false;
    const auto now = std::chrono::steady_clock::now();
    std::vector<std::uint64_t> packetIds;
    for (const auto& item : pending_) {
      const MissingPacket& missing = item.second;
      const bool neverSent = missing.sendCount == 0;
      const bool retryDue = missing.sendCount < maxRetries_ &&
        now - missing.lastSentAt >= std::chrono::milliseconds(80);
      if (neverSent || retryDue) {
        packetIds.push_back(item.first);
        if (packetIds.size() >= maxBatchPackets_) break;
      }
    }
    if (packetIds.empty()) return false;
    if (!sendVideoNackRequest(reason, packetIds)) return false;

    const auto sentAt = std::chrono::steady_clock::now();
    for (std::uint64_t packetId : packetIds) {
      auto found = pending_.find(packetId);
      if (found == pending_.end()) continue;
      found->second.lastSentAt = sentAt;
      found->second.sendCount += 1;
    }
    windowSentBatches_ += 1;
    windowSentPackets_ += packetIds.size();
    return true;
  }

  bool maybeRequestKeyframe(std::uint64_t videoSequence,
                            std::uint64_t decodeErrors) {
    if (pending_.empty()) return false;
    const auto now = std::chrono::steady_clock::now();
    bool needsKeyframe = pending_.size() >= maxPendingPackets_;
    for (const auto& item : pending_) {
      const MissingPacket& missing = item.second;
      if ((missing.sendCount >= maxRetries_ &&
           now - missing.firstSeenAt >= std::chrono::milliseconds(320)) ||
          now - missing.firstSeenAt >= std::chrono::milliseconds(700)) {
        needsKeyframe = true;
        break;
      }
    }
    if (!needsKeyframe) return false;

    const std::uint64_t missingPackets = pending_.size();
    const bool sent = sendKeyframeRequest("udp-nack-timeout",
                                          videoSequence,
                                          missingPackets,
                                          decodeErrors);
    if (sent) {
      windowTimedOutPackets_ += missingPackets;
      pending_.clear();
    }
    return sent;
  }

  std::size_t pendingCount() const {
    return pending_.size();
  }

  UdpVideoNackWindowStats takeWindowStats() {
    UdpVideoNackWindowStats stats;
    stats.sentBatches = windowSentBatches_;
    stats.sentPackets = windowSentPackets_;
    stats.recoveredPackets = windowRecoveredPackets_;
    stats.timedOutPackets = windowTimedOutPackets_;
    windowSentBatches_ = 0;
    windowSentPackets_ = 0;
    windowRecoveredPackets_ = 0;
    windowTimedOutPackets_ = 0;
    return stats;
  }

private:
  struct MissingPacket {
    std::chrono::steady_clock::time_point firstSeenAt;
    std::chrono::steady_clock::time_point lastSentAt;
    std::uint32_t sendCount = 0;
  };

  std::map<std::uint64_t, MissingPacket> pending_;
  std::uint64_t windowSentBatches_ = 0;
  std::uint64_t windowSentPackets_ = 0;
  std::uint64_t windowRecoveredPackets_ = 0;
  std::uint64_t windowTimedOutPackets_ = 0;
  std::uint64_t mediaGeneration_ = 0;
  bool hasMediaGeneration_ = false;
  static constexpr std::size_t maxPendingPackets_ = 128;
  static constexpr std::size_t maxBatchPackets_ = 32;
  static constexpr std::uint32_t maxRetries_ = 3;
};

struct UdpVideoJitterWindowStats {
  std::uint64_t releasedPackets = 0;
  std::uint64_t heldTicks = 0;
  std::uint64_t skippedSequences = 0;
  std::uint64_t lateDrops = 0;
  std::uint64_t duplicateDrops = 0;
  std::uint64_t keyframeResets = 0;
  std::uint64_t generationResets = 0;
  std::uint64_t epochStaleDrops = 0;
  std::uint64_t epochGraceReleases = 0;
};

class UdpVideoPacketJitterBuffer {
public:
  bool resetForMediaGeneration(std::uint64_t generation) {
    if (!hasMediaGeneration_) {
      mediaGeneration_ = generation;
      hasMediaGeneration_ = true;
      return false;
    }
    if (generation == mediaGeneration_) return false;

    const std::size_t dropped = packets_.size();
    packets_.clear();
    nextSequence_ = 0;
    hasNextSequence_ = false;
    activeMediaEpoch_ = 0;
    hasActiveMediaEpoch_ = false;
    windowGenerationResets_ += 1;
    std::cout << "SNU1_JITTER_RESET reason=media-generation"
              << " generation=" << mediaGeneration_
              << " nextGeneration=" << generation
              << " droppedPackets=" << dropped
              << "\n";
    mediaGeneration_ = generation;
    return true;
  }

  bool push(SnvPacket&& packet, std::uint64_t mediaEpoch, bool rekeyGrace) {
    const auto now = std::chrono::steady_clock::now();
    if (hasActiveMediaEpoch_ && mediaEpoch < activeMediaEpoch_) {
      windowEpochStaleDrops_ += 1;
      return false;
    }
    if (!rekeyGrace && (!hasActiveMediaEpoch_ || mediaEpoch > activeMediaEpoch_)) {
      activeMediaEpoch_ = mediaEpoch;
      hasActiveMediaEpoch_ = true;
      dropStaleEpochPackets();
    }
    if (hasNextSequence_ && packet.sequence < nextSequence_) {
      windowLateDrops_ += 1;
      return false;
    }
    if ((packet.flags & 1u) && hasNextSequence_ &&
        packet.sequence > nextSequence_ + maxBufferedPackets_) {
      packets_.clear();
      nextSequence_ = packet.sequence;
      windowKeyframeResets_ += 1;
    }

    const std::uint64_t sequence = packet.sequence;
    auto inserted = packets_.emplace(sequence, Entry{std::move(packet), now, mediaEpoch, rekeyGrace});
    if (!inserted.second) {
      windowDuplicateDrops_ += 1;
      return false;
    }
    return true;
  }

  bool popReady(SnvPacket& packet) {
    if (packets_.empty()) return false;
    dropStaleEpochPackets();
    if (packets_.empty()) return false;
    if (!hasNextSequence_) {
      nextSequence_ = packets_.begin()->first;
      hasNextSequence_ = true;
    }

    auto exact = packets_.find(nextSequence_);
    if (exact != packets_.end()) {
      const bool rekeyGrace = exact->second.rekeyGrace;
      packet = std::move(exact->second.packet);
      packets_.erase(exact);
      nextSequence_ = packet.sequence + 1;
      windowReleasedPackets_ += 1;
      if (rekeyGrace) {
        windowEpochGraceReleases_ += 1;
      }
      return true;
    }

    const auto now = std::chrono::steady_clock::now();
    auto oldest = packets_.begin();
    const bool holdExpired = now - oldest->second.arrivedAt >= holdDuration_;
    const bool bufferFull = packets_.size() >= maxBufferedPackets_;
    if (!holdExpired && !bufferFull) {
      windowHeldTicks_ += 1;
      return false;
    }

    if (oldest->first > nextSequence_) {
      windowSkippedSequences_ += oldest->first - nextSequence_;
      nextSequence_ = oldest->first;
    }
    const bool rekeyGrace = oldest->second.rekeyGrace;
    packet = std::move(oldest->second.packet);
    packets_.erase(oldest);
    nextSequence_ = packet.sequence + 1;
    windowReleasedPackets_ += 1;
    if (rekeyGrace) {
      windowEpochGraceReleases_ += 1;
    }
    return true;
  }

  std::size_t pendingCount() const {
    return packets_.size();
  }

  UdpVideoJitterWindowStats takeWindowStats() {
    UdpVideoJitterWindowStats stats;
    stats.releasedPackets = windowReleasedPackets_;
    stats.heldTicks = windowHeldTicks_;
    stats.skippedSequences = windowSkippedSequences_;
    stats.lateDrops = windowLateDrops_;
    stats.duplicateDrops = windowDuplicateDrops_;
    stats.keyframeResets = windowKeyframeResets_;
    stats.generationResets = windowGenerationResets_;
    stats.epochStaleDrops = windowEpochStaleDrops_;
    stats.epochGraceReleases = windowEpochGraceReleases_;
    windowReleasedPackets_ = 0;
    windowHeldTicks_ = 0;
    windowSkippedSequences_ = 0;
    windowLateDrops_ = 0;
    windowDuplicateDrops_ = 0;
    windowKeyframeResets_ = 0;
    windowGenerationResets_ = 0;
    windowEpochStaleDrops_ = 0;
    windowEpochGraceReleases_ = 0;
    return stats;
  }

private:
  struct Entry {
    SnvPacket packet;
    std::chrono::steady_clock::time_point arrivedAt;
    std::uint64_t mediaEpoch = 0;
    bool rekeyGrace = false;
  };

  std::size_t dropStaleEpochPackets() {
    if (!hasActiveMediaEpoch_) return 0;
    std::size_t dropped = 0;
    for (auto it = packets_.begin(); it != packets_.end();) {
      if (it->second.mediaEpoch < activeMediaEpoch_) {
        it = packets_.erase(it);
        windowEpochStaleDrops_ += 1;
        dropped += 1;
      } else {
        ++it;
      }
    }
    if (dropped > 0 && packets_.empty()) {
      nextSequence_ = 0;
      hasNextSequence_ = false;
    }
    return dropped;
  }

  std::map<std::uint64_t, Entry> packets_;
  std::uint64_t nextSequence_ = 0;
  bool hasNextSequence_ = false;
  std::uint64_t windowReleasedPackets_ = 0;
  std::uint64_t windowHeldTicks_ = 0;
  std::uint64_t windowSkippedSequences_ = 0;
  std::uint64_t windowLateDrops_ = 0;
  std::uint64_t windowDuplicateDrops_ = 0;
  std::uint64_t windowKeyframeResets_ = 0;
  std::uint64_t windowGenerationResets_ = 0;
  std::uint64_t windowEpochStaleDrops_ = 0;
  std::uint64_t windowEpochGraceReleases_ = 0;
  std::uint64_t mediaGeneration_ = 0;
  bool hasMediaGeneration_ = false;
  std::uint64_t activeMediaEpoch_ = 0;
  bool hasActiveMediaEpoch_ = false;
  static constexpr std::size_t maxBufferedPackets_ = 90;
  static constexpr auto holdDuration_ = std::chrono::milliseconds(70);
};

struct UdpAudioStats {
  std::uint64_t datagrams = 0;
  std::uint64_t malformed = 0;
  std::uint64_t authDatagrams = 0;
  std::uint64_t authRejected = 0;
  std::uint64_t replayRejected = 0;
  std::uint64_t peerRejected = 0;
  std::uint64_t rekeyGrace = 0;
  std::uint64_t rekeyGraceAccepted = 0;
};

class AvSyncClock {
public:
  void observeVideoPacket(std::uint64_t hostUnixMicros) {
    if (hostUnixMicros == 0) return;
    std::lock_guard<std::mutex> lock(mutex_);
    lastVideoHostUnixMicros_ = hostUnixMicros;
    lastVideoSteadyMicros_ = steadyMicros();
    valid_ = true;
  }

  bool audioLeadMs(std::uint64_t audioHostUnixMicros, double& leadMs) const {
    if (audioHostUnixMicros == 0) return false;
    std::lock_guard<std::mutex> lock(mutex_);
    if (!valid_ || lastVideoHostUnixMicros_ == 0 || lastVideoSteadyMicros_ == 0) return false;

    const std::uint64_t nowSteady = steadyMicros();
    const std::uint64_t elapsedMicros = nowSteady >= lastVideoSteadyMicros_
      ? nowSteady - lastVideoSteadyMicros_
      : 0;
    if (elapsedMicros > 1500000) return false;

    const std::uint64_t estimatedVideoHostNow = lastVideoHostUnixMicros_ + elapsedMicros;
    leadMs = (static_cast<double>(audioHostUnixMicros) - static_cast<double>(estimatedVideoHostNow)) / 1000.0;
    return leadMs > -5000.0 && leadMs < 5000.0;
  }

private:
  mutable std::mutex mutex_;
  std::uint64_t lastVideoHostUnixMicros_ = 0;
  std::uint64_t lastVideoSteadyMicros_ = 0;
  bool valid_ = false;
};

struct AudioPayloadPacket {
  std::uint64_t sequence = 0;
  std::uint64_t mediaEpoch = 0;
  std::uint64_t hostUnixMicros = 0;
  std::uint32_t sampleRate = 0;
  std::uint16_t channels = 0;
  std::uint32_t frameCount = 0;
  double avLeadMs = 0.0;
  bool hasAvLead = false;
  bool rekeyGrace = false;
  std::vector<std::uint8_t> payload;
  std::chrono::steady_clock::time_point receivedAt;
};

struct AudioJitterStats {
  std::uint64_t submittedPackets = 0;
  std::uint64_t submittedFrames = 0;
  std::uint64_t lostPackets = 0;
  std::uint64_t duplicatePackets = 0;
  std::uint64_t reorderedPackets = 0;
  std::uint64_t staleDrops = 0;
  std::uint64_t latencyDrops = 0;
  std::uint64_t underflows = 0;
  std::uint64_t queueDrops = 0;
  std::uint64_t resets = 0;
  std::uint64_t generationResets = 0;
  std::uint64_t epochSequenceResets = 0;
  std::uint64_t ageSamples = 0;
  double ageSumMs = 0.0;
  double ageMaxMs = 0.0;
  std::uint64_t avLeadSamples = 0;
  double avLeadSumMs = 0.0;
  double avLeadMinMs = 0.0;
  double avLeadMaxMs = 0.0;
  std::uint64_t avHoldDecisions = 0;
  std::uint64_t avSyncDrops = 0;
  std::uint64_t avClockMisses = 0;
  std::uint64_t driftSamples = 0;
  double driftRatioSum = 0.0;
  double driftRatioMin = 1.0;
  double driftRatioMax = 1.0;
  std::int64_t driftFrameDelta = 0;
  std::uint64_t epochStaleDrops = 0;
  std::uint64_t epochGraceSubmitted = 0;
  double bufferMs = 0.0;
  std::size_t queuedPackets = 0;
};

struct AudioDriftCorrectionResult {
  bool adjusted = false;
  double ratio = 1.0;
  int frameDelta = 0;
};

class AudioQueuePcmPlayer {
public:
  AudioQueuePcmPlayer() = default;

  ~AudioQueuePcmPlayer() {
    AudioQueueRef oldQueue = nullptr;
    {
      std::lock_guard<std::mutex> lock(mutex_);
      oldQueue = queue_;
      queue_ = nullptr;
      freeBuffers_.clear();
    }
    disposeQueue(oldQueue);
  }

  AudioQueuePcmPlayer(const AudioQueuePcmPlayer&) = delete;
  AudioQueuePcmPlayer& operator=(const AudioQueuePcmPlayer&) = delete;

  bool submit(std::uint32_t sampleRate,
              std::uint16_t channels,
              const std::uint8_t* payload,
              std::uint32_t payloadSize,
              std::uint32_t frameCount) {
    if (sampleRate < 8000 || sampleRate > 384000 || channels == 0 || channels > 8 || frameCount == 0) {
      return false;
    }
    const std::uint32_t bytesPerFrame = static_cast<std::uint32_t>(channels) * sizeof(float);
    if (bytesPerFrame == 0 || payloadSize < frameCount * bytesPerFrame) {
      return false;
    }

    std::unique_lock<std::mutex> lock(mutex_);
    if (!queue_ || sampleRate_ != sampleRate || channels_ != channels || maxBufferBytes_ < payloadSize) {
      configureLocked(lock, sampleRate, channels, payloadSize);
    }
    if (!queue_ || freeBuffers_.empty()) {
      queueDrops_ += 1;
      return false;
    }

    AudioQueueBufferRef buffer = freeBuffers_.front();
    freeBuffers_.pop_front();
    std::memcpy(buffer->mAudioData, payload, payloadSize);
    buffer->mAudioDataByteSize = payloadSize;
    const OSStatus status = AudioQueueEnqueueBuffer(queue_, buffer, 0, nullptr);
    if (status != noErr) {
      freeBuffers_.push_back(buffer);
      queueDrops_ += 1;
      return false;
    }
    return true;
  }

  std::uint64_t takeQueueDrops() {
    std::lock_guard<std::mutex> lock(mutex_);
    const std::uint64_t value = queueDrops_;
    queueDrops_ = 0;
    return value;
  }

private:
  static void callback(void* userData, AudioQueueRef, AudioQueueBufferRef buffer) {
    auto* player = static_cast<AudioQueuePcmPlayer*>(userData);
    if (player) player->recycle(buffer);
  }

  static void disposeQueue(AudioQueueRef queue) {
    if (!queue) return;
    AudioQueueStop(queue, true);
    AudioQueueDispose(queue, true);
  }

  void configureLocked(std::unique_lock<std::mutex>& lock,
                       std::uint32_t sampleRate,
                       std::uint16_t channels,
                       std::uint32_t requiredBytes) {
    AudioQueueRef oldQueue = queue_;
    queue_ = nullptr;
    freeBuffers_.clear();
    sampleRate_ = 0;
    channels_ = 0;
    maxBufferBytes_ = 0;

    lock.unlock();
    disposeQueue(oldQueue);
    lock.lock();

    AudioStreamBasicDescription description{};
    description.mSampleRate = static_cast<Float64>(sampleRate);
    description.mFormatID = kAudioFormatLinearPCM;
    description.mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked;
    description.mBytesPerPacket = static_cast<UInt32>(channels * sizeof(float));
    description.mFramesPerPacket = 1;
    description.mBytesPerFrame = static_cast<UInt32>(channels * sizeof(float));
    description.mChannelsPerFrame = channels;
    description.mBitsPerChannel = 32;

    AudioQueueRef createdQueue = nullptr;
    checkStatus(AudioQueueNewOutput(&description, callback, this, nullptr, nullptr, 0, &createdQueue),
                "AudioQueueNewOutput");
    queue_ = createdQueue;
    sampleRate_ = sampleRate;
    channels_ = channels;
    maxBufferBytes_ = std::max<std::uint32_t>(requiredBytes, 4096);

    constexpr int bufferCount = 48;
    for (int i = 0; i < bufferCount; ++i) {
      AudioQueueBufferRef buffer = nullptr;
      checkStatus(AudioQueueAllocateBuffer(queue_, maxBufferBytes_, &buffer), "AudioQueueAllocateBuffer");
      freeBuffers_.push_back(buffer);
    }
    checkStatus(AudioQueueStart(queue_, nullptr), "AudioQueueStart");
    std::cout << "SNA1_AUDIO_PLAYBACK sampleRate=" << sampleRate_
              << " channels=" << channels_
              << " buffers=" << bufferCount
              << " bufferBytes=" << maxBufferBytes_
              << "\n";
  }

  void recycle(AudioQueueBufferRef buffer) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (!queue_) return;
    freeBuffers_.push_back(buffer);
  }

  std::mutex mutex_;
  std::deque<AudioQueueBufferRef> freeBuffers_;
  AudioQueueRef queue_ = nullptr;
  std::uint32_t sampleRate_ = 0;
  std::uint16_t channels_ = 0;
  std::uint32_t maxBufferBytes_ = 0;
  std::uint64_t queueDrops_ = 0;
};

class AudioJitterBuffer {
public:
  AudioJitterBuffer(std::shared_ptr<AudioQueuePcmPlayer> player,
                    std::shared_ptr<AvSyncClock> avClock,
                    double targetJitterMs)
    : player_(std::move(player)),
      avClock_(std::move(avClock)),
      targetJitterMs_(std::clamp(targetJitterMs, 0.0, 120.0)),
      maxDelayMs_(std::max(80.0, targetJitterMs_ * 3.0)),
      worker_(&AudioJitterBuffer::pumpLoop, this) {}

  ~AudioJitterBuffer() {
    {
      std::lock_guard<std::mutex> lock(mutex_);
      stopped_ = true;
    }
    condition_.notify_all();
    if (worker_.joinable()) worker_.join();
  }

  AudioJitterBuffer(const AudioJitterBuffer&) = delete;
  AudioJitterBuffer& operator=(const AudioJitterBuffer&) = delete;

  bool resetForMediaGeneration(std::uint64_t generation) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (!hasMediaGeneration_) {
      mediaGeneration_ = generation;
      hasMediaGeneration_ = true;
      return false;
    }
    if (generation == mediaGeneration_) return false;

    const std::size_t dropped = buffer_.size();
    buffer_.clear();
    resetSequenceTrackingLocked();
    activeMediaEpoch_ = 0;
    hasActiveMediaEpoch_ = false;
    sequencingMediaEpoch_ = 0;
    hasSequencingMediaEpoch_ = false;
    stats_.resets += 1;
    stats_.generationResets += 1;
    std::cout << "SNA1_JITTER_RESET reason=media-generation"
              << " generation=" << mediaGeneration_
              << " nextGeneration=" << generation
              << " droppedPackets=" << dropped
              << "\n";
    mediaGeneration_ = generation;
    condition_.notify_one();
    return true;
  }

  void push(AudioPayloadPacket packet) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (stopped_) return;

    if (hasActiveMediaEpoch_ && packet.mediaEpoch < activeMediaEpoch_) {
      stats_.epochStaleDrops += 1;
      return;
    }
    if (!packet.rekeyGrace && (!hasActiveMediaEpoch_ || packet.mediaEpoch > activeMediaEpoch_)) {
      activeMediaEpoch_ = packet.mediaEpoch;
      hasActiveMediaEpoch_ = true;
      dropStaleEpochPacketsLocked();
    }
    switchSequencingMediaEpochLocked(packet.mediaEpoch);

    if ((sampleRate_ != 0 && sampleRate_ != packet.sampleRate) ||
        (channels_ != 0 && channels_ != packet.channels)) {
      buffer_.clear();
      resetSequenceTrackingLocked();
      stats_.resets += 1;
      std::cout << "SNA1_JITTER_RESET sampleRate=" << packet.sampleRate
                << " channels=" << packet.channels
                << "\n";
    }
    sampleRate_ = packet.sampleRate;
    channels_ = packet.channels;

    if (hasExpectedSequence_ && packet.sequence < expectedSequence_) {
      stats_.staleDrops += 1;
      return;
    }
    if (hasHighestSequence_ && packet.sequence < highestSequence_) {
      stats_.reorderedPackets += 1;
    }
    if (!hasHighestSequence_ || packet.sequence > highestSequence_) {
      highestSequence_ = packet.sequence;
      hasHighestSequence_ = true;
    }

    const auto inserted = buffer_.emplace(packet.sequence, std::move(packet));
    if (!inserted.second) {
      stats_.duplicatePackets += 1;
      return;
    }

    pruneLatencyLocked();
    condition_.notify_one();
  }

  AudioJitterStats takeStats() {
    std::lock_guard<std::mutex> lock(mutex_);
    AudioJitterStats snapshot = stats_;
    snapshot.bufferMs = bufferedDurationMsLocked();
    snapshot.queuedPackets = buffer_.size();
    stats_ = {};
    return snapshot;
  }

  double targetJitterMs() const {
    return targetJitterMs_;
  }

private:
  double packetDurationMs(const AudioPayloadPacket& packet) const {
    if (packet.sampleRate == 0) return 0.0;
    return (static_cast<double>(packet.frameCount) * 1000.0) / static_cast<double>(packet.sampleRate);
  }

  double bufferedDurationMsLocked() const {
    double durationMs = 0.0;
    for (const auto& entry : buffer_) {
      durationMs += packetDurationMs(entry.second);
    }
    return durationMs;
  }

  void pruneLatencyLocked() {
    while (buffer_.size() > 1 && bufferedDurationMsLocked() > maxDelayMs_) {
      const auto droppedSequence = buffer_.begin()->first;
      buffer_.erase(buffer_.begin());
      stats_.latencyDrops += 1;
      if (!hasExpectedSequence_ || expectedSequence_ <= droppedSequence) {
        expectedSequence_ = droppedSequence + 1;
        hasExpectedSequence_ = true;
      }
      started_ = true;
      missingStartedAt_ = {};
    }
  }

  void resetSequenceTrackingLocked() {
    started_ = false;
    expectedSequence_ = 0;
    highestSequence_ = 0;
    hasExpectedSequence_ = false;
    hasHighestSequence_ = false;
    missingStartedAt_ = {};
  }

  void switchSequencingMediaEpochLocked(std::uint64_t mediaEpoch) {
    if (!hasSequencingMediaEpoch_) {
      sequencingMediaEpoch_ = mediaEpoch;
      hasSequencingMediaEpoch_ = true;
      return;
    }
    if (sequencingMediaEpoch_ == mediaEpoch) return;

    const std::size_t dropped = buffer_.size();
    buffer_.clear();
    resetSequenceTrackingLocked();
    stats_.resets += 1;
    stats_.epochSequenceResets += 1;
    std::cout << "SNA1_JITTER_RESET reason=media-epoch"
              << " mediaEpoch=" << sequencingMediaEpoch_
              << " nextMediaEpoch=" << mediaEpoch
              << " droppedPackets=" << dropped
              << "\n";
    sequencingMediaEpoch_ = mediaEpoch;
  }

  void dropStaleEpochPacketsLocked() {
    if (!hasActiveMediaEpoch_) return;
    std::size_t dropped = 0;
    for (auto it = buffer_.begin(); it != buffer_.end();) {
      if (it->second.mediaEpoch < activeMediaEpoch_) {
        it = buffer_.erase(it);
        stats_.epochStaleDrops += 1;
        dropped += 1;
      } else {
        ++it;
      }
    }
    if (dropped > 0 && buffer_.empty()) {
      resetSequenceTrackingLocked();
    }
  }

  bool shouldStartLocked() const {
    if (buffer_.empty()) return false;
    return targetJitterMs_ <= 0.0 || bufferedDurationMsLocked() >= targetJitterMs_;
  }

  void recordAvLeadLocked(double leadMs) {
    if (stats_.avLeadSamples == 0) {
      stats_.avLeadMinMs = leadMs;
      stats_.avLeadMaxMs = leadMs;
    } else {
      stats_.avLeadMinMs = std::min(stats_.avLeadMinMs, leadMs);
      stats_.avLeadMaxMs = std::max(stats_.avLeadMaxMs, leadMs);
    }
    stats_.avLeadSumMs += leadMs;
    stats_.avLeadSamples += 1;
  }

  void recordDriftCorrectionLocked(const AudioDriftCorrectionResult& result) {
    if (!result.adjusted) return;
    if (stats_.driftSamples == 0) {
      stats_.driftRatioMin = result.ratio;
      stats_.driftRatioMax = result.ratio;
    } else {
      stats_.driftRatioMin = std::min(stats_.driftRatioMin, result.ratio);
      stats_.driftRatioMax = std::max(stats_.driftRatioMax, result.ratio);
    }
    stats_.driftRatioSum += result.ratio;
    stats_.driftFrameDelta += result.frameDelta;
    stats_.driftSamples += 1;
  }

  static float readFloatSample(const std::uint8_t* data, std::size_t sampleIndex) {
    float value = 0.0f;
    std::memcpy(&value, data + sampleIndex * sizeof(float), sizeof(float));
    return value;
  }

  static void writeFloatSample(std::uint8_t* data, std::size_t sampleIndex, float value) {
    std::memcpy(data + sampleIndex * sizeof(float), &value, sizeof(float));
  }

  AudioDriftCorrectionResult correctAudioDrift(const AudioPayloadPacket& packet,
                                               std::vector<std::uint8_t>& payload,
                                               std::uint32_t& frameCount) const {
    AudioDriftCorrectionResult result;
    if (!packet.hasAvLead ||
        packet.channels == 0 ||
        packet.sampleRate == 0 ||
        frameCount < 8 ||
        std::abs(packet.avLeadMs) <= driftDeadbandMs_) {
      return result;
    }

    const std::size_t channels = packet.channels;
    const std::size_t bytesPerFrame = channels * sizeof(float);
    const std::size_t inputBytes = static_cast<std::size_t>(frameCount) * bytesPerFrame;
    if (bytesPerFrame == 0 || payload.size() < inputBytes) {
      return result;
    }

    const double effectiveLeadMs = std::abs(packet.avLeadMs) - driftDeadbandMs_;
    const double correctionRangeMs = std::max(1.0, driftFullScaleMs_ - driftDeadbandMs_);
    const double correctionWeight = std::clamp(effectiveLeadMs / correctionRangeMs, 0.0, 1.0);
    const double adjust = correctionWeight * maxDriftAdjustRatio_;
    const double ratio = packet.avLeadMs > 0.0 ? 1.0 + adjust : 1.0 - adjust;
    const double roundedFrames = static_cast<double>(
      std::llround(static_cast<double>(frameCount) * ratio));
    const auto requestedFrames = static_cast<std::uint32_t>(std::max(1.0, roundedFrames));
    if (requestedFrames == frameCount) {
      return result;
    }

    std::vector<std::uint8_t> corrected(static_cast<std::size_t>(requestedFrames) * bytesPerFrame);
    const double inputLastFrame = static_cast<double>(frameCount - 1);
    const double outputLastFrame = static_cast<double>(requestedFrames > 1 ? requestedFrames - 1 : 1);

    for (std::uint32_t outFrame = 0; outFrame < requestedFrames; ++outFrame) {
      const double sourcePosition = requestedFrames > 1
        ? (static_cast<double>(outFrame) * inputLastFrame / outputLastFrame)
        : 0.0;
      const auto leftFrame = static_cast<std::uint32_t>(sourcePosition);
      const std::uint32_t rightFrame = std::min<std::uint32_t>(leftFrame + 1, frameCount - 1);
      const float mix = static_cast<float>(sourcePosition - static_cast<double>(leftFrame));
      for (std::size_t channel = 0; channel < channels; ++channel) {
        const std::size_t leftIndex = (static_cast<std::size_t>(leftFrame) * channels) + channel;
        const std::size_t rightIndex = (static_cast<std::size_t>(rightFrame) * channels) + channel;
        const float left = readFloatSample(payload.data(), leftIndex);
        const float right = readFloatSample(payload.data(), rightIndex);
        writeFloatSample(corrected.data(),
                         (static_cast<std::size_t>(outFrame) * channels) + channel,
                         left + ((right - left) * mix));
      }
    }

    result.adjusted = true;
    result.ratio = static_cast<double>(requestedFrames) / static_cast<double>(frameCount);
    result.frameDelta = static_cast<int>(requestedFrames) - static_cast<int>(frameCount);
    payload = std::move(corrected);
    frameCount = requestedFrames;
    return result;
  }

  bool shouldHoldOrDropForAvSyncLocked(std::map<std::uint64_t, AudioPayloadPacket>::iterator it) {
    if (!avClock_ || it->second.hostUnixMicros == 0) {
      stats_.avClockMisses += 1;
      return false;
    }

    double leadMs = 0.0;
    if (!avClock_->audioLeadMs(it->second.hostUnixMicros, leadMs)) {
      stats_.avClockMisses += 1;
      return false;
    }
    recordAvLeadLocked(leadMs);
    it->second.avLeadMs = leadMs;
    it->second.hasAvLead = true;

    if (leadMs > audioLeadHoldMs_ && bufferedDurationMsLocked() < maxDelayMs_) {
      stats_.avHoldDecisions += 1;
      return true;
    }

    if (leadMs < -audioLagDropMs_ && buffer_.size() > 1) {
      const std::uint64_t droppedSequence = it->first;
      buffer_.erase(it);
      expectedSequence_ = droppedSequence + 1;
      hasExpectedSequence_ = true;
      missingStartedAt_ = {};
      stats_.avSyncDrops += 1;
      stats_.latencyDrops += 1;
      return true;
    }

    return false;
  }

  bool popNextLocked(AudioPayloadPacket& packet) {
    const auto now = std::chrono::steady_clock::now();
    pruneLatencyLocked();
    dropStaleEpochPacketsLocked();
    if (buffer_.empty()) {
      if (started_) {
        stats_.underflows += 1;
      }
      started_ = false;
      missingStartedAt_ = {};
      return false;
    }

    if (!started_) {
      if (!shouldStartLocked()) return false;
      expectedSequence_ = buffer_.begin()->first;
      hasExpectedSequence_ = true;
      started_ = true;
      missingStartedAt_ = {};
      stats_.resets += 1;
      std::cout << "SNA1_JITTER_READY targetMs=" << std::fixed << std::setprecision(1)
                << targetJitterMs_
                << " bufferedMs=" << bufferedDurationMsLocked()
                << " packets=" << buffer_.size()
                << "\n";
    }

    while (!buffer_.empty() && buffer_.begin()->first < expectedSequence_) {
      buffer_.erase(buffer_.begin());
      stats_.staleDrops += 1;
    }
    if (buffer_.empty()) return false;

    auto it = buffer_.find(expectedSequence_);
    if (it == buffer_.end()) {
      const std::uint64_t firstSequence = buffer_.begin()->first;
      if (firstSequence > expectedSequence_) {
        if (missingStartedAt_.time_since_epoch().count() == 0) {
          missingStartedAt_ = now;
          return false;
        }
        const double waitedMs = std::chrono::duration<double, std::milli>(now - missingStartedAt_).count();
        if (waitedMs < missingWaitMs_ && bufferedDurationMsLocked() < maxDelayMs_) {
          return false;
        }
        stats_.lostPackets += firstSequence - expectedSequence_;
        expectedSequence_ = firstSequence;
        missingStartedAt_ = {};
        it = buffer_.begin();
      } else {
        return false;
      }
    } else {
      missingStartedAt_ = {};
    }

    if (shouldHoldOrDropForAvSyncLocked(it)) return false;

    packet = std::move(it->second);
    buffer_.erase(it);
    expectedSequence_ = packet.sequence + 1;
    hasExpectedSequence_ = true;
    return true;
  }

  void pumpLoop() {
    while (true) {
      AudioPayloadPacket packet;
      {
        std::unique_lock<std::mutex> lock(mutex_);
        condition_.wait_for(lock, std::chrono::milliseconds(3), [&] {
          return stopped_ || !buffer_.empty();
        });
        if (stopped_) return;
        if (!popNextLocked(packet)) continue;
      }

      bool submitted = false;
      AudioDriftCorrectionResult driftCorrection;
      std::vector<std::uint8_t> playbackPayload = std::move(packet.payload);
      std::uint32_t playbackFrameCount = packet.frameCount;
      driftCorrection = correctAudioDrift(packet, playbackPayload, playbackFrameCount);
      try {
        submitted = player_ && player_->submit(packet.sampleRate,
                                               packet.channels,
                                               playbackPayload.data(),
                                               static_cast<std::uint32_t>(playbackPayload.size()),
                                               playbackFrameCount);
      } catch (const std::exception& error) {
        std::cerr << "SNA1_JITTER submit error: " << error.what() << "\n";
      }

      std::lock_guard<std::mutex> lock(mutex_);
      if (submitted) {
        stats_.submittedPackets += 1;
        stats_.submittedFrames += playbackFrameCount;
        if (packet.rekeyGrace) {
          stats_.epochGraceSubmitted += 1;
        }
        recordDriftCorrectionLocked(driftCorrection);
        if (packet.hostUnixMicros > 0) {
          const double ageMs = static_cast<double>(unixMicros() - packet.hostUnixMicros) / 1000.0;
          stats_.ageSumMs += ageMs;
          stats_.ageMaxMs = std::max(stats_.ageMaxMs, ageMs);
          stats_.ageSamples += 1;
        }
      } else {
        stats_.queueDrops += 1;
      }
    }
  }

  std::shared_ptr<AudioQueuePcmPlayer> player_;
  std::shared_ptr<AvSyncClock> avClock_;
  const double targetJitterMs_;
  const double maxDelayMs_;
  static constexpr double missingWaitMs_ = 6.0;
  static constexpr double audioLeadHoldMs_ = 90.0;
  static constexpr double audioLagDropMs_ = 180.0;
  static constexpr double driftDeadbandMs_ = 12.0;
  static constexpr double driftFullScaleMs_ = 120.0;
  static constexpr double maxDriftAdjustRatio_ = 0.0125;
  std::mutex mutex_;
  std::condition_variable condition_;
  std::map<std::uint64_t, AudioPayloadPacket> buffer_;
  AudioJitterStats stats_;
  std::thread worker_;
  std::uint64_t expectedSequence_ = 0;
  std::uint64_t highestSequence_ = 0;
  std::uint64_t mediaGeneration_ = 0;
  std::uint64_t activeMediaEpoch_ = 0;
  std::uint64_t sequencingMediaEpoch_ = 0;
  std::uint32_t sampleRate_ = 0;
  std::uint16_t channels_ = 0;
  std::chrono::steady_clock::time_point missingStartedAt_{};
  bool hasExpectedSequence_ = false;
  bool hasHighestSequence_ = false;
  bool hasMediaGeneration_ = false;
  bool hasActiveMediaEpoch_ = false;
  bool hasSequencingMediaEpoch_ = false;
  bool started_ = false;
  bool stopped_ = false;
};

class ScopedFd {
public:
  explicit ScopedFd(int fd = -1) : fd_(fd) {}
  ~ScopedFd() {
    if (fd_ >= 0) close(fd_);
  }

  ScopedFd(const ScopedFd&) = delete;
  ScopedFd& operator=(const ScopedFd&) = delete;
  ScopedFd(ScopedFd&& other) noexcept : fd_(other.release()) {}
  ScopedFd& operator=(ScopedFd&& other) noexcept {
    if (this != &other) {
      if (fd_ >= 0) close(fd_);
      fd_ = other.release();
    }
    return *this;
  }

  int get() const { return fd_; }
  int release() {
    const int fd = fd_;
    fd_ = -1;
    return fd;
  }

private:
  int fd_ = -1;
};

ScopedFd createTcpListener(std::uint16_t port, const char* label) {
  ScopedFd server(socket(AF_INET, SOCK_STREAM, 0));
  if (server.get() < 0) {
    throw std::runtime_error(std::string("Could not create ") + label + " TCP listener socket.");
  }

  int reuse = 1;
  setsockopt(server.get(), SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));

  sockaddr_in address{};
  address.sin_family = AF_INET;
  address.sin_addr.s_addr = htonl(INADDR_ANY);
  address.sin_port = htons(port);
  if (bind(server.get(), reinterpret_cast<sockaddr*>(&address), sizeof(address)) != 0) {
    throw std::runtime_error(std::string("Could not bind ") + label + " TCP listener on port " + std::to_string(port));
  }
  if (listen(server.get(), 2) != 0) {
    throw std::runtime_error(std::string("Could not listen on ") + label + " TCP port " + std::to_string(port));
  }
  return server;
}

ScopedFd createUdpListener(std::uint16_t port, const char* label) {
  ScopedFd server(socket(AF_INET, SOCK_DGRAM, 0));
  if (server.get() < 0) {
    throw std::runtime_error(std::string("Could not create ") + label + " UDP listener socket.");
  }

  int reuse = 1;
  setsockopt(server.get(), SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));

  int receiveBufferBytes = 4 * 1024 * 1024;
  setsockopt(server.get(), SOL_SOCKET, SO_RCVBUF, &receiveBufferBytes, sizeof(receiveBufferBytes));

  sockaddr_in address{};
  address.sin_family = AF_INET;
  address.sin_addr.s_addr = htonl(INADDR_ANY);
  address.sin_port = htons(port);
  if (bind(server.get(), reinterpret_cast<sockaddr*>(&address), sizeof(address)) != 0) {
    throw std::runtime_error(std::string("Could not bind ") + label + " UDP listener on port " + std::to_string(port));
  }
  return server;
}

void configureAcceptedTcpSocket(int fd) {
  if (fd < 0) return;

  int noDelay = 1;
  setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &noDelay, sizeof(noDelay));
#ifdef SO_NOSIGPIPE
  int noSigPipe = 1;
  setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, sizeof(noSigPipe));
#endif
}

void serviceControlSocket(int fd) {
  constexpr auto pingInterval = std::chrono::seconds(1);
  constexpr auto helloAckTimeout = std::chrono::seconds(2);
  constexpr auto pongTimeout = std::chrono::seconds(5);
  constexpr auto watchdogLogInterval = std::chrono::seconds(2);

  std::uint64_t pingSequence = 0;
  auto lastPingAt = std::chrono::steady_clock::now() - pingInterval;
  auto lastHelloAt = std::chrono::steady_clock::now();
  auto lastPongAt = lastHelloAt;
  auto lastWatchdogLogAt = std::chrono::steady_clock::time_point{};
  bool helloAcked = false;
  bool controlDegraded = false;
  sendControlHello();
  while (true) {
    const auto now = std::chrono::steady_clock::now();
    if (now - lastPingAt >= pingInterval) {
      lastPingAt = now;
      sendControlPing(++pingSequence);
    }
    if (!helloAcked && now - lastHelloAt >= helloAckTimeout) {
      lastHelloAt = now;
      std::cout << "SNCONTROL_WATCHDOG state=hello-timeout action=resend-hello\n";
      sendControlHello();
    } else if (helloAcked && now - lastPongAt >= pongTimeout) {
      const double silenceMs = std::chrono::duration<double, std::milli>(now - lastPongAt).count();
      if (!controlDegraded || now - lastWatchdogLogAt >= watchdogLogInterval) {
        lastWatchdogLogAt = now;
        std::cout << "SNCONTROL_WATCHDOG state=pong-timeout silenceMs="
                  << std::fixed << std::setprecision(0) << silenceMs
                  << " action=rehandshake\n";
      }
      if (!controlDegraded && gNativeInputSender) {
        gNativeInputSender->setControlAuthenticated(false);
        gNativeInputSender->setPacketAuthEnabled(false);
        gNativeInputSender->setBatchEnabled(false);
      }
      controlDegraded = true;
      helloAcked = false;
      lastHelloAt = now;
      sendControlHello();
    }

    fd_set readSet;
    FD_ZERO(&readSet);
    FD_SET(fd, &readSet);
    timeval timeout{};
    timeout.tv_sec = 0;
    timeout.tv_usec = 200000;
    const int ready = select(fd + 1, &readSet, nullptr, nullptr, &timeout);
    if (ready == 0) continue;
    if (ready < 0) {
      if (errno == EINTR) continue;
      return;
    }

    std::string payload;
    if (!readControlPayloadFromFd(fd, payload)) return;
    const HostControlEvent event = handleHostControlPayload(payload);
    const auto receivedAt = std::chrono::steady_clock::now();
    if (event == HostControlEvent::HelloAck) {
      helloAcked = true;
      lastPongAt = receivedAt;
      if (controlDegraded) {
        std::cout << "SNCONTROL_WATCHDOG state=recovered action=resume-control\n";
      }
      controlDegraded = false;
    } else if (event == HostControlEvent::Pong) {
      lastPongAt = receivedAt;
      if (controlDegraded) {
        std::cout << "SNCONTROL_WATCHDOG state=recovered action=resume-control\n";
        controlDegraded = false;
      }
    }
  }
}

int listenSnvTcp(std::uint16_t port, std::uint64_t maxPackets) {
  @autoreleasepool {
    ScopedFd server = createTcpListener(port, "SNV1");

    std::cout << "SNV1 TCP listener ready on 0.0.0.0:" << port << "\n";
    std::cout << "Start Windows host with: --encode-pipe h264 --tcp-connect 100.100.83.44:" << port << "\n";

    sockaddr_in peerAddress{};
    socklen_t peerLength = sizeof(peerAddress);
    ScopedFd client(accept(server.get(), reinterpret_cast<sockaddr*>(&peerAddress), &peerLength));
    if (client.get() < 0) {
      throw std::runtime_error("TCP accept failed.");
    }
    configureAcceptedTcpSocket(client.get());

    char peerIp[INET_ADDRSTRLEN]{};
    inet_ntop(AF_INET, &peerAddress.sin_addr, peerIp, sizeof(peerIp));
    std::cout << "SNV1 TCP client connected from " << peerIp << ":" << ntohs(peerAddress.sin_port) << "\n";

    VtH264Decoder decoder;
    DecodeSummary summary;
    while (maxPackets == 0 || summary.packets < maxPackets) {
      SnvPacket packet;
      if (!readSnvPacketFromFd(client.get(), packet)) {
        break;
      }
      processSnvPacket(decoder, summary, packet);
      if (summary.packets % 60 == 0) {
        std::cout << "SNV1 TCP packets=" << summary.packets
                  << " decoded=" << decoder.decodedFrames()
                  << " errors=" << decoder.decodeErrors()
                  << "\n";
      }
    }

    decoder.flush();
    printDecodeSummary("tcp-listen:" + std::to_string(port), summary, decoder);
    if (decoder.decodedFrames() == 0) {
      std::cerr << "No frames decoded from TCP stream yet.\n";
      return 2;
    }
  }
  return 0;
}

} // namespace

@interface SanserMetalView : MTKView {
  NSTrackingArea* _trackingArea;
  NSTimer* _mousePollTimer;
  NSPoint _lastPolledPoint;
  BOOL _hasPolledPoint;
}
@end

@implementation SanserMetalView

- (BOOL)acceptsFirstResponder {
  return YES;
}

- (BOOL)canBecomeKeyView {
  return YES;
}

- (BOOL)acceptsFirstMouse:(NSEvent*)event {
  (void)event;
  return YES;
}

- (NSView*)hitTest:(NSPoint)point {
  return NSPointInRect(point, [self bounds]) ? self : nil;
}

- (void)viewDidMoveToWindow {
  [super viewDidMoveToWindow];
  if (![self window]) {
    [_mousePollTimer invalidate];
    _mousePollTimer = nil;
    _hasPolledPoint = NO;
    return;
  }

  [[self window] makeFirstResponder:self];
  [[self window] setAcceptsMouseMovedEvents:YES];
  if (!_mousePollTimer) {
    __weak SanserMetalView* weakSelf = self;
    _mousePollTimer = [NSTimer timerWithTimeInterval:(1.0 / 120.0)
                                             repeats:YES
                                               block:^(NSTimer*) {
                                                 [weakSelf pollMouseLocation];
                                               }];
    [[NSRunLoop mainRunLoop] addTimer:_mousePollTimer forMode:NSRunLoopCommonModes];
  }
}

- (void)dealloc {
  [_mousePollTimer invalidate];
}

- (void)updateTrackingAreas {
  [super updateTrackingAreas];
  if (_trackingArea) {
    [self removeTrackingArea:_trackingArea];
  }
  _trackingArea = [[NSTrackingArea alloc] initWithRect:[self bounds]
                                               options:NSTrackingMouseMoved
                                                     | NSTrackingMouseEnteredAndExited
                                                     | NSTrackingActiveAlways
                                                     | NSTrackingEnabledDuringMouseDrag
                                                     | NSTrackingInVisibleRect
                                                 owner:self
                                              userInfo:nil];
  [self addTrackingArea:_trackingArea];
}

- (void)mouseEntered:(NSEvent*)event {
  (void)event;
  [[self window] makeFirstResponder:self];
  [[self window] setAcceptsMouseMovedEvents:YES];
}

- (void)pollMouseLocation {
  NSWindow* window = [self window];
  if (!window || ![window isVisible]) return;

  const NSPoint windowPoint = [window mouseLocationOutsideOfEventStream];
  const NSPoint point = [self convertPoint:windowPoint fromView:nil];
  if (!NSPointInRect(point, [self bounds])) {
    _hasPolledPoint = NO;
    return;
  }

  if (_hasPolledPoint &&
      std::abs(point.x - _lastPolledPoint.x) < 0.5 &&
      std::abs(point.y - _lastPolledPoint.y) < 0.5) {
    return;
  }

  const double scale = backingScaleForView(self);
  const CGFloat dx = _hasPolledPoint ? static_cast<CGFloat>((point.x - _lastPolledPoint.x) * scale) : 0;
  const CGFloat dy = _hasPolledPoint ? static_cast<CGFloat>((_lastPolledPoint.y - point.y) * scale) : 0;
  _lastPolledPoint = point;
  _hasPolledPoint = YES;
  [[self window] makeFirstResponder:self];
  sendNativeInputJson(pointerJsonFromPoint("pointer-move", point, self, 0, dx, dy, gRelativeMouse));
}

- (void)keyDown:(NSEvent*)event {
  if (sendCommandShortcut(event)) return;
  logKeyEvent("key-down", event);
}

- (void)keyUp:(NSEvent*)event {
  logKeyEvent("key-up", event);
}

- (void)flagsChanged:(NSEvent*)event {
  logKeyEvent("modifiers", event);
}

- (BOOL)performKeyEquivalent:(NSEvent*)event {
  if ([event type] == NSEventTypeKeyDown && sendCommandShortcut(event)) return YES;
  return [super performKeyEquivalent:event];
}

- (void)mouseMoved:(NSEvent*)event {
  [[self window] makeFirstResponder:self];
  logPointerEvent("pointer-move", event, self);
}

- (void)mouseDragged:(NSEvent*)event {
  logPointerEvent("pointer-move", event, self);
}

- (void)rightMouseDragged:(NSEvent*)event {
  logPointerEvent("pointer-move", event, self);
}

- (void)otherMouseDragged:(NSEvent*)event {
  logPointerEvent("pointer-move", event, self);
}

- (void)mouseDown:(NSEvent*)event {
  [[self window] makeFirstResponder:self];
  logPointerEvent("pointer-down", event, self);
}

- (void)mouseUp:(NSEvent*)event {
  logPointerEvent("pointer-up", event, self);
}

- (void)rightMouseDown:(NSEvent*)event {
  [[self window] makeFirstResponder:self];
  logPointerEvent("pointer-down", event, self);
}

- (void)rightMouseUp:(NSEvent*)event {
  logPointerEvent("pointer-up", event, self);
}

- (void)otherMouseDown:(NSEvent*)event {
  [[self window] makeFirstResponder:self];
  logPointerEvent("pointer-down", event, self);
}

- (void)otherMouseUp:(NSEvent*)event {
  logPointerEvent("pointer-up", event, self);
}

- (void)scrollWheel:(NSEvent*)event {
  if (std::abs([event scrollingDeltaX]) < 0.01 && std::abs([event scrollingDeltaY]) < 0.01) return;
  logPointerEvent("wheel", event, self);
}

@end

@interface SanserMetalRenderer : NSObject <MTKViewDelegate>
- (instancetype)initWithDevice:(id<MTLDevice>)device view:(MTKView*)view;
@end

@implementation SanserMetalRenderer {
  id<MTLCommandQueue> _commandQueue;
  id<MTLRenderPipelineState> _pipeline;
  std::chrono::steady_clock::time_point _startedAt;
}

- (instancetype)initWithDevice:(id<MTLDevice>)device view:(MTKView*)view {
  self = [super init];
  if (!self) return nil;

  _startedAt = std::chrono::steady_clock::now();
  _commandQueue = [device newCommandQueue];

  NSString* shaderSource =
    @"#include <metal_stdlib>\n"
     @"using namespace metal;\n"
     @"struct VertexOut { float4 position [[position]]; float2 uv; };\n"
     @"vertex VertexOut vertex_main(uint vid [[vertex_id]]) {\n"
     @"  float2 pos[6] = { {-1,-1}, {1,-1}, {-1,1}, {-1,1}, {1,-1}, {1,1} };\n"
     @"  float2 uv[6] = { {0,1}, {1,1}, {0,0}, {0,0}, {1,1}, {1,0} };\n"
     @"  VertexOut out;\n"
     @"  out.position = float4(pos[vid], 0, 1);\n"
     @"  out.uv = uv[vid];\n"
     @"  return out;\n"
     @"}\n"
     @"fragment float4 fragment_main(VertexOut in [[stage_in]], constant float& time [[buffer(0)]]) {\n"
     @"  float scan = 0.08 * sin((in.uv.y + time * 0.18) * 90.0);\n"
     @"  float3 color = mix(float3(0.02, 0.03, 0.07), float3(0.12, 0.72, 0.58), in.uv.x);\n"
     @"  color += float3(0.42, 0.25, 0.92) * (0.35 + 0.25 * sin(time + in.uv.y * 7.0));\n"
     @"  return float4(color + scan, 1.0);\n"
     @"}\n";

  NSError* error = nil;
  id<MTLLibrary> library = [device newLibraryWithSource:shaderSource options:nil error:&error];
  if (!library) {
    std::cerr << "Metal shader compile failed: " << nsStringToUtf8([error localizedDescription]) << "\n";
    return nil;
  }

  MTLRenderPipelineDescriptor* descriptor = [[MTLRenderPipelineDescriptor alloc] init];
  descriptor.vertexFunction = [library newFunctionWithName:@"vertex_main"];
  descriptor.fragmentFunction = [library newFunctionWithName:@"fragment_main"];
  descriptor.colorAttachments[0].pixelFormat = view.colorPixelFormat;

  _pipeline = [device newRenderPipelineStateWithDescriptor:descriptor error:&error];
  if (!_pipeline) {
    std::cerr << "Metal pipeline creation failed: " << nsStringToUtf8([error localizedDescription]) << "\n";
    return nil;
  }

  return self;
}

- (void)mtkView:(MTKView*)view drawableSizeWillChange:(CGSize)size {
  (void)view;
  (void)size;
}

- (void)drawInMTKView:(MTKView*)view {
  MTLRenderPassDescriptor* pass = [view currentRenderPassDescriptor];
  id<CAMetalDrawable> drawable = [view currentDrawable];
  if (!pass || !drawable) return;

  id<MTLCommandBuffer> commandBuffer = [_commandQueue commandBuffer];
  id<MTLRenderCommandEncoder> encoder = [commandBuffer renderCommandEncoderWithDescriptor:pass];

  float time = static_cast<float>(secondsSince(_startedAt));
  [encoder setRenderPipelineState:_pipeline];
  [encoder setFragmentBytes:&time length:sizeof(time) atIndex:0];
  [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:6];
  [encoder endEncoding];

  [commandBuffer presentDrawable:drawable];
  [commandBuffer commit];
}

@end

struct QueuedVideoFrame {
  CVPixelBufferRef pixelBuffer = nullptr;
  std::uint64_t queuedAtMicros = 0;
  std::uint64_t presentAtMicros = 0;
  std::uint64_t sequence = 0;
  std::uint64_t packetSequence = 0;
  std::uint64_t mediaTimestampMicros = 0;
  std::uint64_t durationMicros = 0;
  std::uint64_t hostUnixMicros = 0;
  std::uint64_t decodeSubmittedAtMicros = 0;
};

std::uint64_t normalizedVideoDurationMicros(std::uint64_t durationMicros) {
  if (durationMicros == 0) return 16667;
  return std::clamp<std::uint64_t>(durationMicros, 4000, 50000);
}

std::uint64_t videoPacingTargetDelayMicros(std::uint64_t durationMicros) {
  const std::uint64_t normalized = normalizedVideoDurationMicros(durationMicros);
  return std::clamp<std::uint64_t>(normalized / 2, 6000, 18000);
}

std::uint64_t videoPacingAdaptiveDelayMaxMicros(std::uint64_t durationMicros) {
  const std::uint64_t normalized = normalizedVideoDurationMicros(durationMicros);
  return std::clamp<std::uint64_t>(normalized * 2, 12000, 24000);
}

std::uint64_t videoPacingMaxLateMicros(std::uint64_t durationMicros) {
  const std::uint64_t normalized = normalizedVideoDurationMicros(durationMicros);
  return std::clamp<std::uint64_t>(normalized * 3, 50000, 120000);
}

@interface SanserVideoRenderer : NSObject <MTKViewDelegate>
- (instancetype)initWithDevice:(id<MTLDevice>)device view:(MTKView*)view;
- (void)submitPixelBuffer:(CVPixelBufferRef)pixelBuffer metadata:(const DecodedVideoFrameMetadata*)metadata;
@end

@implementation SanserVideoRenderer {
  id<MTLDevice> _device;
  id<MTLCommandQueue> _commandQueue;
  id<MTLRenderPipelineState> _pipeline;
  CVMetalTextureCacheRef _textureCache;
  CVPixelBufferRef _currentPixelBuffer;
  std::uint64_t _currentQueuedAtMicros;
  std::deque<QueuedVideoFrame> _frameQueue;
  NSLock* _lock;
  std::uint64_t _submittedBuffers;
  std::uint64_t _renderedBuffers;
  std::uint64_t _drawCalls;
  std::uint64_t _droppedQueueFrames;
  std::uint64_t _droppedLateFrames;
  std::uint64_t _heldForPacing;
  std::uint64_t _pacingResets;
  std::uint64_t _lastTargetDelayMicros;
  std::uint64_t _adaptiveDelayMicros;
  std::uint64_t _adaptiveIncreases;
  std::uint64_t _adaptiveDecreases;
  std::uint32_t _adaptiveClearWindows;
  std::uint64_t _pacingBaseMediaMicros;
  std::uint64_t _pacingBaseSteadyMicros;
  std::uint64_t _lastSubmittedMediaMicros;
  std::uint64_t _lastSubmittedPacketSequence;
  double _renderAgeSumMs;
  double _renderAgeMaxMs;
  std::uint64_t _renderAgeSamples;
  double _presentLateSumMs;
  double _presentLateMaxMs;
  std::uint64_t _presentLateSamples;
  bool _pacingTimelineValid;
  std::chrono::steady_clock::time_point _statsStartedAt;
}

- (instancetype)initWithDevice:(id<MTLDevice>)device view:(MTKView*)view {
  self = [super init];
  if (!self) return nil;

  _device = device;
  _commandQueue = [device newCommandQueue];
  _lock = [[NSLock alloc] init];
  _statsStartedAt = std::chrono::steady_clock::now();
  _lastTargetDelayMicros = videoPacingTargetDelayMicros(16667);

  CVReturn cacheResult = CVMetalTextureCacheCreate(kCFAllocatorDefault, nullptr, device, nullptr, &_textureCache);
  if (cacheResult != kCVReturnSuccess || !_textureCache) {
    std::cerr << "CVMetalTextureCacheCreate failed: " << cacheResult << "\n";
    return nil;
  }

  NSString* shaderSource =
    @"#include <metal_stdlib>\n"
     @"using namespace metal;\n"
     @"struct VertexOut { float4 position [[position]]; float2 uv; };\n"
     @"vertex VertexOut vertex_main(uint vid [[vertex_id]]) {\n"
     @"  float2 pos[6] = { {-1,-1}, {1,-1}, {-1,1}, {-1,1}, {1,-1}, {1,1} };\n"
     @"  float2 uv[6] = { {0,1}, {1,1}, {0,0}, {0,0}, {1,1}, {1,0} };\n"
     @"  VertexOut out;\n"
     @"  out.position = float4(pos[vid], 0, 1);\n"
     @"  out.uv = uv[vid];\n"
     @"  return out;\n"
     @"}\n"
     @"fragment float4 fragment_video(VertexOut in [[stage_in]], texture2d<float> yTex [[texture(0)]], texture2d<float> uvTex [[texture(1)]]) {\n"
     @"  constexpr sampler s(address::clamp_to_edge, filter::linear);\n"
     @"  float y = yTex.sample(s, in.uv).r;\n"
     @"  float2 uv = uvTex.sample(s, in.uv).rg - float2(0.5, 0.5);\n"
     @"  y = max((y - 0.0625) * 1.164383, 0.0);\n"
     @"  float3 rgb;\n"
     @"  rgb.r = y + 1.792741 * uv.y;\n"
     @"  rgb.g = y - 0.213249 * uv.x - 0.532909 * uv.y;\n"
     @"  rgb.b = y + 2.112402 * uv.x;\n"
     @"  return float4(saturate(rgb), 1.0);\n"
     @"}\n";

  NSError* error = nil;
  id<MTLLibrary> library = [device newLibraryWithSource:shaderSource options:nil error:&error];
  if (!library) {
    std::cerr << "Video Metal shader compile failed: " << nsStringToUtf8([error localizedDescription]) << "\n";
    return nil;
  }

  MTLRenderPipelineDescriptor* descriptor = [[MTLRenderPipelineDescriptor alloc] init];
  descriptor.vertexFunction = [library newFunctionWithName:@"vertex_main"];
  descriptor.fragmentFunction = [library newFunctionWithName:@"fragment_video"];
  descriptor.colorAttachments[0].pixelFormat = view.colorPixelFormat;

  _pipeline = [device newRenderPipelineStateWithDescriptor:descriptor error:&error];
  if (!_pipeline) {
    std::cerr << "Video Metal pipeline creation failed: " << nsStringToUtf8([error localizedDescription]) << "\n";
    return nil;
  }

  std::cout << "SNV1_PACING_CONFIG mode=media-timeline minDelayMs=6.0 maxDelayMs=18.0 adaptiveMaxMs=24.0 maxQueue=5 maxLateFrames=3\n";
  return self;
}

- (void)dealloc {
  [_lock lock];
  if (_currentPixelBuffer) {
    CVPixelBufferRelease(_currentPixelBuffer);
    _currentPixelBuffer = nullptr;
  }
  for (const auto& frame : _frameQueue) {
    if (frame.pixelBuffer) CVPixelBufferRelease(frame.pixelBuffer);
  }
  _frameQueue.clear();
  [_lock unlock];
  if (_textureCache) {
    CFRelease(_textureCache);
    _textureCache = nullptr;
  }
}

- (void)submitPixelBuffer:(CVPixelBufferRef)pixelBuffer metadata:(const DecodedVideoFrameMetadata*)metadata {
  if (!pixelBuffer) return;
  CVPixelBufferRetain(pixelBuffer);
  QueuedVideoFrame frame;
  frame.pixelBuffer = pixelBuffer;
  frame.queuedAtMicros = steadyMicros();
  if (metadata) {
    frame.packetSequence = metadata->sequence;
    frame.mediaTimestampMicros = metadata->timestampMicros;
    frame.durationMicros = metadata->durationMicros;
    frame.hostUnixMicros = metadata->hostUnixMicros;
    frame.decodeSubmittedAtMicros = metadata->decodeSubmittedAtMicros;
  }
  frame.durationMicros = normalizedVideoDurationMicros(frame.durationMicros);
  [_lock lock];
  frame.sequence = ++_submittedBuffers;
  const std::uint64_t baseTargetDelayMicros = videoPacingTargetDelayMicros(frame.durationMicros);
  const std::uint64_t targetDelayMicros = baseTargetDelayMicros +
    std::min<std::uint64_t>(_adaptiveDelayMicros, videoPacingAdaptiveDelayMaxMicros(frame.durationMicros));
  const std::uint64_t maxLateMicros = videoPacingMaxLateMicros(frame.durationMicros);
  bool shouldResetTimeline = !_pacingTimelineValid || frame.mediaTimestampMicros == 0;
  if (_lastSubmittedMediaMicros > 0 &&
      frame.mediaTimestampMicros > 0 &&
      frame.mediaTimestampMicros + maxLateMicros < _lastSubmittedMediaMicros) {
    shouldResetTimeline = true;
  }
  if (_lastSubmittedPacketSequence > 0 &&
      frame.packetSequence > 0 &&
      frame.packetSequence + 1 < _lastSubmittedPacketSequence) {
    shouldResetTimeline = true;
  }

  if (shouldResetTimeline) {
    _pacingTimelineValid = frame.mediaTimestampMicros > 0;
    _pacingBaseMediaMicros = frame.mediaTimestampMicros;
    _pacingBaseSteadyMicros = frame.queuedAtMicros + targetDelayMicros;
    _pacingResets += 1;
  }

  if (_pacingTimelineValid && frame.mediaTimestampMicros >= _pacingBaseMediaMicros) {
    frame.presentAtMicros = _pacingBaseSteadyMicros + (frame.mediaTimestampMicros - _pacingBaseMediaMicros);
    if (frame.queuedAtMicros > frame.presentAtMicros + maxLateMicros) {
      _pacingBaseMediaMicros = frame.mediaTimestampMicros;
      _pacingBaseSteadyMicros = frame.queuedAtMicros + targetDelayMicros;
      frame.presentAtMicros = _pacingBaseSteadyMicros;
      _pacingResets += 1;
    }
  } else {
    frame.presentAtMicros = frame.queuedAtMicros + targetDelayMicros;
  }

  _lastTargetDelayMicros = targetDelayMicros;
  if (frame.mediaTimestampMicros > 0) _lastSubmittedMediaMicros = frame.mediaTimestampMicros;
  if (frame.packetSequence > 0) _lastSubmittedPacketSequence = frame.packetSequence;
  _frameQueue.push_back(frame);
  constexpr std::size_t maxQueueDepth = 5;
  while (_frameQueue.size() > maxQueueDepth) {
    if (_frameQueue.front().pixelBuffer) CVPixelBufferRelease(_frameQueue.front().pixelBuffer);
    _frameQueue.pop_front();
    _droppedQueueFrames += 1;
  }
  [_lock unlock];
}

- (void)mtkView:(MTKView*)view drawableSizeWillChange:(CGSize)size {
  (void)view;
  (void)size;
}

- (void)drawInMTKView:(MTKView*)view {
  MTLRenderPassDescriptor* pass = [view currentRenderPassDescriptor];
  id<CAMetalDrawable> drawable = [view currentDrawable];
  if (!pass || !drawable) return;

  CVPixelBufferRef pixelBuffer = nullptr;
  [_lock lock];
  _drawCalls += 1;
  const std::uint64_t nowMicrosValue = steadyMicros();
  while (_frameQueue.size() > 1 &&
         nowMicrosValue > _frameQueue.front().presentAtMicros &&
         nowMicrosValue - _frameQueue.front().presentAtMicros > videoPacingMaxLateMicros(_frameQueue.front().durationMicros)) {
    if (_frameQueue.front().pixelBuffer) CVPixelBufferRelease(_frameQueue.front().pixelBuffer);
    _frameQueue.pop_front();
    _droppedLateFrames += 1;
  }

  if (!_frameQueue.empty()) {
    const bool ready = nowMicrosValue >= _frameQueue.front().presentAtMicros
                    || _frameQueue.size() >= 4;
    if (ready) {
      QueuedVideoFrame frame = _frameQueue.front();
      _frameQueue.pop_front();
      if (_currentPixelBuffer) CVPixelBufferRelease(_currentPixelBuffer);
      _currentPixelBuffer = frame.pixelBuffer;
      _currentQueuedAtMicros = frame.queuedAtMicros;
      _renderedBuffers += 1;
      if (nowMicrosValue >= frame.presentAtMicros) {
        const double presentLateMs = static_cast<double>(nowMicrosValue - frame.presentAtMicros) / 1000.0;
        _presentLateSumMs += presentLateMs;
        _presentLateMaxMs = std::max(_presentLateMaxMs, presentLateMs);
        _presentLateSamples += 1;
      }
      if (nowMicrosValue > _currentQueuedAtMicros) {
        const double frameAgeMs = static_cast<double>(nowMicrosValue - _currentQueuedAtMicros) / 1000.0;
        _renderAgeSumMs += frameAgeMs;
        _renderAgeMaxMs = std::max(_renderAgeMaxMs, frameAgeMs);
        _renderAgeSamples += 1;
      }
    } else {
      _heldForPacing += 1;
    }
  }

  if (_currentPixelBuffer) {
    pixelBuffer = CVPixelBufferRetain(_currentPixelBuffer);
  }
  [_lock unlock];

  id<MTLCommandBuffer> commandBuffer = [_commandQueue commandBuffer];
  id<MTLRenderCommandEncoder> encoder = [commandBuffer renderCommandEncoderWithDescriptor:pass];

  if (pixelBuffer && CVPixelBufferGetPlaneCount(pixelBuffer) >= 2) {
    const std::size_t width = CVPixelBufferGetWidthOfPlane(pixelBuffer, 0);
    const std::size_t height = CVPixelBufferGetHeightOfPlane(pixelBuffer, 0);
    const std::size_t uvWidth = CVPixelBufferGetWidthOfPlane(pixelBuffer, 1);
    const std::size_t uvHeight = CVPixelBufferGetHeightOfPlane(pixelBuffer, 1);

    CVMetalTextureRef yTextureRef = nullptr;
    CVMetalTextureRef uvTextureRef = nullptr;
    CVReturn yResult = CVMetalTextureCacheCreateTextureFromImage(kCFAllocatorDefault,
                                                                 _textureCache,
                                                                 pixelBuffer,
                                                                 nullptr,
                                                                 MTLPixelFormatR8Unorm,
                                                                 width,
                                                                 height,
                                                                 0,
                                                                 &yTextureRef);
    CVReturn uvResult = CVMetalTextureCacheCreateTextureFromImage(kCFAllocatorDefault,
                                                                  _textureCache,
                                                                  pixelBuffer,
                                                                  nullptr,
                                                                  MTLPixelFormatRG8Unorm,
                                                                  uvWidth,
                                                                  uvHeight,
                                                                  1,
                                                                  &uvTextureRef);
    if (yResult == kCVReturnSuccess && uvResult == kCVReturnSuccess && yTextureRef && uvTextureRef) {
      id<MTLTexture> yTexture = CVMetalTextureGetTexture(yTextureRef);
      id<MTLTexture> uvTexture = CVMetalTextureGetTexture(uvTextureRef);
      [encoder setRenderPipelineState:_pipeline];
      [encoder setFragmentTexture:yTexture atIndex:0];
      [encoder setFragmentTexture:uvTexture atIndex:1];
      [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:6];
    }
    if (yTextureRef) CFRelease(yTextureRef);
    if (uvTextureRef) CFRelease(uvTextureRef);
  }

  [encoder endEncoding];
  [commandBuffer presentDrawable:drawable];
  [commandBuffer commit];

  if (pixelBuffer) {
    CVPixelBufferRelease(pixelBuffer);
  }

  const auto statsNow = std::chrono::steady_clock::now();
  const double statsSeconds = std::chrono::duration<double>(statsNow - _statsStartedAt).count();
  if (statsSeconds >= 1.0) {
    std::uint64_t draws = 0;
    std::uint64_t rendered = 0;
    std::size_t queueDepth = 0;
    std::uint64_t droppedQueue = 0;
    std::uint64_t droppedLate = 0;
    std::uint64_t held = 0;
    std::uint64_t pacingReset = 0;
    double targetDelayMs = 0.0;
    double avgRenderAgeMs = -1.0;
    double maxRenderAgeMs = 0.0;
    double avgPresentLateMs = -1.0;
    double maxPresentLateMs = 0.0;
    double adaptiveDelayMs = 0.0;
    std::uint64_t adaptiveUp = 0;
    std::uint64_t adaptiveDown = 0;

    [_lock lock];
    draws = _drawCalls;
    rendered = _renderedBuffers;
    queueDepth = _frameQueue.size();
    droppedQueue = _droppedQueueFrames;
    droppedLate = _droppedLateFrames;
    held = _heldForPacing;
    pacingReset = _pacingResets;
    targetDelayMs = static_cast<double>(_lastTargetDelayMicros) / 1000.0;
    avgRenderAgeMs = _renderAgeSamples > 0
      ? _renderAgeSumMs / static_cast<double>(_renderAgeSamples)
      : -1.0;
    maxRenderAgeMs = _renderAgeMaxMs;
    avgPresentLateMs = _presentLateSamples > 0
      ? _presentLateSumMs / static_cast<double>(_presentLateSamples)
      : -1.0;
    maxPresentLateMs = _presentLateMaxMs;
    const bool renderCongested = droppedQueue > 0 ||
      droppedLate > 0 ||
      queueDepth >= 4 ||
      pacingReset > 1 ||
      (avgPresentLateMs >= 0.0 && avgPresentLateMs > 9.0) ||
      maxPresentLateMs > 20.0 ||
      maxRenderAgeMs > 85.0;
    const bool renderClear = rendered > 0 &&
      droppedQueue == 0 &&
      droppedLate == 0 &&
      queueDepth <= 1 &&
      pacingReset == 0 &&
      (avgPresentLateMs < 0.0 || avgPresentLateMs < 4.0) &&
      maxPresentLateMs < 12.0 &&
      maxRenderAgeMs < 55.0;
    const std::uint64_t oldAdaptiveDelayMicros = _adaptiveDelayMicros;
    if (renderCongested) {
      _adaptiveClearWindows = 0;
      _adaptiveDelayMicros = std::min<std::uint64_t>(_adaptiveDelayMicros + 2000, 24000);
      if (_adaptiveDelayMicros != oldAdaptiveDelayMicros) {
        _adaptiveIncreases += 1;
      }
    } else if (renderClear) {
      _adaptiveClearWindows += 1;
      if (_adaptiveClearWindows >= 3 && _adaptiveDelayMicros > 0) {
        _adaptiveDelayMicros = _adaptiveDelayMicros > 1000 ? _adaptiveDelayMicros - 1000 : 0;
        _adaptiveClearWindows = 0;
        if (_adaptiveDelayMicros != oldAdaptiveDelayMicros) {
          _adaptiveDecreases += 1;
        }
      }
    } else {
      _adaptiveClearWindows = 0;
    }
    if (_adaptiveDelayMicros != oldAdaptiveDelayMicros) {
      _pacingTimelineValid = false;
      std::cout << "SNV1_RENDER_ADAPT adaptiveDelayMs="
                << std::fixed << std::setprecision(1)
                << (static_cast<double>(_adaptiveDelayMicros) / 1000.0)
                << " previousMs=" << (static_cast<double>(oldAdaptiveDelayMicros) / 1000.0)
                << " reason=" << (renderCongested ? "congested" : "clear")
                << " droppedQueue=" << droppedQueue
                << " droppedLate=" << droppedLate
                << " queueDepth=" << queueDepth
                << " avgPresentLateMs=" << avgPresentLateMs
                << " maxPresentLateMs=" << maxPresentLateMs
                << "\n";
    }
    adaptiveDelayMs = static_cast<double>(_adaptiveDelayMicros) / 1000.0;
    adaptiveUp = _adaptiveIncreases;
    adaptiveDown = _adaptiveDecreases;
    _drawCalls = 0;
    _renderedBuffers = 0;
    _droppedQueueFrames = 0;
    _droppedLateFrames = 0;
    _heldForPacing = 0;
    _pacingResets = 0;
    _adaptiveIncreases = 0;
    _adaptiveDecreases = 0;
    _renderAgeSumMs = 0.0;
    _renderAgeMaxMs = 0.0;
    _renderAgeSamples = 0;
    _presentLateSumMs = 0.0;
    _presentLateMaxMs = 0.0;
    _presentLateSamples = 0;
    _statsStartedAt = statsNow;
    [_lock unlock];

    std::cout << "SNV1_RENDER_STATS draws=" << draws
              << " rendered=" << rendered
              << " queueDepth=" << queueDepth
              << " droppedQueue=" << droppedQueue
              << " droppedLate=" << droppedLate
              << " held=" << held
              << " pacingReset=" << pacingReset
              << std::fixed << std::setprecision(1)
              << " targetDelayMs=" << targetDelayMs
              << " adaptiveDelayMs=" << adaptiveDelayMs
              << " adaptiveUp=" << adaptiveUp
              << " adaptiveDown=" << adaptiveDown
              << " avgRenderAgeMs=" << avgRenderAgeMs
              << " maxRenderAgeMs=" << maxRenderAgeMs
              << " avgPresentLateMs=" << avgPresentLateMs
              << " maxPresentLateMs=" << maxPresentLateMs
              << "\n";

    std::ostringstream feedback;
    feedback << std::fixed << std::setprecision(1)
             << "{\"type\":\"stream-stats\""
             << ",\"source\":\"render\""
             << ",\"draws\":" << draws
             << ",\"rendered\":" << rendered
             << ",\"queueDepth\":" << queueDepth
             << ",\"droppedQueue\":" << droppedQueue
             << ",\"droppedLate\":" << droppedLate
             << ",\"held\":" << held
             << ",\"pacingReset\":" << pacingReset
             << ",\"targetDelayMs\":" << targetDelayMs
             << ",\"adaptiveDelayMs\":" << adaptiveDelayMs
             << ",\"adaptiveUp\":" << adaptiveUp
             << ",\"adaptiveDown\":" << adaptiveDown
             << ",\"avgRenderAgeMs\":" << avgRenderAgeMs
             << ",\"maxRenderAgeMs\":" << maxRenderAgeMs
             << ",\"avgPresentLateMs\":" << avgPresentLateMs
             << ",\"maxPresentLateMs\":" << maxPresentLateMs
             << "}";
    const bool feedbackSent = sendNativeInputJson(feedback.str());
    std::cout << "SNV1_RENDER_FEEDBACK sent=" << boolText(feedbackSent)
              << " source=render"
              << " pressureFields=droppedQueue,droppedLate,presentLate,queueDepth,adaptiveDelay\n";
  }
}

@end

namespace {

void submitFrameToVideoRenderer(void* context,
                                CVImageBufferRef imageBuffer,
                                const DecodedVideoFrameMetadata& metadata) {
  auto* renderer = (__bridge SanserVideoRenderer*)context;
  [renderer submitPixelBuffer:(CVPixelBufferRef)imageBuffer metadata:&metadata];
}

void startDedicatedControlListener(std::uint16_t controlPort,
                                   std::shared_ptr<NativeInputSender> inputSender) {
  if (controlPort == 0) return;
  auto controlServer = std::make_shared<ScopedFd>(createTcpListener(controlPort, "SNINPUT control"));
  std::cout << "SNINPUT dedicated control listener ready on 0.0.0.0:" << controlPort << "\n";
  std::thread controlWorker([controlServer, inputSender, controlPort]() {
    while (true) {
      sockaddr_in peerAddress{};
      socklen_t peerLength = sizeof(peerAddress);
      ScopedFd control(accept(controlServer->get(), reinterpret_cast<sockaddr*>(&peerAddress), &peerLength));
      if (control.get() < 0) {
        std::cerr << "SNINPUT control accept failed on port " << controlPort << ".\n";
        return;
      }
      configureAcceptedTcpSocket(control.get());

      char peerIp[INET_ADDRSTRLEN]{};
      inet_ntop(AF_INET, &peerAddress.sin_addr, peerIp, sizeof(peerIp));
      std::cout << "SNINPUT dedicated control connected from "
                << peerIp << ":" << ntohs(peerAddress.sin_port) << "\n";
      if (inputSender) {
        inputSender->setFd(control.get());
      }
      serviceControlSocket(control.get());
      if (inputSender) {
        inputSender->clearFd(control.get());
      }
      std::cout << "SNINPUT dedicated control disconnected; waiting for reconnect.\n";
    }
  });
  controlWorker.detach();
}

void listenUdpAudio(std::uint16_t audioPort,
                    double audioJitterMs,
                    std::shared_ptr<AudioQueuePcmPlayer> player,
                    std::shared_ptr<AvSyncClock> avSyncClock) {
  if (audioPort == 0 || !player) return;
  @autoreleasepool {
    try {
      ScopedFd server = createUdpListener(audioPort, "SNA1 audio");
      std::cout << "SNA1 audio listener ready on 0.0.0.0:" << audioPort
                << " mediaCrypto=" << boolText(gMediaCryptoEnabled)
                << "\n";

      AudioJitterBuffer jitter(player, avSyncClock, audioJitterMs);
      std::cout << "SNA1_JITTER_CONFIG targetMs=" << std::fixed << std::setprecision(1)
                << jitter.targetJitterMs()
                << "\n";
      std::cout << "SNAV_SYNC audioLeadHoldMs=90.0 audioLagDropMs=180.0 clock=video-host-unix\n";
      std::cout << "SNAV_DRIFT mode=soft-resample deadbandMs=12.0 fullScaleMs=120.0 maxAdjustPct=1.25\n";

      std::array<std::uint8_t, 65536> datagramBuffer{};
      UdpAudioStats stats;
      UdpPeerLock audioPeerLock("SNA1");
      auto statsStartedAt = std::chrono::steady_clock::now();
      while (true) {
        sockaddr_in peerAddress{};
        socklen_t peerLength = sizeof(peerAddress);
        const ssize_t received = recvfrom(server.get(),
                                          datagramBuffer.data(),
                                          datagramBuffer.size(),
                                          0,
                                          reinterpret_cast<sockaddr*>(&peerAddress),
                                          &peerLength);
        if (received < 0) {
          if (errno == EINTR) continue;
          throw std::runtime_error("SNA1 UDP receive failed.");
        }
        if (received == 0) continue;

        const std::uint64_t mediaGeneration = gMediaPeerGeneration.load(std::memory_order_relaxed);
        audioPeerLock.observeGeneration(mediaGeneration);
        jitter.resetForMediaGeneration(mediaGeneration);
        if (audioPeerLock.isLocked() && !audioPeerLock.acceptKnownPeer(peerAddress)) {
          stats.peerRejected += 1;
          continue;
        }
        stats.datagrams += 1;
        const std::uint8_t* data = datagramBuffer.data();
        const std::size_t size = static_cast<std::size_t>(received);
        if (size < sizeof(AudioPacketHeader)) {
          stats.malformed += 1;
          continue;
        }
        std::vector<std::uint8_t> authenticatedDatagram;
        const bool authenticatedAudio = std::memcmp(data, "SNA2", 4) == 0;
        ResolvedMediaCrypto audioCrypto;
        std::uint64_t audioAuthSeq = 0;
        std::uint64_t audioAuthEpoch = 0;
        std::uint64_t packetMediaEpoch = 0;
        bool packetRekeyGrace = false;
        if (authenticatedAudio) {
          authenticatedDatagram.assign(data, data + size);
          audioAuthSeq = size >= sizeof(AudioAuthenticatedPacketHeader)
            ? readLe64Raw(data + offsetof(AudioAuthenticatedPacketHeader, authSeq))
            : 0;
          audioAuthEpoch = size >= sizeof(AudioAuthenticatedPacketHeader)
            ? readLe64Raw(data + offsetof(AudioAuthenticatedPacketHeader, authEpoch))
            : 0;
          audioCrypto = resolveMediaCryptoForEpoch(audioAuthEpoch, false);
          if (size < sizeof(AudioAuthenticatedPacketHeader) ||
              !audioCrypto.enabled ||
              !verifyUdpDatagramMac(audioCrypto.authKey,
                                    "audio",
                                    authenticatedDatagram,
                                    offsetof(AudioAuthenticatedPacketHeader, authTag))) {
            stats.authRejected += 1;
            continue;
          }
          data = authenticatedDatagram.data();
          stats.authDatagrams += 1;
          if (audioCrypto.rekeyGrace) {
            stats.rekeyGrace += 1;
          }
          packetMediaEpoch = audioAuthEpoch;
          packetRekeyGrace = audioCrypto.rekeyGrace;
        } else if (std::memcmp(data, "SNA1", 4) == 0) {
          if (!gExpectedSessionToken.empty()) {
            stats.authRejected += 1;
            continue;
          }
        } else {
          stats.malformed += 1;
          continue;
        }

        const std::uint16_t headerSize = readLe16Raw(data + 4);
        const std::uint16_t channels = readLe16Raw(data + 6);
        const std::uint32_t sampleRate = readLe32Raw(data + 8);
        const std::uint32_t frameCount = readLe32Raw(data + 12);
        const std::uint64_t sequence = readLe64Raw(data + 16);
        const std::uint64_t hostUnixMicros = readLe64Raw(data + 24);
        const std::uint32_t payloadSize = readLe32Raw(data + 32);
        const std::size_t expectedPayloadBytes = static_cast<std::size_t>(frameCount)
          * static_cast<std::size_t>(channels)
          * sizeof(float);

        if (headerSize < (authenticatedAudio ? sizeof(AudioAuthenticatedPacketHeader) : sizeof(AudioPacketHeader)) ||
            headerSize > size ||
            payloadSize == 0 ||
            static_cast<std::size_t>(headerSize) + payloadSize > size ||
            channels == 0 ||
            channels > 8 ||
            frameCount == 0 ||
            expectedPayloadBytes == 0 ||
            static_cast<std::size_t>(payloadSize) < expectedPayloadBytes) {
          stats.malformed += 1;
          continue;
        }
        if (!audioPeerLock.lockOrAccept(peerAddress)) {
          stats.peerRejected += 1;
          continue;
        }
        if (authenticatedAudio) {
          if (!audioCrypto.enabled) {
            stats.authRejected += 1;
            continue;
          }
          MediaReplayWindow& replayWindow = audioCrypto.rekeyGrace
            ? gPreviousAudioMediaReplay
            : gAudioMediaReplay;
          if (!replayWindow.accept(audioAuthEpoch, audioAuthSeq)) {
            stats.replayRejected += 1;
            continue;
          }
          chacha20Xor(authenticatedDatagram.data() + headerSize,
                      expectedPayloadBytes,
                      audioCrypto.cryptoKey,
                      "audio",
                      audioAuthSeq);
          data = authenticatedDatagram.data();
          if (audioCrypto.rekeyGrace) {
            stats.rekeyGraceAccepted += 1;
          }
        }

        const auto* payload = data + headerSize;
        AudioPayloadPacket packet;
        packet.sequence = sequence;
        packet.mediaEpoch = packetMediaEpoch;
        packet.hostUnixMicros = hostUnixMicros;
        packet.sampleRate = sampleRate;
        packet.channels = channels;
        packet.frameCount = frameCount;
        packet.rekeyGrace = packetRekeyGrace;
        packet.payload.assign(payload, payload + expectedPayloadBytes);
        packet.receivedAt = std::chrono::steady_clock::now();
        jitter.push(std::move(packet));

        const auto statsNow = std::chrono::steady_clock::now();
        const double statsSeconds = std::chrono::duration<double>(statsNow - statsStartedAt).count();
        if (statsSeconds >= 1.0) {
          const AudioJitterStats jitterStats = jitter.takeStats();
          const double avgAgeMs = jitterStats.ageSamples > 0
            ? jitterStats.ageSumMs / static_cast<double>(jitterStats.ageSamples)
            : -1.0;
          const double avgAvLeadMs = jitterStats.avLeadSamples > 0
            ? jitterStats.avLeadSumMs / static_cast<double>(jitterStats.avLeadSamples)
            : 0.0;
          const double avgDriftRatio = jitterStats.driftSamples > 0
            ? jitterStats.driftRatioSum / static_cast<double>(jitterStats.driftSamples)
            : 1.0;
          std::ostringstream line;
          line << std::fixed << std::setprecision(1)
               << "SNA1_AUDIO_STATS datagrams=" << stats.datagrams
               << " played=" << jitterStats.submittedPackets
               << " frames=" << jitterStats.submittedFrames
               << " lost=" << jitterStats.lostPackets
               << " reorder=" << jitterStats.reorderedPackets
               << " dup=" << jitterStats.duplicatePackets
               << " stale=" << jitterStats.staleDrops
               << " lateDrop=" << jitterStats.latencyDrops
               << " underflow=" << jitterStats.underflows
               << " queueDrop=" << jitterStats.queueDrops
               << " resets=" << jitterStats.resets
               << " generationResets=" << jitterStats.generationResets
               << " epochSequenceResets=" << jitterStats.epochSequenceResets
               << " avHold=" << jitterStats.avHoldDecisions
               << " avDrop=" << jitterStats.avSyncDrops
               << " avClockMiss=" << jitterStats.avClockMisses
               << " malformed=" << stats.malformed
               << " auth=" << stats.authDatagrams
               << " authRejected=" << stats.authRejected
               << " replayRejected=" << stats.replayRejected
               << " peerRejected=" << stats.peerRejected
               << " rekeyGrace=" << stats.rekeyGrace
               << " rekeyGraceAccepted=" << stats.rekeyGraceAccepted
               << " epochStaleDrop=" << jitterStats.epochStaleDrops
               << " epochGracePlayed=" << jitterStats.epochGraceSubmitted
               << " bufferMs=" << jitterStats.bufferMs
               << " queued=" << jitterStats.queuedPackets;
          if (jitterStats.avLeadSamples > 0) {
            line << " avLeadMs=" << avgAvLeadMs
                 << " avMinMs=" << jitterStats.avLeadMinMs
                 << " avMaxMs=" << jitterStats.avLeadMaxMs;
          } else {
            line << " avLeadMs=-- avMinMs=-- avMaxMs=--";
          }
          if (jitterStats.driftSamples > 0) {
            line << std::setprecision(2)
                 << " driftPct=" << ((avgDriftRatio - 1.0) * 100.0)
                 << " driftMinPct=" << ((jitterStats.driftRatioMin - 1.0) * 100.0)
                 << " driftMaxPct=" << ((jitterStats.driftRatioMax - 1.0) * 100.0)
                 << std::setprecision(1)
                 << " driftFrames=" << jitterStats.driftFrameDelta;
          } else {
            line << " driftPct=-- driftMinPct=-- driftMaxPct=-- driftFrames=0";
          }
          if (avgAgeMs >= 0.0) {
            line << " avgAgeMs=" << avgAgeMs
                 << " maxAgeMs=" << jitterStats.ageMaxMs;
          } else {
            line << " avgAgeMs=-- maxAgeMs=--";
          }
          std::cout << line.str() << "\n";

          stats = {};
          statsStartedAt = statsNow;
        }
      }
    } catch (const std::exception& error) {
      std::cerr << "SNA1 audio listener error: " << error.what() << "\n";
    }
  }
}

void decodeTcpStreamToRenderer(std::uint16_t port,
                               std::uint16_t controlPort,
                               std::uint16_t audioPort,
                               std::uint64_t maxPackets,
                               void* retainedRendererContext,
                               std::shared_ptr<NativeInputSender> inputSender,
                               std::shared_ptr<AvSyncClock> avSyncClock) {
  @autoreleasepool {
    try {
      ScopedFd server = createTcpListener(port, "SNV1 render");

      std::cout << "SNV1 Metal render listener ready on 0.0.0.0:" << port << "\n";
      std::cout << "Start Windows host with: --encode-pipe h264 --tcp-connect 100.100.83.44:" << port;
      if (controlPort > 0) {
        std::cout << " --control-connect 100.100.83.44:" << controlPort;
        startDedicatedControlListener(controlPort, inputSender);
      }
      if (audioPort > 0) {
        std::cout << " --audio-udp-connect 100.100.83.44:" << audioPort;
      }
      std::cout << "\n";

      std::uint64_t totalPackets = 0;
      while (maxPackets == 0 || totalPackets < maxPackets) {
        sockaddr_in peerAddress{};
        socklen_t peerLength = sizeof(peerAddress);
        ScopedFd client(accept(server.get(), reinterpret_cast<sockaddr*>(&peerAddress), &peerLength));
        if (client.get() < 0) {
          throw std::runtime_error("TCP accept failed.");
        }
        configureAcceptedTcpSocket(client.get());

        char peerIp[INET_ADDRSTRLEN]{};
        inet_ntop(AF_INET, &peerAddress.sin_addr, peerIp, sizeof(peerIp));
        std::cout << "SNV1 render client connected from " << peerIp << ":" << ntohs(peerAddress.sin_port) << "\n";
        if (inputSender && controlPort == 0) {
          inputSender->setFd(client.get());
          std::cout << "SNINPUT fallback control enabled on render TCP socket.\n";
        } else if (inputSender) {
          std::cout << "SNINPUT waiting for dedicated control socket.\n";
        }

        VtH264Decoder decoder(submitFrameToVideoRenderer, retainedRendererContext);
        DecodeSummary summary;
        ClientStreamStats stats;
        std::uint64_t lastFeedbackDecodeErrors = 0;
        while (maxPackets == 0 || totalPackets < maxPackets) {
          SnvPacket packet;
          if (!readSnvPacketFromFd(client.get(), packet)) {
            break;
          }
          if (avSyncClock) avSyncClock->observeVideoPacket(packet.hostUnixMicros);
          stats.observe(packet);
          processSnvPacket(decoder, summary, packet);
          totalPackets += 1;
          if (summary.packets % 60 == 0) {
            std::cout << "SNV1 render packets=" << summary.packets
                      << " total=" << totalPackets
                      << " decoded=" << decoder.decodedFrames()
                      << " errors=" << decoder.decodeErrors()
                      << "\n";
            const double avgAgeMs = stats.ageSamples > 0
              ? stats.ageSumMs / static_cast<double>(stats.ageSamples)
              : -1.0;
            const std::uint64_t currentDecodeErrors = decoder.decodeErrors();
            const std::uint64_t decodeErrorDelta = currentDecodeErrors >= lastFeedbackDecodeErrors
              ? currentDecodeErrors - lastFeedbackDecodeErrors
              : currentDecodeErrors;
            std::ostringstream statLine;
            statLine << std::fixed << std::setprecision(1)
                     << "SNV1_CLIENT_STATS packets=" << stats.windowPackets
                     << " decoded=" << decoder.decodedFrames()
                     << " dropped=" << stats.windowDropped
                     << " totalDropped=" << stats.totalDropped
                     << " decodeErrors=" << currentDecodeErrors
                     << " jitterMs=" << stats.jitterMs;
            if (avgAgeMs >= 0.0) {
              statLine << " avgAgeMs=" << avgAgeMs
                       << " maxAgeMs=" << stats.ageMaxMs;
            } else {
              statLine << " avgAgeMs=-- maxAgeMs=--";
            }
            std::cout << statLine.str() << "\n";

            if (stats.windowDropped > 0) {
              sendKeyframeRequest("tcp-sequence-gap",
                                  summary.lastSequence,
                                  stats.windowDropped,
                                  currentDecodeErrors);
            } else if (decodeErrorDelta > 0) {
              sendKeyframeRequest("decode-error",
                                  summary.lastSequence,
                                  decodeErrorDelta,
                                  currentDecodeErrors);
            }

            std::ostringstream feedback;
            feedback << std::fixed << std::setprecision(1)
                     << "{\"type\":\"stream-stats\""
                     << ",\"packets\":" << stats.windowPackets
                     << ",\"decoded\":" << decoder.decodedFrames()
                     << ",\"dropped\":" << stats.windowDropped
                     << ",\"totalDropped\":" << stats.totalDropped
                     << ",\"decodeErrors\":" << currentDecodeErrors
                     << ",\"jitterMs\":" << stats.jitterMs
                     << ",\"avgAgeMs\":" << avgAgeMs
                     << ",\"maxAgeMs\":" << (avgAgeMs >= 0.0 ? stats.ageMaxMs : -1.0)
                     << "}";
            sendNativeInputJson(feedback.str());
            lastFeedbackDecodeErrors = currentDecodeErrors;
            stats.resetWindow();
          }
        }

        decoder.flush();
        printDecodeSummary("tcp-render:" + std::to_string(port), summary, decoder);
        if (inputSender && controlPort == 0) {
          inputSender->clearFd(client.get());
        }
        std::cout << "SNV1 render client disconnected; listener stays open for reconnect.\n";
      }
    } catch (const std::exception& error) {
      std::cerr << "SNV1 render listener error: " << error.what() << "\n";
    }

    CFRelease((CFTypeRef)retainedRendererContext);
    dispatch_async(dispatch_get_main_queue(), ^{
      [NSApp terminate:nil];
    });
  }
}

void decodeUdpStreamToRenderer(std::uint16_t port,
                               std::uint16_t controlPort,
                               std::uint16_t audioPort,
                               std::uint64_t maxPackets,
                               void* retainedRendererContext,
                               std::shared_ptr<NativeInputSender> inputSender,
                               std::shared_ptr<AvSyncClock> avSyncClock) {
  @autoreleasepool {
    try {
      ScopedFd server = createUdpListener(port, "SNU1 render");

      std::cout << "SNU1 UDP render listener ready on 0.0.0.0:" << port
                << " mediaCrypto=" << boolText(gMediaCryptoEnabled)
                << "\n";
      std::cout << "Start Windows host with: --encode-pipe h264 --udp-connect 100.100.83.44:" << port;
      if (controlPort > 0) {
        std::cout << " --control-connect 100.100.83.44:" << controlPort;
        startDedicatedControlListener(controlPort, inputSender);
      }
      if (audioPort > 0) {
        std::cout << " --audio-udp-connect 100.100.83.44:" << audioPort;
      }
      std::cout << "\n";

      VtH264Decoder decoder(submitFrameToVideoRenderer, retainedRendererContext);
      DecodeSummary summary;
      ClientStreamStats stats;
      UdpVideoReassembler reassembler;
      UdpVideoNackController nackController;
      UdpVideoPacketJitterBuffer jitterBuffer;
      UdpPeerLock videoPeerLock("SNU1");
      UdpVideoStats udpStats;
      auto udpStatsStartedAt = std::chrono::steady_clock::now();
      std::array<std::uint8_t, 1500> datagramBuffer{};
      std::uint64_t lastFeedbackDecodeErrors = 0;
      std::uint64_t repairFeedbackSequence = 0;
      while (maxPackets == 0 || summary.packets < maxPackets) {
        sockaddr_in peerAddress{};
        socklen_t peerLength = sizeof(peerAddress);
        const ssize_t received = recvfrom(server.get(),
                                          datagramBuffer.data(),
                                          datagramBuffer.size(),
                                          0,
                                          reinterpret_cast<sockaddr*>(&peerAddress),
                                          &peerLength);
        if (received < 0) {
          if (errno == EINTR) continue;
          throw std::runtime_error("UDP receive failed.");
        }
        if (received == 0) continue;

        const std::uint64_t mediaGeneration = gMediaPeerGeneration.load(std::memory_order_relaxed);
        const bool jitterGenerationReset = jitterBuffer.resetForMediaGeneration(mediaGeneration);
        const bool nackGenerationReset = nackController.resetForMediaGeneration(mediaGeneration);
        if (jitterGenerationReset || nackGenerationReset) {
          stats.resetSequencing();
        }

        std::vector<std::uint8_t> datagram(datagramBuffer.begin(),
                                           datagramBuffer.begin() + static_cast<std::ptrdiff_t>(received));
        std::vector<std::uint8_t> packetBytes;
        std::uint64_t completedPacketId = 0;
        std::uint64_t completedMediaEpoch = 0;
        bool completedRekeyGrace = false;
        const bool completedPacket = reassembler.push(std::move(datagram),
                                                      packetBytes,
                                                      udpStats,
                                                      completedPacketId,
                                                      completedMediaEpoch,
                                                      completedRekeyGrace,
                                                      peerAddress,
                                                      videoPeerLock);
        nackController.observeMissing(udpStats.newNackPacketIds);
        udpStats.newNackPacketIds.clear();
        if (completedPacketId != 0) {
          nackController.markRecovered(completedPacketId);
        }
        nackController.maybeSend("udp-loss");
        nackController.maybeRequestKeyframe(summary.lastSequence, decoder.decodeErrors());

        if (completedPacket) {
          jitterBuffer.push(parseSnvPacketBytes(packetBytes), completedMediaEpoch, completedRekeyGrace);
        }

        SnvPacket packet;
        while (jitterBuffer.popReady(packet)) {
          if (avSyncClock) avSyncClock->observeVideoPacket(packet.hostUnixMicros);
          stats.observe(packet);
          processSnvPacket(decoder, summary, packet);

          if (summary.packets == 1 || summary.packets % 60 == 0) {
            std::cout << "SNU1 render packets=" << summary.packets
                      << " decoded=" << decoder.decodedFrames()
                      << " errors=" << decoder.decodeErrors()
                      << "\n";
            const double avgAgeMs = stats.ageSamples > 0
              ? stats.ageSumMs / static_cast<double>(stats.ageSamples)
              : -1.0;
            const std::uint64_t currentDecodeErrors = decoder.decodeErrors();
            const std::uint64_t decodeErrorDelta = currentDecodeErrors >= lastFeedbackDecodeErrors
              ? currentDecodeErrors - lastFeedbackDecodeErrors
              : currentDecodeErrors;
            std::ostringstream statLine;
            statLine << std::fixed << std::setprecision(1)
                     << "SNV1_CLIENT_STATS packets=" << stats.windowPackets
                     << " decoded=" << decoder.decodedFrames()
                     << " dropped=" << stats.windowDropped
                     << " totalDropped=" << stats.totalDropped
                     << " decodeErrors=" << currentDecodeErrors
                     << " jitterMs=" << stats.jitterMs;
            if (avgAgeMs >= 0.0) {
              statLine << " avgAgeMs=" << avgAgeMs
                       << " maxAgeMs=" << stats.ageMaxMs;
            } else {
              statLine << " avgAgeMs=-- maxAgeMs=--";
            }
            std::cout << statLine.str() << "\n";

            if (stats.windowDropped > 0) {
              sendKeyframeRequest("udp-sequence-gap",
                                  summary.lastSequence,
                                  stats.windowDropped,
                                  currentDecodeErrors);
            } else if (decodeErrorDelta > 0) {
              sendKeyframeRequest("decode-error",
                                  summary.lastSequence,
                                  decodeErrorDelta,
                                  currentDecodeErrors);
            }

            std::ostringstream feedback;
            feedback << std::fixed << std::setprecision(1)
                     << "{\"type\":\"stream-stats\""
                     << ",\"packets\":" << stats.windowPackets
                     << ",\"decoded\":" << decoder.decodedFrames()
                     << ",\"dropped\":" << stats.windowDropped
                     << ",\"totalDropped\":" << stats.totalDropped
                     << ",\"decodeErrors\":" << currentDecodeErrors
                     << ",\"jitterMs\":" << stats.jitterMs
                     << ",\"avgAgeMs\":" << avgAgeMs
                     << ",\"maxAgeMs\":" << (avgAgeMs >= 0.0 ? stats.ageMaxMs : -1.0)
                     << "}";
            sendNativeInputJson(feedback.str());
            lastFeedbackDecodeErrors = currentDecodeErrors;
            stats.resetWindow();
          }
        }

        const auto statsNow = std::chrono::steady_clock::now();
        const double statsSeconds = std::chrono::duration<double>(statsNow - udpStatsStartedAt).count();
        if (statsSeconds >= 1.0) {
          const UdpVideoNackWindowStats nackWindow = nackController.takeWindowStats();
          const UdpVideoJitterWindowStats jitterWindow = jitterBuffer.takeWindowStats();
          std::cout << "SNU1_STATS datagrams=" << udpStats.datagrams
                    << " fragments=" << udpStats.fragments
                    << " retransmitFragments=" << udpStats.retransmitFragments
                    << " completed=" << udpStats.completedPackets
                    << " retransmitCompleted=" << udpStats.retransmitCompletedPackets
                    << " droppedAssemblies=" << udpStats.droppedAssemblies
                    << " malformed=" << udpStats.malformedDatagrams
                    << " auth=" << udpStats.authDatagrams
                    << " authRejected=" << udpStats.authRejectedDatagrams
                    << " replayRejected=" << udpStats.replayRejectedDatagrams
                    << " peerRejected=" << udpStats.peerRejectedDatagrams
                    << " rekeyGrace=" << udpStats.rekeyGraceDatagrams
                    << " rekeyGraceCompleted=" << udpStats.rekeyGraceCompletedPackets
                    << " epochResetAssemblies=" << udpStats.epochResetAssemblies
                    << " duplicateFragments=" << udpStats.duplicateFragments
                    << " nackPackets=" << udpStats.nackPacketIds.size()
                    << " nackPending=" << nackController.pendingCount()
                    << " nackSent=" << nackWindow.sentPackets
                    << " nackRecovered=" << nackWindow.recoveredPackets
                    << " nackTimedOut=" << nackWindow.timedOutPackets
                    << " jitterPending=" << jitterBuffer.pendingCount()
                    << " jitterReleased=" << jitterWindow.releasedPackets
                    << " jitterHeld=" << jitterWindow.heldTicks
                    << " jitterSkipped=" << jitterWindow.skippedSequences
                    << " jitterLate=" << jitterWindow.lateDrops
                    << " jitterDup=" << jitterWindow.duplicateDrops
                    << " jitterKeyframeReset=" << jitterWindow.keyframeResets
                    << " jitterGenerationReset=" << jitterWindow.generationResets
                    << " jitterEpochStaleDrop=" << jitterWindow.epochStaleDrops
                    << " jitterEpochGraceRelease=" << jitterWindow.epochGraceReleases
                    << "\n";
          const std::uint64_t repairSequence = ++repairFeedbackSequence;
          std::ostringstream repairFeedback;
          repairFeedback << "{\"type\":\"udp-repair-stats\""
                         << ",\"sequence\":" << repairSequence
                         << ",\"datagrams\":" << udpStats.datagrams
                         << ",\"fragments\":" << udpStats.fragments
                         << ",\"retransmitFragments\":" << udpStats.retransmitFragments
                         << ",\"completed\":" << udpStats.completedPackets
                         << ",\"retransmitCompleted\":" << udpStats.retransmitCompletedPackets
                         << ",\"droppedAssemblies\":" << udpStats.droppedAssemblies
                         << ",\"malformed\":" << udpStats.malformedDatagrams
                         << ",\"authDatagrams\":" << udpStats.authDatagrams
                         << ",\"authRejected\":" << udpStats.authRejectedDatagrams
                         << ",\"replayRejected\":" << udpStats.replayRejectedDatagrams
                         << ",\"peerRejected\":" << udpStats.peerRejectedDatagrams
                         << ",\"rekeyGrace\":" << udpStats.rekeyGraceDatagrams
                         << ",\"rekeyGraceCompleted\":" << udpStats.rekeyGraceCompletedPackets
                         << ",\"epochResetAssemblies\":" << udpStats.epochResetAssemblies
                         << ",\"duplicateFragments\":" << udpStats.duplicateFragments
                         << ",\"nackPackets\":" << udpStats.nackPacketIds.size()
                         << ",\"nackPending\":" << nackController.pendingCount()
                         << ",\"nackSent\":" << nackWindow.sentPackets
                         << ",\"nackRecovered\":" << nackWindow.recoveredPackets
                         << ",\"nackTimedOut\":" << nackWindow.timedOutPackets
                         << ",\"jitterPending\":" << jitterBuffer.pendingCount()
                         << ",\"jitterReleased\":" << jitterWindow.releasedPackets
                         << ",\"jitterHeld\":" << jitterWindow.heldTicks
                         << ",\"jitterSkipped\":" << jitterWindow.skippedSequences
                         << ",\"jitterLate\":" << jitterWindow.lateDrops
                         << ",\"jitterDup\":" << jitterWindow.duplicateDrops
                         << ",\"jitterKeyframeReset\":" << jitterWindow.keyframeResets
                         << ",\"jitterGenerationReset\":" << jitterWindow.generationResets
                         << ",\"jitterEpochStaleDrop\":" << jitterWindow.epochStaleDrops
                         << ",\"jitterEpochGraceRelease\":" << jitterWindow.epochGraceReleases
                         << ",\"decoded\":" << decoder.decodedFrames()
                         << ",\"decodeErrors\":" << decoder.decodeErrors()
                         << ",\"videoSequence\":" << summary.lastSequence
                         << ",\"sentSteadyMicros\":" << steadyMicros()
                         << "}";
          const bool repairSent = sendNativeInputJson(repairFeedback.str());
          std::cout << "SNU1_REPAIR_FEEDBACK sequence=" << repairSequence
                    << " sent=" << boolText(repairSent)
                    << " nackSent=" << nackWindow.sentPackets
                    << " nackRecovered=" << nackWindow.recoveredPackets
                    << " nackTimedOut=" << nackWindow.timedOutPackets
                    << " jitterSkipped=" << jitterWindow.skippedSequences
                    << " replayRejected=" << udpStats.replayRejectedDatagrams
                    << " peerRejected=" << udpStats.peerRejectedDatagrams
                    << " rekeyGrace=" << udpStats.rekeyGraceDatagrams
                    << " rekeyGraceCompleted=" << udpStats.rekeyGraceCompletedPackets
                    << " epochResetAssemblies=" << udpStats.epochResetAssemblies
                    << " jitterEpochStaleDrop=" << jitterWindow.epochStaleDrops
                    << " jitterEpochGraceRelease=" << jitterWindow.epochGraceReleases
                    << "\n";
          if (udpStats.malformedDatagrams > 0 && udpStats.nackPacketIds.empty()) {
            sendKeyframeRequest("udp-loss",
                                summary.lastSequence,
                                udpStats.malformedDatagrams,
                                decoder.decodeErrors());
          }
          udpStats = {};
          udpStatsStartedAt = statsNow;
        }
      }

      decoder.flush();
      printDecodeSummary("udp-render:" + std::to_string(port), summary, decoder);
    } catch (const std::exception& error) {
      std::cerr << "SNU1 UDP render listener error: " << error.what() << "\n";
    }

    CFRelease((CFTypeRef)retainedRendererContext);
    dispatch_async(dispatch_get_main_queue(), ^{
      [NSApp terminate:nil];
    });
  }
}

int runVideoRenderTcp(std::uint16_t port,
                      std::uint16_t controlPort,
                      std::uint16_t audioPort,
                      double audioJitterMs,
                      bool udpVideo,
                      std::uint64_t maxPackets,
                      bool fullscreen,
                      bool hideCursor,
                      bool relativeMouse) {
  @autoreleasepool {
    id<MTLDevice> device = defaultMetalDevice();
    if (!device) {
      throw std::runtime_error("Metal is not available on this Mac.");
    }

    gRelativeMouse = relativeMouse;
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];

    const NSRect frame = NSMakeRect(0, 0, 1280, 720);
    const NSWindowStyleMask style = NSWindowStyleMaskTitled
                                  | NSWindowStyleMaskClosable
                                  | NSWindowStyleMaskMiniaturizable
                                  | NSWindowStyleMaskResizable;
    NSWindow* window = [[NSWindow alloc] initWithContentRect:frame
                                                   styleMask:style
                                                     backing:NSBackingStoreBuffered
                                                       defer:NO];
    [window setTitle:@"Sanser Native Client - SNV1 Metal Render"];
    [window setCollectionBehavior:NSWindowCollectionBehaviorFullScreenPrimary
                                  | NSWindowCollectionBehaviorFullScreenAllowsTiling];

    SanserMetalView* view = [[SanserMetalView alloc] initWithFrame:frame device:device];
    [view setColorPixelFormat:MTLPixelFormatBGRA8Unorm];
    [view setPreferredFramesPerSecond:120];
    [view setPaused:NO];
    [view setEnableSetNeedsDisplay:NO];

    SanserVideoRenderer* renderer = [[SanserVideoRenderer alloc] initWithDevice:device view:view];
    if (!renderer) {
      throw std::runtime_error("Failed to create video Metal renderer.");
    }

    [view setDelegate:renderer];
    [window setContentView:view];
    [window center];
    [window makeKeyAndOrderFront:nil];
    [window makeFirstResponder:view];
    [NSApp activateIgnoringOtherApps:YES];
    if (fullscreen) {
      [window toggleFullScreen:nil];
    }
    if (hideCursor) {
      [NSCursor hide];
    }
    if (relativeMouse) {
      setRelativeMouseGrab(true);
    }

    id resignObserver = [[NSNotificationCenter defaultCenter] addObserverForName:NSApplicationDidResignActiveNotification
                                                                           object:nil
                                                                            queue:[NSOperationQueue mainQueue]
                                                                       usingBlock:^(NSNotification* notification) {
                                                                         (void)notification;
                                                                         restoreRelativeMouseGrab();
                                                                       }];
    id activeObserver = [[NSNotificationCenter defaultCenter] addObserverForName:NSApplicationDidBecomeActiveNotification
                                                                          object:nil
                                                                           queue:[NSOperationQueue mainQueue]
                                                                      usingBlock:^(NSNotification* notification) {
                                                                        (void)notification;
                                                                        if (gRelativeMouse) setRelativeMouseGrab(true);
                                                                      }];
    id closeObserver = [[NSNotificationCenter defaultCenter] addObserverForName:NSWindowWillCloseNotification
                                                                          object:window
                                                                           queue:[NSOperationQueue mainQueue]
                                                                      usingBlock:^(NSNotification* notification) {
                                                                        (void)notification;
                                                                        restoreRelativeMouseGrab();
                                                                        [NSApp terminate:nil];
                                                                      }];

    auto inputSender = std::make_shared<NativeInputSender>();
    inputSender->setAuthRequired(!gExpectedSessionToken.empty(), gExpectedSessionToken);
    gNativeInputSender = inputSender;
    auto avSyncClock = std::make_shared<AvSyncClock>();

    if (audioPort > 0) {
      auto audioPlayer = std::make_shared<AudioQueuePcmPlayer>();
      std::thread audioWorker([audioPort, audioJitterMs, audioPlayer, avSyncClock]() {
        listenUdpAudio(audioPort, audioJitterMs, audioPlayer, avSyncClock);
      });
      audioWorker.detach();
    }

    void* rendererContext = (__bridge_retained void*)renderer;
    std::thread worker([port, controlPort, audioPort, udpVideo, maxPackets, rendererContext, inputSender, avSyncClock]() {
      if (udpVideo) {
        decodeUdpStreamToRenderer(port, controlPort, audioPort, maxPackets, rendererContext, inputSender, avSyncClock);
      } else {
        decodeTcpStreamToRenderer(port, controlPort, audioPort, maxPackets, rendererContext, inputSender, avSyncClock);
      }
    });
    worker.detach();

    std::cout << "Metal video renderer running on " << nsStringToUtf8([device name]) << "\n";
    std::cout << "Native renderer mode fullscreen=" << boolText(fullscreen)
              << " hideCursor=" << boolText(hideCursor)
              << " relativeMouse=" << boolText(relativeMouse)
              << " videoTransport=" << (udpVideo ? "udp" : "tcp")
              << " sessionAuth=" << boolText(!gExpectedSessionToken.empty())
              << " controlPort=" << controlPort
              << " audioPort=" << audioPort
              << " audioJitterMs=" << std::fixed << std::setprecision(1) << audioJitterMs
              << "\n";
    [NSApp run];
    [[NSNotificationCenter defaultCenter] removeObserver:resignObserver];
    [[NSNotificationCenter defaultCenter] removeObserver:activeObserver];
    [[NSNotificationCenter defaultCenter] removeObserver:closeObserver];
    restoreRelativeMouseGrab();
    if (hideCursor) {
      [NSCursor unhide];
    }
  }
  return 0;
}

int runMetalTest(double seconds) {
  @autoreleasepool {
    id<MTLDevice> device = defaultMetalDevice();
    if (!device) {
      throw std::runtime_error("Metal is not available on this Mac.");
    }

    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];

    const NSRect frame = NSMakeRect(0, 0, 960, 540);
    const NSWindowStyleMask style = NSWindowStyleMaskTitled
                                  | NSWindowStyleMaskClosable
                                  | NSWindowStyleMaskMiniaturizable
                                  | NSWindowStyleMaskResizable;
    NSWindow* window = [[NSWindow alloc] initWithContentRect:frame
                                                   styleMask:style
                                                     backing:NSBackingStoreBuffered
                                                       defer:NO];
    [window setTitle:@"Sanser Native Client - Metal Render Test"];

    SanserMetalView* view = [[SanserMetalView alloc] initWithFrame:frame device:device];
    [view setColorPixelFormat:MTLPixelFormatBGRA8Unorm];
    [view setPreferredFramesPerSecond:60];
    [view setPaused:NO];
    [view setEnableSetNeedsDisplay:NO];

    SanserMetalRenderer* renderer = [[SanserMetalRenderer alloc] initWithDevice:device view:view];
    if (!renderer) {
      throw std::runtime_error("Failed to create Metal renderer.");
    }
    [view setDelegate:renderer];
    [window setContentView:view];
    [window center];
    [window makeKeyAndOrderFront:nil];
    [window makeFirstResponder:view];
    [NSApp activateIgnoringOtherApps:YES];

    std::cout << "Metal render test running on " << nsStringToUtf8([device name]) << "\n";
    std::cout << "Move mouse, click, type, or scroll to see SNINPUT events.\n";

    if (seconds > 0) {
      [NSTimer scheduledTimerWithTimeInterval:seconds
                                      repeats:NO
                                        block:^(NSTimer*) {
                                          [NSApp terminate:nil];
                                        }];
    }

    [NSApp run];
  }
  return 0;
}

struct Options {
  bool probe = false;
  bool decodeSnv = false;
  bool listenSnv = false;
  bool listenRenderSnv = false;
  bool metalTest = false;
  bool clipboardRead = false;
  bool clipboardWrite = false;
  bool logInput = false;
  bool fullscreen = false;
  bool hideCursor = false;
  bool relativeMouse = false;
  bool udpVideo = false;
  bool help = false;
  std::uint64_t maxPackets = 0;
  std::uint16_t listenPort = 0;
  std::uint16_t listenRenderPort = 0;
  std::uint16_t controlPort = 0;
  std::uint16_t audioPort = 0;
  bool controlPortProvided = false;
  bool audioPortProvided = false;
  double audioJitterMs = 24.0;
  double seconds = 5;
  std::string snvFile;
  std::string clipboardText;
  std::string sessionToken;
};

Options parseOptions(int argc, char** argv) {
  Options options;
  if (argc <= 1) {
    options.probe = true;
    return options;
  }

  for (int i = 1; i < argc; ++i) {
    const std::string arg = argv[i];
    if (arg == "--probe") {
      options.probe = true;
    } else if (arg == "--decode-snv") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --decode-snv");
      options.decodeSnv = true;
      options.snvFile = argv[++i];
    } else if (arg == "--listen-snv") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --listen-snv");
      const auto port = std::stoul(argv[++i]);
      if (port == 0 || port > 65535) throw std::runtime_error("--listen-snv port must be 1-65535");
      options.listenSnv = true;
      options.listenPort = static_cast<std::uint16_t>(port);
    } else if (arg == "--listen-render-snv") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --listen-render-snv");
      const auto port = std::stoul(argv[++i]);
      if (port == 0 || port > 65535) throw std::runtime_error("--listen-render-snv port must be 1-65535");
      options.listenRenderSnv = true;
      options.listenRenderPort = static_cast<std::uint16_t>(port);
    } else if (arg == "--max-packets") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --max-packets");
      options.maxPackets = static_cast<std::uint64_t>(std::stoull(argv[++i]));
    } else if (arg == "--control-port") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --control-port");
      const auto port = std::stoul(argv[++i]);
      if (port > 65535) throw std::runtime_error("--control-port must be 0-65535");
      options.controlPort = static_cast<std::uint16_t>(port);
      options.controlPortProvided = true;
    } else if (arg == "--audio-port") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --audio-port");
      const auto port = std::stoul(argv[++i]);
      if (port > 65535) throw std::runtime_error("--audio-port must be 0-65535");
      options.audioPort = static_cast<std::uint16_t>(port);
      options.audioPortProvided = true;
    } else if (arg == "--audio-jitter-ms") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --audio-jitter-ms");
      options.audioJitterMs = std::clamp(std::atof(argv[++i]), 0.0, 120.0);
    } else if (arg == "--session-token") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --session-token");
      options.sessionToken = argv[++i];
    } else if (arg == "--metal-test") {
      options.metalTest = true;
    } else if (arg == "--log-input") {
      options.logInput = true;
    } else if (arg == "--fullscreen") {
      options.fullscreen = true;
    } else if (arg == "--hide-cursor") {
      options.hideCursor = true;
    } else if (arg == "--relative-mouse") {
      options.relativeMouse = true;
    } else if (arg == "--udp-video") {
      options.udpVideo = true;
    } else if (arg == "--clipboard-read") {
      options.clipboardRead = true;
    } else if (arg == "--clipboard-write") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --clipboard-write");
      options.clipboardWrite = true;
      options.clipboardText = argv[++i];
    } else if (arg == "--seconds") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --seconds");
      options.seconds = std::atof(argv[++i]);
    } else if (arg == "--help" || arg == "-h") {
      options.help = true;
    } else {
      throw std::runtime_error("Unknown argument: " + arg);
    }
  }
  return options;
}

std::uint16_t defaultControlPort(std::uint16_t videoPort) {
  return videoPort < 65535 ? static_cast<std::uint16_t>(videoPort + 1) : 0;
}

std::uint16_t defaultAudioPort(std::uint16_t videoPort) {
  return videoPort < 65534 ? static_cast<std::uint16_t>(videoPort + 2) : 0;
}

} // namespace

int main(int argc, char** argv) {
  try {
    Options options = parseOptions(argc, argv);
    if (options.sessionToken.empty()) {
      if (const char* token = std::getenv("SANSER_NATIVE_SESSION_TOKEN")) {
        options.sessionToken = token;
      }
    }
    gLogInputEvents = options.logInput;
    gExpectedSessionToken = options.sessionToken;
    if (!gExpectedSessionToken.empty()) {
      activateMediaCryptoKeys(0, "bootstrap");
    }
    if (options.help) {
      printHelp();
      return 0;
    }
    if (options.probe) return runProbe();
    if (options.decodeSnv) return decodeSnvFile(options.snvFile, options.maxPackets);
    if (options.listenSnv) return listenSnvTcp(options.listenPort, options.maxPackets);
    if (options.listenRenderSnv) {
      const std::uint16_t controlPort = options.controlPortProvided
        ? options.controlPort
        : defaultControlPort(options.listenRenderPort);
      const std::uint16_t audioPort = options.audioPortProvided
        ? options.audioPort
        : defaultAudioPort(options.listenRenderPort);
      if (controlPort > 0 && controlPort == options.listenRenderPort) {
        throw std::runtime_error("--control-port must differ from --listen-render-snv port, or use 0 to disable.");
      }
      if (audioPort > 0 && audioPort == options.listenRenderPort) {
        throw std::runtime_error("--audio-port must differ from --listen-render-snv port, or use 0 to disable.");
      }
      if (audioPort > 0 && controlPort > 0 && audioPort == controlPort) {
        throw std::runtime_error("--audio-port must differ from --control-port, or use 0 to disable.");
      }
      return runVideoRenderTcp(options.listenRenderPort,
                               controlPort,
                               audioPort,
                               options.audioJitterMs,
                               options.udpVideo,
                               options.maxPackets,
                               options.fullscreen,
                               options.hideCursor,
                               options.relativeMouse);
    }
    if (options.clipboardRead) return clipboardRead();
    if (options.clipboardWrite) return clipboardWrite(options.clipboardText);
    if (options.metalTest) return runMetalTest(options.seconds);

    printHelp();
    return 0;
  } catch (const std::exception& error) {
    std::cerr << "sanser-native-client error: " << error.what() << "\n";
    return 1;
  }
}
