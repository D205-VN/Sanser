#include "bmp_writer.h"
#include "desktop_duplication.h"
#include "mf_video_encoder.h"
#include "mf_video_packet_encoder.h"

#include <algorithm>
#include <atomic>
#include <array>
#include <chrono>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <map>
#include <memory>
#include <mutex>
#include <sstream>
#include <string>
#include <string_view>
#include <thread>
#include <utility>
#include <vector>

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <audioclient.h>
#include <avrt.h>
#include <bcrypt.h>
#include <ks.h>
#include <ksmedia.h>
#include <mmdeviceapi.h>
#include <mmreg.h>
#include <wrl/client.h>
#endif

struct InputBounds {
  int left = 0;
  int top = 0;
  int width = 1;
  int height = 1;
};

InputBounds gInputBounds;
std::mutex gInputBoundsMutex;

struct StreamFeedbackSnapshot {
  std::string source = "network";
  std::uint64_t sequence = 0;
  std::uint64_t packets = 0;
  std::uint64_t dropped = 0;
  std::uint64_t totalDropped = 0;
  double jitterMs = 0.0;
  double avgAgeMs = -1.0;
  double maxAgeMs = -1.0;
  std::uint64_t rendered = 0;
  std::uint64_t renderQueueDepth = 0;
  std::uint64_t renderDroppedQueue = 0;
  std::uint64_t renderDroppedLate = 0;
  std::uint64_t renderHeld = 0;
  std::uint64_t renderPacingReset = 0;
  double renderTargetDelayMs = -1.0;
  double renderAdaptiveDelayMs = -1.0;
  std::uint64_t renderAdaptiveUp = 0;
  std::uint64_t renderAdaptiveDown = 0;
  double renderAvgAgeMs = -1.0;
  double renderMaxAgeMs = -1.0;
  double renderAvgPresentLateMs = -1.0;
  double renderMaxPresentLateMs = -1.0;
  int pressure = 0;
};

StreamFeedbackSnapshot gStreamFeedback;
std::mutex gStreamFeedbackMutex;

struct UdpRepairFeedbackSnapshot {
  std::uint64_t sequence = 0;
  std::uint64_t clientSequence = 0;
  std::uint64_t datagrams = 0;
  std::uint64_t fragments = 0;
  std::uint64_t retransmitFragments = 0;
  std::uint64_t completed = 0;
  std::uint64_t retransmitCompleted = 0;
  std::uint64_t droppedAssemblies = 0;
  std::uint64_t malformed = 0;
  std::uint64_t authRejected = 0;
  std::uint64_t replayRejected = 0;
  std::uint64_t peerRejected = 0;
  std::uint64_t rekeyGrace = 0;
  std::uint64_t rekeyGraceCompleted = 0;
  std::uint64_t epochResetAssemblies = 0;
  std::uint64_t duplicateFragments = 0;
  std::uint64_t nackPackets = 0;
  std::uint64_t nackPending = 0;
  std::uint64_t nackSent = 0;
  std::uint64_t nackRecovered = 0;
  std::uint64_t nackTimedOut = 0;
  std::uint64_t jitterPending = 0;
  std::uint64_t jitterReleased = 0;
  std::uint64_t jitterHeld = 0;
  std::uint64_t jitterSkipped = 0;
  std::uint64_t jitterLate = 0;
  std::uint64_t jitterDup = 0;
  std::uint64_t jitterKeyframeReset = 0;
  std::uint64_t jitterGenerationReset = 0;
  std::uint64_t jitterEpochStaleDrop = 0;
  std::uint64_t jitterEpochGraceRelease = 0;
  std::uint64_t decoded = 0;
  std::uint64_t decodeErrors = 0;
  std::uint64_t videoSequence = 0;
  int pressure = 0;
};

UdpRepairFeedbackSnapshot gUdpRepairFeedback;
std::mutex gUdpRepairFeedbackMutex;

struct KeyframeRequestSnapshot {
  std::uint64_t sequence = 0;
  std::uint64_t clientSequence = 0;
  std::uint64_t videoSequence = 0;
  std::uint64_t dropped = 0;
  std::uint64_t decodeErrors = 0;
  std::uint64_t requestedAtMicros = 0;
  std::string reason;
};

KeyframeRequestSnapshot gKeyframeRequest;
std::mutex gKeyframeRequestMutex;

struct VideoNackSnapshot {
  std::uint64_t sequence = 0;
  std::uint64_t clientSequence = 0;
  std::uint64_t requestedAtMicros = 0;
  std::string reason;
  std::vector<std::uint64_t> packetIds;
};

VideoNackSnapshot gVideoNack;
std::mutex gVideoNackMutex;

std::uint64_t unixMicros();

const char* pressureLabel(int pressure) {
  if (pressure >= 2) return "severe";
  if (pressure == 1) return "congested";
  if (pressure < 0) return "clear";
  return "stable";
}

StreamFeedbackSnapshot latestStreamFeedback() {
  std::lock_guard<std::mutex> lock(gStreamFeedbackMutex);
  return gStreamFeedback;
}

UdpRepairFeedbackSnapshot latestUdpRepairFeedback() {
  std::lock_guard<std::mutex> lock(gUdpRepairFeedbackMutex);
  return gUdpRepairFeedback;
}

KeyframeRequestSnapshot latestKeyframeRequest() {
  std::lock_guard<std::mutex> lock(gKeyframeRequestMutex);
  return gKeyframeRequest;
}

void requestKeyframe(const std::string& reason,
                     std::uint64_t clientSequence,
                     std::uint64_t videoSequence,
                     std::uint64_t dropped,
                     std::uint64_t decodeErrors) {
  KeyframeRequestSnapshot snapshot;
  {
    std::lock_guard<std::mutex> lock(gKeyframeRequestMutex);
    snapshot.sequence = gKeyframeRequest.sequence + 1;
    snapshot.clientSequence = clientSequence;
    snapshot.videoSequence = videoSequence;
    snapshot.dropped = dropped;
    snapshot.decodeErrors = decodeErrors;
    snapshot.requestedAtMicros = unixMicros();
    snapshot.reason = reason.empty() ? "client" : reason;
    gKeyframeRequest = snapshot;
  }

  std::cerr << "SNV1_KEYFRAME_REQUEST_RX sequence=" << snapshot.sequence
            << " clientSequence=" << snapshot.clientSequence
            << " reason=" << snapshot.reason
            << " videoSequence=" << snapshot.videoSequence
            << " dropped=" << snapshot.dropped
            << " decodeErrors=" << snapshot.decodeErrors
            << "\n";
}

VideoNackSnapshot latestVideoNack() {
  std::lock_guard<std::mutex> lock(gVideoNackMutex);
  return gVideoNack;
}

void requestVideoRetransmit(const std::string& reason,
                            std::uint64_t clientSequence,
                            std::vector<std::uint64_t> packetIds) {
  if (packetIds.empty()) return;
  if (packetIds.size() > 64) packetIds.resize(64);

  VideoNackSnapshot snapshot;
  {
    std::lock_guard<std::mutex> lock(gVideoNackMutex);
    snapshot.sequence = gVideoNack.sequence + 1;
    snapshot.clientSequence = clientSequence;
    snapshot.requestedAtMicros = unixMicros();
    snapshot.reason = reason.empty() ? "client" : reason;
    snapshot.packetIds = std::move(packetIds);
    gVideoNack = snapshot;
  }

  std::cerr << "SNU1_NACK_RX sequence=" << snapshot.sequence
            << " clientSequence=" << snapshot.clientSequence
            << " reason=" << snapshot.reason
            << " packetIds=" << snapshot.packetIds.size()
            << " first=" << snapshot.packetIds.front()
            << "\n";
}

void makeProcessDpiAware() {
#ifdef _WIN32
  SetProcessDPIAware();
#endif
}

struct Options {
  std::filesystem::path outputDir = "captures";
  std::filesystem::path outputFile = "captures/capture_h264.mp4";
  std::filesystem::path packetFile;
  std::string tcpConnect;
  std::string udpConnect;
  std::string controlConnect;
  std::string audioUdpConnect;
  std::string sessionToken;
  std::uint32_t frames = 1;
  std::uint32_t intervalMs = 250;
  std::uint32_t adapterIndex = 0;
  std::uint32_t outputIndex = 0;
  std::uint32_t fps = 30;
  std::uint32_t bitrate = 28000000;
  std::uint32_t keyframeIntervalSeconds = 1;
  VideoCodec codec = VideoCodec::H264;
  bool pipe = false;
  bool encode = false;
  bool encodePipe = false;
  bool framesProvided = false;
  bool hardwareEncoder = true;
  bool lowLatencyEncoder = true;
  bool udpPacing = true;
  bool listEncoders = false;
};

struct PipeFrameHeader {
  char magic[4] = {'S', 'N', 'F', '1'};
  std::uint32_t headerSize = sizeof(PipeFrameHeader);
  std::uint32_t width = 0;
  std::uint32_t height = 0;
  std::uint32_t stride = 0;
  std::uint32_t pixelFormat = 1; // 1 = BGRA8
  std::uint64_t timestampMicros = 0;
  std::uint32_t payloadSize = 0;
};

#pragma pack(push, 1)
struct EncodedPacketHeader {
  char magic[4] = {'S', 'N', 'V', '1'};
  std::uint32_t headerSize = sizeof(EncodedPacketHeader);
  std::uint32_t codec = 1; // 1 = H.264
  std::uint32_t packetFormat = 1; // 1 = compressed sample bytes as emitted by Media Foundation
  std::uint32_t width = 0;
  std::uint32_t height = 0;
  std::uint64_t sequence = 0;
  std::uint64_t timestampMicros = 0;
  std::uint32_t durationMicros = 0;
  std::uint32_t flags = 0; // bit 0 = keyframe, bit 1 = hardware encoder
  std::uint32_t payloadSize = 0;
  std::uint64_t hostUnixMicros = 0;
};
#pragma pack(pop)

#ifdef _WIN32
#pragma pack(push, 1)
struct ControlMessageHeader {
  char magic[4] = {'S', 'N', 'I', '1'};
  std::uint32_t headerSize = sizeof(ControlMessageHeader);
  std::uint32_t payloadSize = 0;
};

struct UdpVideoFragmentHeader {
  char magic[4] = {'S', 'N', 'U', '1'};
  std::uint16_t headerSize = sizeof(UdpVideoFragmentHeader);
  std::uint16_t fragmentCount = 0;
  std::uint16_t fragmentIndex = 0;
  std::uint16_t flags = 0; // bit 0 = first, bit 1 = last
  std::uint64_t packetId = 0;
  std::uint32_t packetSize = 0;
  std::uint32_t fragmentOffset = 0;
  std::uint32_t payloadSize = 0;
};

static constexpr std::size_t kUdpAuthTagHexSize = 32;

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
#endif

std::uint32_t readUintArg(int argc, char** argv, int& i, const char* name) {
  if (i + 1 >= argc) {
    throw std::runtime_error(std::string("Missing value for ") + name);
  }
  return static_cast<std::uint32_t>(std::stoul(argv[++i]));
}

Options parseOptions(int argc, char** argv) {
  Options options;
  for (int i = 1; i < argc; ++i) {
    const std::string arg = argv[i];
    if (arg == "--output-dir") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --output-dir");
      options.outputDir = argv[++i];
    } else if (arg == "--output-file") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --output-file");
      options.outputFile = argv[++i];
    } else if (arg == "--packet-file") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --packet-file");
      options.packetFile = argv[++i];
    } else if (arg == "--tcp-connect") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --tcp-connect");
      options.tcpConnect = argv[++i];
    } else if (arg == "--udp-connect") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --udp-connect");
      options.udpConnect = argv[++i];
    } else if (arg == "--control-connect") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --control-connect");
      options.controlConnect = argv[++i];
    } else if (arg == "--audio-udp-connect") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --audio-udp-connect");
      options.audioUdpConnect = argv[++i];
    } else if (arg == "--session-token") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --session-token");
      options.sessionToken = argv[++i];
    } else if (arg == "--frames") {
      options.frames = readUintArg(argc, argv, i, "--frames");
      options.framesProvided = true;
    } else if (arg == "--interval-ms") {
      options.intervalMs = readUintArg(argc, argv, i, "--interval-ms");
    } else if (arg == "--adapter") {
      options.adapterIndex = readUintArg(argc, argv, i, "--adapter");
    } else if (arg == "--output") {
      options.outputIndex = readUintArg(argc, argv, i, "--output");
    } else if (arg == "--fps") {
      options.fps = readUintArg(argc, argv, i, "--fps");
    } else if (arg == "--bitrate") {
      options.bitrate = readUintArg(argc, argv, i, "--bitrate");
    } else if (arg == "--keyframe-interval") {
      options.keyframeIntervalSeconds = std::max<std::uint32_t>(1, readUintArg(argc, argv, i, "--keyframe-interval"));
    } else if (arg == "--low-latency-encoder") {
      options.lowLatencyEncoder = true;
    } else if (arg == "--no-low-latency-encoder") {
      options.lowLatencyEncoder = false;
    } else if (arg == "--udp-pacing") {
      options.udpPacing = true;
    } else if (arg == "--no-udp-pacing") {
      options.udpPacing = false;
    } else if (arg == "--pipe") {
      options.pipe = true;
    } else if (arg == "--encode") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --encode");
      options.codec = parseVideoCodec(argv[++i]);
      options.encode = true;
    } else if (arg == "--encode-pipe") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --encode-pipe");
      options.codec = parseVideoCodec(argv[++i]);
      options.encodePipe = true;
    } else if (arg == "--software-encoder") {
      options.hardwareEncoder = false;
    } else if (arg == "--list-encoders") {
      options.listEncoders = true;
    } else if (arg == "--help" || arg == "-h") {
      std::cout
        << "sanser-native-host --frames 10 --interval-ms 100 --output-dir captures\n"
        << "sanser-native-host --pipe --fps 60\n"
        << "sanser-native-host --list-encoders\n"
        << "sanser-native-host --encode h264 --frames 300 --fps 60 --output-file captures/capture_h264.mp4\n"
        << "sanser-native-host --encode-pipe h264 --fps 60 --packet-file native-captures/capture.snv\n"
        << "sanser-native-host --encode-pipe h264 --fps 60 --tcp-connect 100.x.y.z:7777\n"
        << "  --adapter N      DXGI adapter index, default 0\n"
        << "  --output N       DXGI output/monitor index, default 0\n"
        << "  --pipe           Write BGRA frames to stdout with SNF1 headers\n"
        << "  --fps N          Target FPS in pipe/encode mode, default 30\n"
        << "  --encode CODEC   Encode captured frames with Media Foundation: h264, hevc, av1\n"
        << "  --encode-pipe CODEC Encode H.264 packets to stdout or --packet-file with SNV1 headers\n"
        << "  --bitrate N      Target encode bitrate, default 28000000\n"
        << "  --keyframe-interval N Seconds between keyframes in SNV1 mode, default 1\n"
        << "  SNV1 network mode adapts bitrate and frame pacing from client feedback\n"
        << "  --low-latency-encoder Enable low-latency encoder hints, default on\n"
        << "  --no-low-latency-encoder Disable low-latency encoder hints\n"
        << "  --output-file P  Encoded output file path\n"
        << "  --packet-file P  SNV1 packet output file; stdout is used when omitted\n"
        << "  --tcp-connect H:P Connect to a TCP SNV1 receiver, stream video, receive native input\n"
        << "  --udp-connect H:P Send SNV1 video over UDP with SNU1/SNU2 fragmentation\n"
        << "  --udp-pacing     Pace UDP video fragments by bitrate, default on\n"
        << "  --no-udp-pacing  Disable UDP video fragment pacing\n"
        << "  --control-connect H:P Connect a dedicated TCP native input/stats backchannel\n"
        << "  --audio-udp-connect H:P Send loopback system audio as SNA1/SNA2 UDP float PCM\n"
        << "  --session-token T Enable native session proof and HMAC-authenticated control packets\n"
        << "  --list-encoders  List Media Foundation hardware encoders\n"
        << "  --software-encoder Disable hardware transform request\n";
      std::exit(0);
    } else {
      throw std::runtime_error("Unknown argument: " + arg);
    }
  }
  return options;
}

std::uint64_t nowMicros() {
  const auto now = std::chrono::steady_clock::now().time_since_epoch();
  return static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(now).count());
}

std::uint64_t unixMicros() {
  const auto now = std::chrono::system_clock::now().time_since_epoch();
  return static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(now).count());
}

#ifdef _WIN32
class WinsockRuntime {
public:
  WinsockRuntime() {
    WSADATA data{};
    const int result = WSAStartup(MAKEWORD(2, 2), &data);
    if (result != 0) {
      throw std::runtime_error("WSAStartup failed: " + std::to_string(result));
    }
  }

  ~WinsockRuntime() {
    WSACleanup();
  }

  WinsockRuntime(const WinsockRuntime&) = delete;
  WinsockRuntime& operator=(const WinsockRuntime&) = delete;
};

std::size_t skipJsonWhitespace(const std::string& json, std::size_t offset) {
  while (offset < json.size()) {
    const char ch = json[offset];
    if (ch != ' ' && ch != '\n' && ch != '\r' && ch != '\t') break;
    ++offset;
  }
  return offset;
}

std::size_t findJsonValue(const std::string& json, const char* key) {
  const std::string needle = std::string("\"") + key + "\"";
  const std::size_t keyOffset = json.find(needle);
  if (keyOffset == std::string::npos) return std::string::npos;
  const std::size_t colon = json.find(':', keyOffset + needle.size());
  if (colon == std::string::npos) return std::string::npos;
  return skipJsonWhitespace(json, colon + 1);
}

std::string jsonUnescape(std::string_view value) {
  std::string out;
  out.reserve(value.size());
  for (std::size_t i = 0; i < value.size(); ++i) {
    const char ch = value[i];
    if (ch != '\\' || i + 1 >= value.size()) {
      out.push_back(ch);
      continue;
    }
    const char escaped = value[++i];
    switch (escaped) {
      case 'n': out.push_back('\n'); break;
      case 'r': out.push_back('\r'); break;
      case 't': out.push_back('\t'); break;
      case '\\': out.push_back('\\'); break;
      case '"': out.push_back('"'); break;
      default: out.push_back(escaped); break;
    }
  }
  return out;
}

std::string jsonEscape(std::string_view value) {
  std::ostringstream escaped;
  for (const char ch : value) {
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

struct HmacSha256Provider {
  HmacSha256Provider() {
    NTSTATUS status = BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, BCRYPT_ALG_HANDLE_HMAC_FLAG);
    if (!BCRYPT_SUCCESS(status)) {
      throw std::runtime_error("BCryptOpenAlgorithmProvider(HMAC-SHA256) failed");
    }
    DWORD dataLength = 0;
    status = BCryptGetProperty(algorithm,
                               BCRYPT_OBJECT_LENGTH,
                               reinterpret_cast<PUCHAR>(&objectLength),
                               sizeof(objectLength),
                               &dataLength,
                               0);
    if (!BCRYPT_SUCCESS(status)) {
      BCryptCloseAlgorithmProvider(algorithm, 0);
      algorithm = nullptr;
      throw std::runtime_error("BCryptGetProperty(BCRYPT_OBJECT_LENGTH) failed");
    }
  }

  ~HmacSha256Provider() {
    if (algorithm) BCryptCloseAlgorithmProvider(algorithm, 0);
  }

  BCRYPT_ALG_HANDLE algorithm = nullptr;
  DWORD objectLength = 0;
  std::mutex mutex;
};

HmacSha256Provider& hmacSha256Provider() {
  static HmacSha256Provider provider;
  return provider;
}

std::string hmacSha256Hex(const std::string& key, const std::string& message) {
  HmacSha256Provider& provider = hmacSha256Provider();
  std::lock_guard<std::mutex> providerLock(provider.mutex);
  BCRYPT_HASH_HANDLE hash = nullptr;
  std::vector<unsigned char> hashObject(provider.objectLength);
  std::array<unsigned char, 32> digest{};

  auto cleanup = [&]() {
    if (hash) BCryptDestroyHash(hash);
  };

  NTSTATUS status = BCryptCreateHash(provider.algorithm,
                            &hash,
                            hashObject.data(),
                            static_cast<ULONG>(hashObject.size()),
                            reinterpret_cast<PUCHAR>(const_cast<char*>(key.data())),
                            static_cast<ULONG>(key.size()),
                            0);
  if (!BCRYPT_SUCCESS(status)) {
    cleanup();
    throw std::runtime_error("BCryptCreateHash(HMAC-SHA256) failed");
  }
  status = BCryptHashData(hash,
                          reinterpret_cast<PUCHAR>(const_cast<char*>(message.data())),
                          static_cast<ULONG>(message.size()),
                          0);
  if (!BCRYPT_SUCCESS(status)) {
    cleanup();
    throw std::runtime_error("BCryptHashData(HMAC-SHA256) failed");
  }
  status = BCryptFinishHash(hash, digest.data(), static_cast<ULONG>(digest.size()), 0);
  if (!BCRYPT_SUCCESS(status)) {
    cleanup();
    throw std::runtime_error("BCryptFinishHash(HMAC-SHA256) failed");
  }
  cleanup();
  return bytesToHex(digest.data(), digest.size());
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
      << ",\"payload\":\"" << jsonEscape(payload) << "\""
      << ",\"authMac\":\"" << mac << "\""
      << "}";
  return out.str();
}

std::string udpDatagramMacHex(const std::string& token,
                              const char* media,
                              const std::uint8_t* datagram,
                              std::size_t datagramSize) {
  std::string message = std::string("sanser-udp-v1|") + media + "|";
  message.append(reinterpret_cast<const char*>(datagram), datagramSize);
  return hmacSha256Hex(token, message).substr(0, kUdpAuthTagHexSize);
}

void writeUdpAuthTag(const std::string& token,
                     const char* media,
                     std::vector<std::uint8_t>& datagram,
                     std::size_t tagOffset) {
  if (token.empty() || tagOffset + kUdpAuthTagHexSize > datagram.size()) return;
  std::memset(datagram.data() + tagOffset, 0, kUdpAuthTagHexSize);
  const std::string tag = udpDatagramMacHex(token, media, datagram.data(), datagram.size());
  std::memcpy(datagram.data() + tagOffset, tag.data(), kUdpAuthTagHexSize);
}

template <std::size_t Size>
void writeUdpAuthTag(const std::string& token,
                     const char* media,
                     std::array<std::uint8_t, Size>& datagram,
                     std::size_t datagramSize,
                     std::size_t tagOffset) {
  if (token.empty() || datagramSize > datagram.size() || tagOffset + kUdpAuthTagHexSize > datagramSize) return;
  std::memset(datagram.data() + tagOffset, 0, kUdpAuthTagHexSize);
  const std::string tag = udpDatagramMacHex(token, media, datagram.data(), datagramSize);
  std::memcpy(datagram.data() + tagOffset, tag.data(), kUdpAuthTagHexSize);
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

struct MediaCryptoSnapshot {
  bool enabled = false;
  std::uint64_t epoch = 0;
  std::uint64_t generation = 0;
  std::array<std::uint8_t, 32> videoCryptoKey{};
  std::array<std::uint8_t, 32> audioCryptoKey{};
  std::string videoAuthKey;
  std::string audioAuthKey;
};

class MediaCryptoState {
public:
  explicit MediaCryptoState(std::string sessionToken)
    : sessionToken_(std::move(sessionToken)) {
    if (!sessionToken_.empty()) {
      rotateLocked(0, bootstrapNonce_);
    }
  }

  MediaCryptoSnapshot snapshot() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return snapshot_;
  }

  std::uint64_t rotateForNonce(const std::string& nonce) {
    if (sessionToken_.empty() || nonce.empty()) return 0;
    std::lock_guard<std::mutex> lock(mutex_);
    const std::uint64_t nextEpoch = std::max<std::uint64_t>(snapshot_.epoch + 1, 1);
    rotateLocked(nextEpoch, nonce);
    return snapshot_.epoch;
  }

private:
  void rotateLocked(std::uint64_t epoch, const std::string& nonce) {
    snapshot_.enabled = true;
    snapshot_.epoch = epoch;
    snapshot_.generation += 1;
    snapshot_.videoCryptoKey = deriveMediaCryptoKey(sessionToken_, "video", epoch, nonce);
    snapshot_.audioCryptoKey = deriveMediaCryptoKey(sessionToken_, "audio", epoch, nonce);
    snapshot_.videoAuthKey = deriveMediaAuthKey(sessionToken_, "video", epoch, nonce);
    snapshot_.audioAuthKey = deriveMediaAuthKey(sessionToken_, "audio", epoch, nonce);
    std::cerr << "SNMEDIA_KEY_ROTATE epoch=" << snapshot_.epoch
              << " generation=" << snapshot_.generation
              << " nonce=" << (nonce == bootstrapNonce_ ? "bootstrap" : "control")
              << "\n";
  }

  static constexpr const char* bootstrapNonce_ = "bootstrap";
  std::string sessionToken_;
  mutable std::mutex mutex_;
  MediaCryptoSnapshot snapshot_;
};

std::string jsonStringValue(const std::string& json, const char* key, const std::string& fallback = {}) {
  std::size_t offset = findJsonValue(json, key);
  if (offset == std::string::npos || offset >= json.size() || json[offset] != '"') return fallback;
  ++offset;

  std::string raw;
  while (offset < json.size()) {
    const char ch = json[offset++];
    if (ch == '"') return jsonUnescape(raw);
    if (ch == '\\' && offset < json.size()) {
      raw.push_back(ch);
      raw.push_back(json[offset++]);
    } else {
      raw.push_back(ch);
    }
  }
  return fallback;
}

double jsonDoubleValue(const std::string& json, const char* key, double fallback = 0.0) {
  const std::size_t offset = findJsonValue(json, key);
  if (offset == std::string::npos) return fallback;
  try {
    return std::stod(json.substr(offset));
  } catch (...) {
    return fallback;
  }
}

int jsonIntValue(const std::string& json, const char* key, int fallback = 0) {
  const std::size_t offset = findJsonValue(json, key);
  if (offset == std::string::npos) return fallback;
  try {
    return std::stoi(json.substr(offset));
  } catch (...) {
    return fallback;
  }
}

std::uint64_t jsonUint64Value(const std::string& json, const char* key, std::uint64_t fallback = 0) {
  const std::size_t offset = findJsonValue(json, key);
  if (offset == std::string::npos) return fallback;
  try {
    return static_cast<std::uint64_t>(std::stoull(json.substr(offset)));
  } catch (...) {
    return fallback;
  }
}

std::vector<std::uint64_t> jsonUint64ArrayValue(const std::string& json,
                                                const char* key,
                                                std::size_t maxValues = 64) {
  std::vector<std::uint64_t> values;
  std::size_t offset = findJsonValue(json, key);
  if (offset == std::string::npos || offset >= json.size() || json[offset] != '[') return values;
  ++offset;

  while (offset < json.size() && values.size() < maxValues) {
    offset = skipJsonWhitespace(json, offset);
    if (offset >= json.size() || json[offset] == ']') break;
    if (json[offset] == ',') {
      ++offset;
      continue;
    }
    if (json[offset] < '0' || json[offset] > '9') break;
    std::size_t end = offset;
    while (end < json.size() && json[end] >= '0' && json[end] <= '9') {
      ++end;
    }
    try {
      values.push_back(static_cast<std::uint64_t>(std::stoull(json.substr(offset, end - offset))));
    } catch (...) {
      break;
    }
    offset = end;
  }
  return values;
}

bool jsonBoolValue(const std::string& json, const char* key, bool fallback = false) {
  const std::size_t offset = findJsonValue(json, key);
  if (offset == std::string::npos) return fallback;
  if (json.compare(offset, 4, "true") == 0) return true;
  if (json.compare(offset, 5, "false") == 0) return false;
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

std::vector<std::string> jsonObjectArrayValue(const std::string& json, const char* key) {
  std::vector<std::string> objects;
  std::size_t offset = findJsonValue(json, key);
  if (offset == std::string::npos || offset >= json.size() || json[offset] != '[') return objects;
  ++offset;

  bool inString = false;
  bool escaped = false;
  int depth = 0;
  std::size_t objectStart = std::string::npos;
  for (std::size_t i = offset; i < json.size(); ++i) {
    const char ch = json[i];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (ch == '\\') {
        escaped = true;
      } else if (ch == '"') {
        inString = false;
      }
      continue;
    }

    if (ch == '"') {
      inString = true;
      continue;
    }
    if (ch == '{') {
      if (depth == 0) objectStart = i;
      depth += 1;
      continue;
    }
    if (ch == '}') {
      if (depth > 0) {
        depth -= 1;
        if (depth == 0 && objectStart != std::string::npos) {
          objects.push_back(json.substr(objectStart, i - objectStart + 1));
          objectStart = std::string::npos;
        }
      }
      continue;
    }
    if (ch == ']' && depth == 0) break;
  }
  return objects;
}

std::wstring utf8ToWide(const std::string& value) {
  if (value.empty()) return {};
  const int required = MultiByteToWideChar(CP_UTF8, 0, value.data(), static_cast<int>(value.size()), nullptr, 0);
  if (required <= 0) return {};
  std::wstring wide(static_cast<std::size_t>(required), L'\0');
  MultiByteToWideChar(CP_UTF8, 0, value.data(), static_cast<int>(value.size()), wide.data(), required);
  return wide;
}

POINT normalizedToScreenPoint(double normalizedX, double normalizedY) {
  InputBounds bounds;
  {
    std::lock_guard<std::mutex> lock(gInputBoundsMutex);
    bounds = gInputBounds;
  }
  const int left = bounds.left;
  const int top = bounds.top;
  const int width = std::max(bounds.width, 1);
  const int height = std::max(bounds.height, 1);

  const double x = std::clamp(normalizedX, 0.0, 1.0);
  const double y = std::clamp(normalizedY, 0.0, 1.0);
  const int screenX = left + static_cast<int>(std::lround(x * static_cast<double>(width - 1)));
  const int screenY = top + static_cast<int>(std::lround(y * static_cast<double>(height - 1)));
  return POINT{static_cast<LONG>(screenX), static_cast<LONG>(screenY)};
}

bool cursorNearTarget(const POINT& target, POINT* actualOut = nullptr) {
  POINT actual{};
  if (!GetCursorPos(&actual)) return false;
  if (actualOut) *actualOut = actual;
  return std::abs(actual.x - target.x) <= 2 && std::abs(actual.y - target.y) <= 2;
}

bool movePointerNormalized(double normalizedX, double normalizedY) {
  const POINT target = normalizedToScreenPoint(normalizedX, normalizedY);

  ClipCursor(nullptr);
  if (SetCursorPos(target.x, target.y) != 0 && cursorNearTarget(target)) return true;

  const int virtualLeft = GetSystemMetrics(SM_XVIRTUALSCREEN);
  const int virtualTop = GetSystemMetrics(SM_YVIRTUALSCREEN);
  const int virtualWidth = std::max(GetSystemMetrics(SM_CXVIRTUALSCREEN), 1);
  const int virtualHeight = std::max(GetSystemMetrics(SM_CYVIRTUALSCREEN), 1);
  const LONG absoluteX = static_cast<LONG>(std::lround((target.x - virtualLeft) * 65535.0 / std::max(virtualWidth - 1, 1)));
  const LONG absoluteY = static_cast<LONG>(std::lround((target.y - virtualTop) * 65535.0 / std::max(virtualHeight - 1, 1)));

  INPUT input{};
  input.type = INPUT_MOUSE;
  input.mi.dx = absoluteX;
  input.mi.dy = absoluteY;
  input.mi.dwFlags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
  if (SendInput(1, &input, sizeof(input)) == 1 && cursorNearTarget(target)) return true;

  mouse_event(input.mi.dwFlags,
              static_cast<DWORD>(absoluteX),
              static_cast<DWORD>(absoluteY),
              0,
              0);
  return cursorNearTarget(target);
}

bool movePointerRelative(double deltaX, double deltaY) {
  const LONG relativeX = static_cast<LONG>(std::lround(deltaX));
  const LONG relativeY = static_cast<LONG>(std::lround(deltaY));
  if (relativeX == 0 && relativeY == 0) return true;

  INPUT input{};
  input.type = INPUT_MOUSE;
  input.mi.dx = relativeX;
  input.mi.dy = relativeY;
  input.mi.dwFlags = MOUSEEVENTF_MOVE;
  if (SendInput(1, &input, sizeof(input)) == 1) return true;

  mouse_event(MOUSEEVENTF_MOVE,
              static_cast<DWORD>(relativeX),
              static_cast<DWORD>(relativeY),
              0,
              0);
  return GetLastError() == ERROR_SUCCESS;
}

DWORD mouseButtonFlag(int button, bool down) {
  if (button == 1) return down ? MOUSEEVENTF_MIDDLEDOWN : MOUSEEVENTF_MIDDLEUP;
  if (button == 2) return down ? MOUSEEVENTF_RIGHTDOWN : MOUSEEVENTF_RIGHTUP;
  return down ? MOUSEEVENTF_LEFTDOWN : MOUSEEVENTF_LEFTUP;
}

DWORD normalizeMacMouseButton(int button, bool down) {
  if (button == 1) return down ? MOUSEEVENTF_RIGHTDOWN : MOUSEEVENTF_RIGHTUP;
  if (button == 2) return down ? MOUSEEVENTF_MIDDLEDOWN : MOUSEEVENTF_MIDDLEUP;
  return down ? MOUSEEVENTF_LEFTDOWN : MOUSEEVENTF_LEFTUP;
}

bool sendMouseButton(int button, bool down) {
  INPUT input{};
  input.type = INPUT_MOUSE;
  input.mi.dwFlags = normalizeMacMouseButton(button, down);
  if (SendInput(1, &input, sizeof(input)) == 1) return true;
  mouse_event(input.mi.dwFlags, 0, 0, 0, 0);
  return GetLastError() == ERROR_SUCCESS;
}

bool sendWheel(double deltaY) {
  if (std::abs(deltaY) < 0.01) return true;
  const double wheelSteps = std::clamp(-deltaY / 8.0, -3.0, 3.0);
  const double clamped = std::clamp(wheelSteps * static_cast<double>(WHEEL_DELTA), -360.0, 360.0);
  INPUT input{};
  input.type = INPUT_MOUSE;
  input.mi.dwFlags = MOUSEEVENTF_WHEEL;
  input.mi.mouseData = static_cast<DWORD>(static_cast<LONG>(std::lround(clamped)));
  if (SendInput(1, &input, sizeof(input)) == 1) return true;
  mouse_event(MOUSEEVENTF_WHEEL, 0, 0, static_cast<DWORD>(static_cast<LONG>(std::lround(clamped))), 0);
  return GetLastError() == ERROR_SUCCESS;
}

WORD macKeyCodeToVirtualKey(int keyCode, const std::string& keyText) {
  switch (keyCode) {
    case 0: return 'A';
    case 1: return 'S';
    case 2: return 'D';
    case 3: return 'F';
    case 4: return 'H';
    case 5: return 'G';
    case 6: return 'Z';
    case 7: return 'X';
    case 8: return 'C';
    case 9: return 'V';
    case 11: return 'B';
    case 12: return 'Q';
    case 13: return 'W';
    case 14: return 'E';
    case 15: return 'R';
    case 16: return 'Y';
    case 17: return 'T';
    case 18: return '1';
    case 19: return '2';
    case 20: return '3';
    case 21: return '4';
    case 22: return '6';
    case 23: return '5';
    case 24: return VK_OEM_PLUS;
    case 25: return '9';
    case 26: return '7';
    case 27: return VK_OEM_MINUS;
    case 28: return '8';
    case 29: return '0';
    case 30: return VK_OEM_6;
    case 31: return 'O';
    case 32: return 'U';
    case 33: return VK_OEM_4;
    case 34: return 'I';
    case 35: return 'P';
    case 36: return VK_RETURN;
    case 37: return 'L';
    case 38: return 'J';
    case 39: return VK_OEM_7;
    case 40: return 'K';
    case 41: return VK_OEM_1;
    case 42: return VK_OEM_5;
    case 43: return VK_OEM_COMMA;
    case 44: return VK_OEM_2;
    case 45: return 'N';
    case 46: return 'M';
    case 47: return VK_OEM_PERIOD;
    case 48: return VK_TAB;
    case 49: return VK_SPACE;
    case 50: return VK_OEM_3;
    case 51: return VK_BACK;
    case 53: return VK_ESCAPE;
    case 54:
    case 55: return VK_CONTROL;
    case 56:
    case 60: return VK_SHIFT;
    case 58:
    case 61: return VK_MENU;
    case 59:
    case 62: return VK_CONTROL;
    case 63: return 0;
    case 115: return VK_HOME;
    case 116: return VK_PRIOR;
    case 117: return VK_DELETE;
    case 119: return VK_END;
    case 121: return VK_NEXT;
    case 123: return VK_LEFT;
    case 124: return VK_RIGHT;
    case 125: return VK_DOWN;
    case 126: return VK_UP;
    default:
      break;
  }

  if (!keyText.empty()) {
    const std::wstring wide = utf8ToWide(keyText);
    if (!wide.empty()) {
      const SHORT mapped = VkKeyScanW(wide[0]);
      if (mapped != -1) return static_cast<WORD>(mapped & 0xff);
    }
  }
  return 0;
}

bool sendVirtualKey(WORD virtualKey, bool down) {
  if (virtualKey == 0) return false;
  INPUT input{};
  input.type = INPUT_KEYBOARD;
  input.ki.wVk = virtualKey;
  input.ki.dwFlags = down ? 0 : KEYEVENTF_KEYUP;
  if (SendInput(1, &input, sizeof(input)) == 1) return true;
  keybd_event(static_cast<BYTE>(virtualKey), 0, down ? 0 : KEYEVENTF_KEYUP, 0);
  return GetLastError() == ERROR_SUCCESS;
}

void sendShortcut(WORD virtualKey) {
  sendVirtualKey(VK_CONTROL, true);
  sendVirtualKey(virtualKey, true);
  sendVirtualKey(virtualKey, false);
  sendVirtualKey(VK_CONTROL, false);
}

void setClipboardText(const std::string& text) {
  const std::wstring wide = utf8ToWide(text);
  const SIZE_T bytes = (wide.size() + 1) * sizeof(wchar_t);
  HGLOBAL memory = GlobalAlloc(GMEM_MOVEABLE, bytes);
  if (!memory) return;

  void* target = GlobalLock(memory);
  if (!target) {
    GlobalFree(memory);
    return;
  }
  std::memcpy(target, wide.c_str(), bytes);
  GlobalUnlock(memory);

  if (!OpenClipboard(nullptr)) {
    GlobalFree(memory);
    return;
  }
  EmptyClipboard();
  if (!SetClipboardData(CF_UNICODETEXT, memory)) {
    GlobalFree(memory);
  }
  CloseClipboard();
}

void handleModifierState(int keyCode, int modifiers) {
  constexpr int shiftMask = 1 << 17;
  constexpr int controlMask = 1 << 18;
  constexpr int optionMask = 1 << 19;
  constexpr int commandMask = 1 << 20;

  WORD virtualKey = macKeyCodeToVirtualKey(keyCode, {});
  bool down = false;
  if (keyCode == 56 || keyCode == 60) down = (modifiers & shiftMask) != 0;
  if (keyCode == 58 || keyCode == 61) down = (modifiers & optionMask) != 0;
  if (keyCode == 59 || keyCode == 62) down = (modifiers & controlMask) != 0;
  if (keyCode == 54 || keyCode == 55) down = (modifiers & commandMask) != 0;
  sendVirtualKey(virtualKey, down);
}

void handleStreamStatsPayload(const std::string& json) {
  StreamFeedbackSnapshot feedback;
  feedback.source = jsonStringValue(json, "source", "network");
  feedback.packets = static_cast<std::uint64_t>(std::max(0, jsonIntValue(json, "packets")));
  feedback.dropped = static_cast<std::uint64_t>(std::max(0, jsonIntValue(json, "dropped")));
  feedback.totalDropped = static_cast<std::uint64_t>(std::max(0, jsonIntValue(json, "totalDropped")));
  feedback.jitterMs = jsonDoubleValue(json, "jitterMs");
  feedback.avgAgeMs = jsonDoubleValue(json, "avgAgeMs", -1.0);
  feedback.maxAgeMs = jsonDoubleValue(json, "maxAgeMs", -1.0);
  feedback.rendered = static_cast<std::uint64_t>(std::max(0, jsonIntValue(json, "rendered")));
  feedback.renderQueueDepth = static_cast<std::uint64_t>(std::max(0, jsonIntValue(json, "queueDepth")));
  feedback.renderDroppedQueue = static_cast<std::uint64_t>(std::max(0, jsonIntValue(json, "droppedQueue")));
  feedback.renderDroppedLate = static_cast<std::uint64_t>(std::max(0, jsonIntValue(json, "droppedLate")));
  feedback.renderHeld = static_cast<std::uint64_t>(std::max(0, jsonIntValue(json, "held")));
  feedback.renderPacingReset = static_cast<std::uint64_t>(std::max(0, jsonIntValue(json, "pacingReset")));
  feedback.renderTargetDelayMs = jsonDoubleValue(json, "targetDelayMs", -1.0);
  feedback.renderAdaptiveDelayMs = jsonDoubleValue(json, "adaptiveDelayMs", -1.0);
  feedback.renderAdaptiveUp = jsonUint64Value(json, "adaptiveUp");
  feedback.renderAdaptiveDown = jsonUint64Value(json, "adaptiveDown");
  feedback.renderAvgAgeMs = jsonDoubleValue(json, "avgRenderAgeMs", -1.0);
  feedback.renderMaxAgeMs = jsonDoubleValue(json, "maxRenderAgeMs", -1.0);
  feedback.renderAvgPresentLateMs = jsonDoubleValue(json, "avgPresentLateMs", -1.0);
  feedback.renderMaxPresentLateMs = jsonDoubleValue(json, "maxPresentLateMs", -1.0);

  const bool hasNetworkSample = feedback.packets > 0 || feedback.dropped > 0 || feedback.totalDropped > 0;
  const bool hasRenderSample = feedback.source == "render" || feedback.rendered > 0 ||
                               feedback.renderDroppedQueue > 0 || feedback.renderDroppedLate > 0;

  const bool networkSevere = feedback.dropped > 0 || feedback.jitterMs > 35.0 ||
                             feedback.avgAgeMs > 180.0 || feedback.maxAgeMs > 260.0;
  const bool networkCongested = feedback.jitterMs > 16.0 ||
                                feedback.avgAgeMs > 90.0 || feedback.maxAgeMs > 150.0;
  const bool networkClear = hasNetworkSample &&
                            feedback.dropped == 0 && feedback.jitterMs < 8.0 &&
                            (feedback.avgAgeMs < 0.0 || feedback.avgAgeMs < 70.0) &&
                            (feedback.maxAgeMs < 0.0 || feedback.maxAgeMs < 120.0);

  const bool renderSevere = hasRenderSample &&
                            (feedback.renderDroppedQueue > 0 ||
                             feedback.renderDroppedLate > 0 ||
                             feedback.renderQueueDepth >= 5 ||
                             feedback.renderPacingReset > 2 ||
                             feedback.renderAdaptiveDelayMs > 20.0 ||
                             feedback.renderAvgPresentLateMs > 18.0 ||
                             feedback.renderMaxPresentLateMs > 35.0 ||
                             feedback.renderMaxAgeMs > 100.0);
  const bool renderCongested = hasRenderSample &&
                               (feedback.renderQueueDepth >= 3 ||
                                feedback.renderPacingReset > 0 ||
                                feedback.renderAdaptiveDelayMs > 10.0 ||
                                feedback.renderAvgPresentLateMs > 10.0 ||
                                feedback.renderMaxPresentLateMs > 22.0 ||
                                feedback.renderMaxAgeMs > 70.0);
  const bool renderClear = hasRenderSample &&
                           feedback.rendered > 0 &&
                           feedback.renderDroppedQueue == 0 &&
                           feedback.renderDroppedLate == 0 &&
                           feedback.renderQueueDepth <= 2 &&
                           feedback.renderPacingReset == 0 &&
                           (feedback.renderAdaptiveDelayMs < 0.0 || feedback.renderAdaptiveDelayMs < 8.0) &&
                           (feedback.renderAvgPresentLateMs < 0.0 || feedback.renderAvgPresentLateMs < 8.0) &&
                           (feedback.renderMaxPresentLateMs < 0.0 || feedback.renderMaxPresentLateMs < 16.0);

  if (networkSevere || renderSevere) {
    feedback.pressure = 2;
  } else if (networkCongested || renderCongested) {
    feedback.pressure = 1;
  } else if ((networkClear && (!hasRenderSample || renderClear)) ||
             (!hasNetworkSample && renderClear)) {
    feedback.pressure = -1;
  }

  {
    std::lock_guard<std::mutex> lock(gStreamFeedbackMutex);
    feedback.sequence = gStreamFeedback.sequence + 1;
    gStreamFeedback = feedback;
  }

  std::cerr << "SNFEEDBACK source=" << feedback.source
            << " pressure=" << pressureLabel(feedback.pressure)
            << " packets=" << feedback.packets
            << " dropped=" << feedback.dropped
            << " totalDropped=" << feedback.totalDropped
            << " jitterMs=" << feedback.jitterMs
            << " avgAgeMs=" << feedback.avgAgeMs
            << " maxAgeMs=" << feedback.maxAgeMs;
  if (hasRenderSample) {
    std::cerr << " rendered=" << feedback.rendered
              << " renderQueue=" << feedback.renderQueueDepth
              << " renderDropQ=" << feedback.renderDroppedQueue
              << " renderDropLate=" << feedback.renderDroppedLate
              << " renderReset=" << feedback.renderPacingReset
              << " renderAdaptiveDelayMs=" << feedback.renderAdaptiveDelayMs
              << " renderAdaptiveUp=" << feedback.renderAdaptiveUp
              << " renderAdaptiveDown=" << feedback.renderAdaptiveDown
              << " renderAvgLateMs=" << feedback.renderAvgPresentLateMs
              << " renderMaxLateMs=" << feedback.renderMaxPresentLateMs
              << " renderMaxAgeMs=" << feedback.renderMaxAgeMs;
  }
  std::cerr << "\n";
}

void handleUdpRepairStatsPayload(const std::string& json) {
  UdpRepairFeedbackSnapshot feedback;
  feedback.clientSequence = jsonUint64Value(json, "sequence");
  feedback.datagrams = jsonUint64Value(json, "datagrams");
  feedback.fragments = jsonUint64Value(json, "fragments");
  feedback.retransmitFragments = jsonUint64Value(json, "retransmitFragments");
  feedback.completed = jsonUint64Value(json, "completed");
  feedback.retransmitCompleted = jsonUint64Value(json, "retransmitCompleted");
  feedback.droppedAssemblies = jsonUint64Value(json, "droppedAssemblies");
  feedback.malformed = jsonUint64Value(json, "malformed");
  feedback.authRejected = jsonUint64Value(json, "authRejected");
  feedback.replayRejected = jsonUint64Value(json, "replayRejected");
  feedback.peerRejected = jsonUint64Value(json, "peerRejected");
  feedback.rekeyGrace = jsonUint64Value(json, "rekeyGrace");
  feedback.rekeyGraceCompleted = jsonUint64Value(json, "rekeyGraceCompleted");
  feedback.epochResetAssemblies = jsonUint64Value(json, "epochResetAssemblies");
  feedback.duplicateFragments = jsonUint64Value(json, "duplicateFragments");
  feedback.nackPackets = jsonUint64Value(json, "nackPackets");
  feedback.nackPending = jsonUint64Value(json, "nackPending");
  feedback.nackSent = jsonUint64Value(json, "nackSent");
  feedback.nackRecovered = jsonUint64Value(json, "nackRecovered");
  feedback.nackTimedOut = jsonUint64Value(json, "nackTimedOut");
  feedback.jitterPending = jsonUint64Value(json, "jitterPending");
  feedback.jitterReleased = jsonUint64Value(json, "jitterReleased");
  feedback.jitterHeld = jsonUint64Value(json, "jitterHeld");
  feedback.jitterSkipped = jsonUint64Value(json, "jitterSkipped");
  feedback.jitterLate = jsonUint64Value(json, "jitterLate");
  feedback.jitterDup = jsonUint64Value(json, "jitterDup");
  feedback.jitterKeyframeReset = jsonUint64Value(json, "jitterKeyframeReset");
  feedback.jitterGenerationReset = jsonUint64Value(json, "jitterGenerationReset");
  feedback.jitterEpochStaleDrop = jsonUint64Value(json, "jitterEpochStaleDrop");
  feedback.jitterEpochGraceRelease = jsonUint64Value(json, "jitterEpochGraceRelease");
  feedback.decoded = jsonUint64Value(json, "decoded");
  feedback.decodeErrors = jsonUint64Value(json, "decodeErrors");
  feedback.videoSequence = jsonUint64Value(json, "videoSequence");

  const bool severe = feedback.nackTimedOut > 0 ||
                      feedback.jitterSkipped > 0 ||
                      feedback.malformed > 0 ||
                      feedback.authRejected > 0 ||
                      feedback.replayRejected > 0 ||
                      feedback.jitterKeyframeReset > 0 ||
                      feedback.nackPending >= 8;
  const bool congested = feedback.nackSent >= 3 ||
                         feedback.nackPending > 0 ||
                         feedback.retransmitCompleted > 0 ||
                         feedback.retransmitFragments > 0 ||
                         feedback.jitterLate > 0 ||
                         feedback.jitterHeld >= 30;
  const bool clear = feedback.datagrams > 0 &&
                     feedback.nackSent == 0 &&
                     feedback.nackPending == 0 &&
                     feedback.nackTimedOut == 0 &&
                     feedback.jitterSkipped == 0 &&
                     feedback.malformed == 0 &&
                     feedback.authRejected == 0 &&
                     feedback.replayRejected == 0;
  if (severe) {
    feedback.pressure = 2;
  } else if (congested) {
    feedback.pressure = 1;
  } else if (clear) {
    feedback.pressure = -1;
  }

  {
    std::lock_guard<std::mutex> lock(gUdpRepairFeedbackMutex);
    feedback.sequence = gUdpRepairFeedback.sequence + 1;
    gUdpRepairFeedback = feedback;
  }

  std::cerr << "SNUDP_REPAIR pressure=" << pressureLabel(feedback.pressure)
            << " clientSequence=" << feedback.clientSequence
            << " nackSent=" << feedback.nackSent
            << " nackRecovered=" << feedback.nackRecovered
            << " nackTimedOut=" << feedback.nackTimedOut
            << " nackPending=" << feedback.nackPending
            << " retransmitCompleted=" << feedback.retransmitCompleted
            << " jitterSkipped=" << feedback.jitterSkipped
            << " jitterHeld=" << feedback.jitterHeld
            << " jitterGenerationReset=" << feedback.jitterGenerationReset
            << " malformed=" << feedback.malformed
            << " authRejected=" << feedback.authRejected
            << " replayRejected=" << feedback.replayRejected
            << " peerRejected=" << feedback.peerRejected
            << " rekeyGrace=" << feedback.rekeyGrace
            << " rekeyGraceCompleted=" << feedback.rekeyGraceCompleted
            << " epochResetAssemblies=" << feedback.epochResetAssemblies
            << " jitterEpochStaleDrop=" << feedback.jitterEpochStaleDrop
            << " jitterEpochGraceRelease=" << feedback.jitterEpochGraceRelease
            << "\n";
}

int handleControlPayload(const std::string& json) {
  const std::string type = jsonStringValue(json, "type");
  if (type == "input-batch") {
    const auto events = jsonObjectArrayValue(json, "events");
    int applied = 0;
    for (const auto& eventJson : events) {
      applied += handleControlPayload(eventJson);
    }
    return applied;
  } else if (type == "pointer-move") {
    const double x = jsonDoubleValue(json, "x");
    const double y = jsonDoubleValue(json, "y");
    const double dx = jsonDoubleValue(json, "dx");
    const double dy = jsonDoubleValue(json, "dy");
    const bool relative = jsonBoolValue(json, "relative");
    const bool moved = relative ? movePointerRelative(dx, dy) : movePointerNormalized(x, y);
    static auto lastMoveLog = std::chrono::steady_clock::time_point{};
    const auto now = std::chrono::steady_clock::now();
    if (now - lastMoveLog > std::chrono::milliseconds(250)) {
      lastMoveLog = now;
      const POINT target = normalizedToScreenPoint(x, y);
      POINT cursor{};
      GetCursorPos(&cursor);
      std::cerr << "SNINPUT_APPLIED pointer-move"
                << " mode=" << (relative ? "relative" : "absolute")
                << " x=" << x
                << " y=" << y
                << " dx=" << dx
                << " dy=" << dy
                << " moved=" << (moved ? "yes" : "no")
                << " target=" << target.x << "," << target.y
                << " cursor=" << cursor.x << "," << cursor.y
                << "\n";
    }
    return moved ? 1 : 0;
  } else if (type == "pointer-down" || type == "pointer-up") {
    const double x = jsonDoubleValue(json, "x");
    const double y = jsonDoubleValue(json, "y");
    const bool relative = jsonBoolValue(json, "relative");
    const bool moved = relative ? true : movePointerNormalized(x, y);
    const bool clicked = sendMouseButton(jsonIntValue(json, "button"), type == "pointer-down");
    const POINT target = normalizedToScreenPoint(x, y);
    POINT cursor{};
    GetCursorPos(&cursor);
    std::cerr << "SNINPUT_APPLIED " << type
              << " mode=" << (relative ? "relative" : "absolute")
              << " button=" << jsonIntValue(json, "button")
              << " x=" << x
              << " y=" << y
              << " moved=" << (moved ? "yes" : "no")
              << " sent=" << (clicked ? "yes" : "no")
              << " target=" << target.x << "," << target.y
              << " cursor=" << cursor.x << "," << cursor.y
              << "\n";
    return clicked ? 1 : 0;
  } else if (type == "wheel") {
    const bool relative = jsonBoolValue(json, "relative");
    const bool moved = relative ? true : movePointerNormalized(jsonDoubleValue(json, "x"), jsonDoubleValue(json, "y"));
    const bool wheeled = sendWheel(jsonDoubleValue(json, "dy"));
    std::cerr << "SNINPUT_APPLIED wheel"
              << " mode=" << (relative ? "relative" : "absolute")
              << " dy=" << jsonDoubleValue(json, "dy")
              << " moved=" << (moved ? "yes" : "no")
              << " sent=" << (wheeled ? "yes" : "no")
              << "\n";
    return wheeled ? 1 : 0;
  } else if (type == "key-down" || type == "key-up") {
    const WORD virtualKey = macKeyCodeToVirtualKey(jsonIntValue(json, "keyCode", -1),
                                                   jsonStringValue(json, "key"));
    const bool sent = sendVirtualKey(virtualKey, type == "key-down");
    std::cerr << "SNINPUT_APPLIED " << type
              << " vk=" << virtualKey
              << " sent=" << (sent ? "yes" : "no")
              << "\n";
    return sent ? 1 : 0;
  } else if (type == "modifiers") {
    handleModifierState(jsonIntValue(json, "keyCode", -1), jsonIntValue(json, "modifiers", 0));
    return 1;
  } else if (type == "clipboard") {
    setClipboardText(jsonStringValue(json, "text"));
    return 1;
  } else if (type == "copy") {
    sendShortcut('C');
    return 1;
  } else if (type == "cut") {
    sendShortcut('X');
    return 1;
  } else if (type == "paste") {
    sendShortcut('V');
    return 1;
  } else if (type == "select-all") {
    sendShortcut('A');
    return 1;
  } else if (type == "stream-stats") {
    handleStreamStatsPayload(json);
    return 0;
  } else if (type == "udp-repair-stats") {
    handleUdpRepairStatsPayload(json);
    return 0;
  } else if (type == "keyframe-request") {
    requestKeyframe(jsonStringValue(json, "reason", "client"),
                    jsonUint64Value(json, "sequence"),
                    jsonUint64Value(json, "videoSequence"),
                    jsonUint64Value(json, "dropped"),
                    jsonUint64Value(json, "decodeErrors"));
    return 0;
  } else if (type == "video-nack") {
    requestVideoRetransmit(jsonStringValue(json, "reason", "client"),
                           jsonUint64Value(json, "sequence"),
                           jsonUint64ArrayValue(json, "packetIds"));
    return 0;
  }
  return 0;
}

class TcpClient {
public:
  explicit TcpClient(const std::string& endpoint,
                     const std::string& controlEndpoint = {},
                     const std::string& sessionToken = {},
                     std::shared_ptr<MediaCryptoState> mediaCrypto = {})
    : sessionToken_(sessionToken),
      controlEndpoint_(controlEndpoint),
      mediaCrypto_(std::move(mediaCrypto)),
      sessionVerified_(sessionToken.empty()) {
    if (!endpoint.empty()) {
      socket_ = connectEndpoint(endpoint, "--tcp-connect");
      configureSocket(socket_);
    }
    if (!controlEndpoint.empty()) {
      controlSocket_ = connectEndpoint(controlEndpoint, "--control-connect");
      configureSocket(controlSocket_);
      std::cerr << "SNINPUT dedicated control connecting " << controlEndpoint << "\n";
    }
    if (socket_ == INVALID_SOCKET && controlSocket_ == INVALID_SOCKET) {
      throw std::runtime_error("TcpClient needs --tcp-connect or --control-connect.");
    }
  }

  ~TcpClient() {
    running_ = false;
    if (socket_ != INVALID_SOCKET) {
      shutdown(socket_, SD_BOTH);
    }
    if (controlSocket_ != INVALID_SOCKET) {
      shutdown(controlSocket_, SD_BOTH);
    }
    if (controlThread_.joinable()) {
      controlThread_.join();
    }
    if (controlSocket_ != INVALID_SOCKET) {
      closesocket(controlSocket_);
    }
    if (socket_ != INVALID_SOCKET) {
      closesocket(socket_);
    }
  }

  TcpClient(const TcpClient&) = delete;
  TcpClient& operator=(const TcpClient&) = delete;

  void sendAll(const void* data, std::size_t size) {
    if (socket_ == INVALID_SOCKET) {
      throw std::runtime_error("TCP video socket is not connected.");
    }
    const char* cursor = static_cast<const char*>(data);
    std::size_t remaining = size;
    while (remaining > 0) {
      const int chunk = remaining > static_cast<std::size_t>(std::numeric_limits<int>::max())
        ? std::numeric_limits<int>::max()
        : static_cast<int>(remaining);
      const int sent = send(socket_, cursor, chunk, 0);
      if (sent == SOCKET_ERROR || sent == 0) {
        throw std::runtime_error("TCP send failed, WSA error " + std::to_string(WSAGetLastError()));
      }
      cursor += sent;
      remaining -= static_cast<std::size_t>(sent);
    }
  }

  void startControlReceiver() {
    if (controlThread_.joinable()) return;
    if (controlReceiveSocket() == INVALID_SOCKET) return;
    running_ = true;
    controlThread_ = std::thread([this]() { controlLoop(); });
  }

private:
  static SOCKET connectEndpoint(const std::string& endpoint, const char* optionName) {
    const auto separator = endpoint.rfind(':');
    if (separator == std::string::npos || separator == 0 || separator == endpoint.size() - 1) {
      throw std::runtime_error(std::string(optionName) + " must be HOST:PORT");
    }
    const std::string host = endpoint.substr(0, separator);
    const std::string port = endpoint.substr(separator + 1);

    addrinfo hints{};
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;

    addrinfo* results = nullptr;
    const int lookup = getaddrinfo(host.c_str(), port.c_str(), &hints, &results);
    if (lookup != 0) {
      throw std::runtime_error("getaddrinfo failed for " + endpoint + ": " + std::to_string(lookup));
    }

    SOCKET connected = INVALID_SOCKET;
    for (addrinfo* item = results; item; item = item->ai_next) {
      SOCKET candidate = socket(item->ai_family, item->ai_socktype, item->ai_protocol);
      if (candidate == INVALID_SOCKET) continue;

      if (connect(candidate, item->ai_addr, static_cast<int>(item->ai_addrlen)) == 0) {
        connected = candidate;
        break;
      }

      closesocket(candidate);
    }
    freeaddrinfo(results);

    if (connected == INVALID_SOCKET) {
      throw std::runtime_error(std::string("Could not connect ") + optionName + " to " + endpoint +
                               ", WSA error " + std::to_string(WSAGetLastError()));
    }
    return connected;
  }

  static void configureSocket(SOCKET socket) {
    int noDelay = 1;
    setsockopt(socket, IPPROTO_TCP, TCP_NODELAY, reinterpret_cast<const char*>(&noDelay), sizeof(noDelay));
  }

  SOCKET controlReceiveSocket() const {
    return controlSocket_ != INVALID_SOCKET ? controlSocket_ : socket_;
  }

  enum class RecvStatus {
    Complete,
    Timeout,
    Closed,
    Error
  };

  RecvStatus recvAll(void* data, std::size_t size) {
    const SOCKET receiveSocket = controlReceiveSocket();
    char* cursor = static_cast<char*>(data);
    std::size_t remaining = size;
    while (remaining > 0 && running_) {
      fd_set readSet;
      FD_ZERO(&readSet);
      FD_SET(receiveSocket, &readSet);
      timeval timeout{};
      timeout.tv_sec = 1;
      timeout.tv_usec = 0;
      const int ready = select(static_cast<int>(receiveSocket + 1), &readSet, nullptr, nullptr, &timeout);
      if (!running_) return RecvStatus::Closed;
      if (ready == 0) return remaining == size ? RecvStatus::Timeout : RecvStatus::Error;
      if (ready == SOCKET_ERROR) {
        std::cerr << "SNINPUT select failed, WSA error " << WSAGetLastError() << "\n";
        return RecvStatus::Error;
      }

      const int chunk = remaining > static_cast<std::size_t>(std::numeric_limits<int>::max())
        ? std::numeric_limits<int>::max()
        : static_cast<int>(remaining);
      const int received = recv(receiveSocket, cursor, chunk, 0);
      if (received == 0) return RecvStatus::Closed;
      if (received == SOCKET_ERROR) {
        if (!running_) return RecvStatus::Closed;
        std::cerr << "SNINPUT receive failed, WSA error " << WSAGetLastError() << "\n";
        return RecvStatus::Error;
      }
      cursor += received;
      remaining -= static_cast<std::size_t>(received);
    }
    return remaining == 0 ? RecvStatus::Complete : RecvStatus::Closed;
  }

  void sendControlJson(const std::string& json) {
    const SOCKET sendSocket = controlReceiveSocket();
    if (sendSocket == INVALID_SOCKET) return;

    const std::string payload = packetAuthEnabled_
      ? makeSecureControlEnvelope(sessionToken_, "h2c", ++hostSecureSequence_, json)
      : json;

    ControlMessageHeader header{};
    header.payloadSize = static_cast<std::uint32_t>(payload.size());
    sendAllOnSocket(sendSocket, &header, sizeof(header));
    if (!payload.empty()) {
      sendAllOnSocket(sendSocket, payload.data(), payload.size());
    }
  }

  static void sendAllOnSocket(SOCKET socket, const void* data, std::size_t size) {
    const char* cursor = static_cast<const char*>(data);
    std::size_t remaining = size;
    while (remaining > 0) {
      const int chunk = remaining > static_cast<std::size_t>(std::numeric_limits<int>::max())
        ? std::numeric_limits<int>::max()
        : static_cast<int>(remaining);
      const int sent = send(socket, cursor, chunk, 0);
      if (sent == SOCKET_ERROR || sent == 0) {
        throw std::runtime_error("control send failed, WSA error " + std::to_string(WSAGetLastError()));
      }
      cursor += sent;
      remaining -= static_cast<std::size_t>(sent);
    }
  }

  void observeInputSession(const std::string& inputSessionId) {
    if (inputSessionId.empty() || inputSessionId == inputSessionId_) return;
    const std::size_t dropped = recentInputBatches_.size();
    recentInputBatches_.clear();
    highestAppliedInputBatchSequence_ = 0;
    inputSessionId_ = inputSessionId;
    std::cerr << "SNINPUT_SESSION inputSessionId=" << inputSessionId_
              << " resetDedupe=yes"
              << " droppedRecent=" << dropped
              << "\n";
  }

  void resetControlSecurityState() {
    packetAuthEnabled_ = false;
    hostSecureSequence_ = 0;
    lastClientSecureSequence_ = 0;
    sessionVerified_ = sessionToken_.empty();
  }

  void closeDedicatedControlSocket() {
    if (controlSocket_ == INVALID_SOCKET) return;
    shutdown(controlSocket_, SD_BOTH);
    closesocket(controlSocket_);
    controlSocket_ = INVALID_SOCKET;
  }

  bool reconnectDedicatedControlSocket() {
    if (controlEndpoint_.empty()) return false;
    closeDedicatedControlSocket();
    resetControlSecurityState();
    while (running_) {
      try {
        controlSocket_ = connectEndpoint(controlEndpoint_, "--control-connect");
        configureSocket(controlSocket_);
        std::cerr << "SNINPUT dedicated control reconnected " << controlEndpoint_ << "\n";
        return true;
      } catch (const std::exception& error) {
        std::cerr << "SNINPUT dedicated control reconnect failed: " << error.what() << "\n";
        std::this_thread::sleep_for(std::chrono::seconds(1));
      }
    }
    return false;
  }

  bool handleControlTimeout(const char* stage,
                            std::chrono::steady_clock::time_point lastControlRxAt) {
    const auto now = std::chrono::steady_clock::now();
    const auto silence = now - lastControlRxAt;
    if (silence < controlIdleTimeout_) return false;
    const double silenceMs = std::chrono::duration<double, std::milli>(silence).count();
    std::cerr << "SNCONTROL_WATCHDOG side=host state=rx-timeout"
              << " stage=" << stage
              << " silenceMs=" << std::fixed << std::setprecision(0) << silenceMs
              << " action=" << (controlEndpoint_.empty() ? "close-backchannel" : "reconnect-backchannel")
              << "\n";
    return true;
  }

  bool serviceControlSocket() {
    auto lastControlRxAt = std::chrono::steady_clock::now();
    while (running_ && controlReceiveSocket() != INVALID_SOCKET) {
      ControlMessageHeader header{};
      const RecvStatus headerStatus = recvAll(&header, sizeof(header));
      if (headerStatus == RecvStatus::Timeout) {
        if (handleControlTimeout("header", lastControlRxAt)) return false;
        continue;
      }
      if (headerStatus != RecvStatus::Complete) return false;
      lastControlRxAt = std::chrono::steady_clock::now();
      if (std::memcmp(header.magic, "SNI1", 4) != 0 || header.headerSize < sizeof(ControlMessageHeader)) {
        std::cerr << "SNINPUT invalid control header.\n";
        return false;
      }
      if (header.payloadSize > 4 * 1024 * 1024) {
        std::cerr << "SNINPUT payload too large: " << header.payloadSize << "\n";
        return false;
      }
      if (header.headerSize > sizeof(ControlMessageHeader)) {
        std::vector<char> extra(header.headerSize - sizeof(ControlMessageHeader));
        if (recvAll(extra.data(), extra.size()) != RecvStatus::Complete) {
          handleControlTimeout("extra-header", lastControlRxAt);
          return false;
        }
      }

      std::string payload(header.payloadSize, '\0');
      if (!payload.empty() && recvAll(payload.data(), payload.size()) != RecvStatus::Complete) {
        handleControlTimeout("payload", lastControlRxAt);
        return false;
      }
      lastControlRxAt = std::chrono::steady_clock::now();
      try {
        const std::string rawType = jsonStringValue(payload, "type");
        bool securePayload = false;
        if (rawType == "secure-control") {
          if (!packetAuthEnabled_ ||
              !unwrapSecureControlEnvelope(payload, sessionToken_, "c2h", lastClientSecureSequence_, payload)) {
            continue;
          }
          securePayload = true;
        }

        const std::string payloadType = jsonStringValue(payload, "type");
        if (!sessionToken_.empty() && !sessionVerified_ &&
            payloadType != "control-hello" && payloadType != "control-ping") {
          std::cerr << "SNCONTROL_PACKET_AUTH rejected reason=session-not-verified type="
                    << (payloadType.empty() ? "unknown" : payloadType)
                    << "\n";
          continue;
        }
        if (packetAuthEnabled_ && !securePayload &&
            payloadType != "control-hello" && payloadType != "control-ping") {
          std::cerr << "SNCONTROL_PACKET_AUTH rejected reason=missing-envelope type="
                    << (payloadType.empty() ? "unknown" : payloadType)
                    << "\n";
          continue;
        }

        if (payloadType == "control-hello") {
          packetAuthEnabled_ = false;
          hostSecureSequence_ = 0;
          lastClientSecureSequence_ = 0;
          sessionVerified_ = sessionToken_.empty();
          const std::string inputSessionId = jsonStringValue(payload, "inputSessionId");
          observeInputSession(inputSessionId);
          const bool clientPacketAuth = jsonBoolValue(payload, "packetAuth");
          const bool clientMediaCrypto = jsonBoolValue(payload, "mediaCrypto");
          const std::string sessionNonce = jsonStringValue(payload, "sessionNonce");
          const bool canPacketAuth = !sessionToken_.empty() && clientPacketAuth && !sessionNonce.empty();
          const std::uint64_t mediaEpoch = canPacketAuth && clientMediaCrypto && mediaCrypto_
            ? mediaCrypto_->rotateForNonce(sessionNonce)
            : 0;
          const bool mediaCryptoEnabled = mediaEpoch > 0;
          std::ostringstream hello;
          hello << "{\"type\":\"control-hello-ack\""
                << ",\"protocolVersion\":2"
                << ",\"inputSessionId\":\"" << jsonEscape(inputSessionId) << "\""
                << ",\"inputBatch\":true"
                << ",\"inputAck\":true"
                << ",\"videoUdpPacing\":true"
                << ",\"audioUdp\":true"
                << ",\"keyframeRequest\":true"
                << ",\"videoNack\":true"
                << ",\"udpRepairStats\":true"
                << ",\"sessionAuth\":" << (sessionToken_.empty() ? "false" : "true")
                << ",\"packetAuth\":" << (canPacketAuth ? "true" : "false")
                << ",\"mediaCrypto\":" << (mediaCryptoEnabled ? "true" : "false")
                << ",\"mediaEpoch\":" << mediaEpoch;
          if (canPacketAuth) {
            hello << ",\"sessionProof\":\"" << controlSessionProof(sessionToken_, sessionNonce) << "\"";
          }
          hello
                << ",\"hostUnixMicros\":" << unixMicros()
                << "}";
          sendControlJson(hello.str());
          sessionVerified_ = sessionToken_.empty() || canPacketAuth;
          packetAuthEnabled_ = canPacketAuth;
          std::cerr << "SNCONTROL_HELLO_ACK protocol=2 inputBatch=yes inputAck=yes keyframeRequest=yes videoNack=yes udpRepairStats=yes sessionAuth="
                    << (sessionToken_.empty() ? "disabled" : "enabled")
                    << " packetAuth=" << (packetAuthEnabled_ ? "enabled" : "disabled")
                    << " mediaEpoch=" << mediaEpoch
                    << "\n";
          continue;
        }
        if (payloadType == "control-ping") {
          std::ostringstream pong;
          pong << "{\"type\":\"control-pong\""
               << ",\"sentSteadyMicros\":" << jsonUint64Value(payload, "sentSteadyMicros")
               << ",\"hostUnixMicros\":" << unixMicros()
               << "}";
          sendControlJson(pong.str());
          continue;
        }
        if (payloadType == "input-batch") {
          const std::uint64_t sequence = jsonUint64Value(payload, "sequence");
          const std::uint64_t sentSteadyMicros = jsonUint64Value(payload, "sentSteadyMicros");
          const std::string batchInputSessionId = jsonStringValue(payload, "inputSessionId");
          if (inputSessionId_.empty() && !batchInputSessionId.empty()) {
            observeInputSession(batchInputSessionId);
          }
          int applied = 0;
          bool duplicate = false;
          bool stale = false;
          bool sessionMismatch = false;
          const auto duplicateIt = recentInputBatches_.find(sequence);
          if (!inputSessionId_.empty() &&
              !batchInputSessionId.empty() &&
              batchInputSessionId != inputSessionId_) {
            stale = true;
            sessionMismatch = true;
          } else if (sequence != 0 && duplicateIt != recentInputBatches_.end()) {
            applied = duplicateIt->second;
            duplicate = true;
          } else if (sequence != 0 &&
                     highestAppliedInputBatchSequence_ != 0 &&
                     sequence <= highestAppliedInputBatchSequence_) {
            stale = true;
          } else {
            applied = handleControlPayload(payload);
            if (sequence != 0) {
              recentInputBatches_[sequence] = applied;
              highestAppliedInputBatchSequence_ = std::max(highestAppliedInputBatchSequence_, sequence);
              while (recentInputBatches_.size() > maxRecentInputBatches_) {
                recentInputBatches_.erase(recentInputBatches_.begin());
              }
            }
          }
          std::ostringstream ack;
          ack << "{\"type\":\"input-ack\""
              << ",\"sequence\":" << sequence
              << ",\"sentSteadyMicros\":" << sentSteadyMicros
              << ",\"applied\":" << applied
              << ",\"duplicate\":" << (duplicate ? "true" : "false")
              << ",\"stale\":" << (stale ? "true" : "false")
              << ",\"sessionMismatch\":" << (sessionMismatch ? "true" : "false")
              << ",\"hostUnixMicros\":" << unixMicros()
              << "}";
          sendControlJson(ack.str());
          static auto lastBatchLog = std::chrono::steady_clock::time_point{};
          const auto now = std::chrono::steady_clock::now();
          if (now - lastBatchLog > std::chrono::milliseconds(250)) {
            lastBatchLog = now;
            std::cerr << "SNINPUT_BATCH sequence=" << sequence
                      << " applied=" << applied
                      << " duplicate=" << (duplicate ? "yes" : "no")
                      << " stale=" << (stale ? "yes" : "no")
                      << " sessionMismatch=" << (sessionMismatch ? "yes" : "no")
                      << " highestApplied=" << highestAppliedInputBatchSequence_
                      << "\n";
          }
          continue;
        }
        handleControlPayload(payload);
      } catch (const std::exception& error) {
        std::cerr << "SNINPUT control error: " << error.what() << "\n";
      }
    }
    return false;
  }

  void controlLoop() {
    while (running_) {
      if (controlReceiveSocket() == INVALID_SOCKET && !reconnectDedicatedControlSocket()) break;
      std::cerr << "SNINPUT control backchannel enabled"
                << (controlSocket_ != INVALID_SOCKET ? " dedicated=yes" : " dedicated=no")
                << ".\n";
      serviceControlSocket();
      std::cerr << "SNINPUT control backchannel closed.\n";
      if (!running_ || controlEndpoint_.empty()) break;
      closeDedicatedControlSocket();
      std::cerr << "SNINPUT dedicated control reconnecting " << controlEndpoint_ << "\n";
      std::this_thread::sleep_for(std::chrono::milliseconds(250));
    }
  }

  SOCKET socket_ = INVALID_SOCKET;
  SOCKET controlSocket_ = INVALID_SOCKET;
  std::string sessionToken_;
  std::string controlEndpoint_;
  std::string inputSessionId_;
  std::shared_ptr<MediaCryptoState> mediaCrypto_;
  std::map<std::uint64_t, int> recentInputBatches_;
  std::uint64_t highestAppliedInputBatchSequence_ = 0;
  bool sessionVerified_ = true;
  bool packetAuthEnabled_ = false;
  std::uint64_t hostSecureSequence_ = 0;
  std::uint64_t lastClientSecureSequence_ = 0;
  std::atomic<bool> running_{false};
  std::thread controlThread_;
  static constexpr std::size_t maxRecentInputBatches_ = 256;
  static constexpr auto controlIdleTimeout_ = std::chrono::seconds(6);
};

class UdpVideoClient {
public:
  struct RetransmitCacheStats {
    std::uint64_t resets = 0;
    std::uint64_t droppedPackets = 0;
  };

  explicit UdpVideoClient(const std::string& endpoint,
                          std::uint32_t bitrate,
                          bool pacingEnabled,
                          std::shared_ptr<MediaCryptoState> mediaCrypto = {})
    : bitrate_(std::max<std::uint32_t>(bitrate, 1000000)),
      pacingEnabled_(pacingEnabled),
      mediaCrypto_(std::move(mediaCrypto)),
      nextSendAt_(std::chrono::steady_clock::now()) {
    const auto separator = endpoint.rfind(':');
    if (separator == std::string::npos || separator == 0 || separator == endpoint.size() - 1) {
      throw std::runtime_error("--udp-connect must be HOST:PORT");
    }
    const std::string host = endpoint.substr(0, separator);
    const std::string port = endpoint.substr(separator + 1);

    addrinfo hints{};
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_DGRAM;
    hints.ai_protocol = IPPROTO_UDP;

    addrinfo* results = nullptr;
    const int lookup = getaddrinfo(host.c_str(), port.c_str(), &hints, &results);
    if (lookup != 0) {
      throw std::runtime_error("getaddrinfo failed for " + endpoint + ": " + std::to_string(lookup));
    }

    for (addrinfo* item = results; item; item = item->ai_next) {
      SOCKET candidate = socket(item->ai_family, item->ai_socktype, item->ai_protocol);
      if (candidate == INVALID_SOCKET) continue;

      if (connect(candidate, item->ai_addr, static_cast<int>(item->ai_addrlen)) == 0) {
        socket_ = candidate;
        break;
      }

      closesocket(candidate);
    }
    freeaddrinfo(results);

    if (socket_ == INVALID_SOCKET) {
      throw std::runtime_error("Could not connect --udp-connect to " + endpoint +
                               ", WSA error " + std::to_string(WSAGetLastError()));
    }

    int sendBufferBytes = 4 * 1024 * 1024;
    setsockopt(socket_, SOL_SOCKET, SO_SNDBUF, reinterpret_cast<const char*>(&sendBufferBytes), sizeof(sendBufferBytes));
    const MediaCryptoSnapshot crypto = mediaCrypto_ ? mediaCrypto_->snapshot() : MediaCryptoSnapshot{};
    std::cerr << "SNU1 UDP video connected " << endpoint
              << " pacing=" << (pacingEnabled_ ? "on" : "off")
              << " mediaCrypto=" << (crypto.enabled ? "SNU2+ChaCha20" : "off")
              << " mediaEpoch=" << crypto.epoch
              << " bitrate=" << bitrate_
              << "\n";
  }

  ~UdpVideoClient() {
    if (socket_ != INVALID_SOCKET) {
      closesocket(socket_);
    }
  }

  UdpVideoClient(const UdpVideoClient&) = delete;
  UdpVideoClient& operator=(const UdpVideoClient&) = delete;

  void setBitrate(std::uint32_t bitrate) {
    bitrate_ = std::max<std::uint32_t>(bitrate, 1000000);
  }

  double takePacedSleepMs() {
    const double value = pacedSleepMs_;
    pacedSleepMs_ = 0.0;
    return value;
  }

  RetransmitCacheStats takeRetransmitCacheStats() {
    RetransmitCacheStats stats;
    stats.resets = windowCacheResets_;
    stats.droppedPackets = windowCacheDroppedPackets_;
    windowCacheResets_ = 0;
    windowCacheDroppedPackets_ = 0;
    return stats;
  }

  std::uint32_t sendPacket(const std::vector<std::uint8_t>& packetBytes) {
    if (packetBytes.empty()) return 0;
    if (packetBytes.size() > std::numeric_limits<std::uint32_t>::max()) {
      throw std::runtime_error("UDP packet payload too large.");
    }

    const MediaCryptoSnapshot crypto = mediaCrypto_ ? mediaCrypto_->snapshot() : MediaCryptoSnapshot{};
    syncRetransmitCacheGeneration(crypto);
    const std::uint64_t packetId = nextPacketId_++;
    packetCache_[packetId] = PacketCacheEntry{packetBytes, crypto.enabled ? crypto.generation : 0};
    packetCacheOrder_.push_back(packetId);
    while (packetCacheOrder_.size() > maxPacketCache_) {
      packetCache_.erase(packetCacheOrder_.front());
      packetCacheOrder_.pop_front();
    }
    return sendPacketWithId(packetId, packetBytes, false, crypto);
  }

  std::uint32_t resendPacket(std::uint64_t packetId) {
    const MediaCryptoSnapshot crypto = mediaCrypto_ ? mediaCrypto_->snapshot() : MediaCryptoSnapshot{};
    syncRetransmitCacheGeneration(crypto);
    const std::uint64_t currentGeneration = crypto.enabled ? crypto.generation : 0;
    const auto found = packetCache_.find(packetId);
    if (found == packetCache_.end()) return 0;
    if (found->second.mediaGeneration != currentGeneration) return 0;
    retransmittedPackets_ += 1;
    return sendPacketWithId(packetId, found->second.bytes, true, crypto);
  }

private:
  struct PacketCacheEntry {
    std::vector<std::uint8_t> bytes;
    std::uint64_t mediaGeneration = 0;
  };

  std::uint32_t sendPacketWithId(std::uint64_t packetId,
                                 const std::vector<std::uint8_t>& packetBytes,
                                 bool retransmit,
                                 const MediaCryptoSnapshot& crypto) {
    if (packetBytes.empty()) return 0;
    constexpr std::size_t maxPayloadBytes = 1152;
    const std::size_t fragmentCountSize = (packetBytes.size() + maxPayloadBytes - 1) / maxPayloadBytes;
    if (fragmentCountSize > std::numeric_limits<std::uint16_t>::max()) {
      throw std::runtime_error("UDP video packet requires too many fragments.");
    }
    const std::uint16_t fragmentCount = static_cast<std::uint16_t>(fragmentCountSize);
    if (fragmentCount == 0) return 0;

    syncMediaAuthEpoch(crypto, "video");

    std::uint32_t fragmentsSent = 0;
    for (std::uint16_t fragmentIndex = 0; fragmentIndex < fragmentCount; ++fragmentIndex) {
      const std::size_t offset = static_cast<std::size_t>(fragmentIndex) * maxPayloadBytes;
      const std::size_t chunkSize = std::min(maxPayloadBytes, packetBytes.size() - offset);

      std::vector<std::uint8_t> datagram;
      if (crypto.enabled) {
        UdpVideoAuthenticatedFragmentHeader header{};
        header.fragmentCount = fragmentCount;
        header.fragmentIndex = fragmentIndex;
        header.flags = (fragmentIndex == 0 ? 1u : 0u) |
                       (fragmentIndex + 1 == fragmentCount ? 2u : 0u) |
                       (retransmit ? 4u : 0u);
        header.packetId = packetId;
        header.packetSize = static_cast<std::uint32_t>(packetBytes.size());
        header.fragmentOffset = static_cast<std::uint32_t>(offset);
        header.payloadSize = static_cast<std::uint32_t>(chunkSize);
        header.authSeq = ++mediaAuthSequence_;
        header.authEpoch = crypto.epoch;

        datagram.resize(sizeof(header) + chunkSize);
        std::memcpy(datagram.data(), &header, sizeof(header));
        std::memcpy(datagram.data() + sizeof(header), packetBytes.data() + offset, chunkSize);
        chacha20Xor(datagram.data() + sizeof(header), chunkSize, crypto.videoCryptoKey, "video", header.authSeq);
        writeUdpAuthTag(crypto.videoAuthKey, "video", datagram, offsetof(UdpVideoAuthenticatedFragmentHeader, authTag));
      } else {
        UdpVideoFragmentHeader header{};
        header.fragmentCount = fragmentCount;
        header.fragmentIndex = fragmentIndex;
        header.flags = (fragmentIndex == 0 ? 1u : 0u) |
                       (fragmentIndex + 1 == fragmentCount ? 2u : 0u) |
                       (retransmit ? 4u : 0u);
        header.packetId = packetId;
        header.packetSize = static_cast<std::uint32_t>(packetBytes.size());
        header.fragmentOffset = static_cast<std::uint32_t>(offset);
        header.payloadSize = static_cast<std::uint32_t>(chunkSize);

        datagram.resize(sizeof(header) + chunkSize);
        std::memcpy(datagram.data(), &header, sizeof(header));
        std::memcpy(datagram.data() + sizeof(header), packetBytes.data() + offset, chunkSize);
      }

      paceDatagram(datagram.size());
      const int sent = send(socket_,
                            reinterpret_cast<const char*>(datagram.data()),
                            static_cast<int>(datagram.size()),
                            0);
      if (sent == SOCKET_ERROR || sent != static_cast<int>(datagram.size())) {
        throw std::runtime_error("UDP video send failed, WSA error " + std::to_string(WSAGetLastError()));
      }
      fragmentsSent += 1;
    }
    return fragmentsSent;
  }

  void syncRetransmitCacheGeneration(const MediaCryptoSnapshot& crypto) {
    const std::uint64_t generation = crypto.enabled ? crypto.generation : 0;
    if (!hasPacketCacheGeneration_) {
      packetCacheGeneration_ = generation;
      hasPacketCacheGeneration_ = true;
      return;
    }
    if (generation == packetCacheGeneration_) return;

    const std::size_t dropped = packetCache_.size();
    packetCache_.clear();
    packetCacheOrder_.clear();
    nextPacketId_ = 1;
    windowCacheResets_ += 1;
    windowCacheDroppedPackets_ += dropped;
    std::cerr << "SNU1_RETRANSMIT_CACHE_RESET generation=" << packetCacheGeneration_
              << " nextGeneration=" << generation
              << " droppedPackets=" << dropped
              << " nextPacketId=" << nextPacketId_
              << "\n";
    packetCacheGeneration_ = generation;
  }

  void syncMediaAuthEpoch(const MediaCryptoSnapshot& crypto, const char* media) {
    if (!crypto.enabled) {
      mediaAuthSequence_ = 0;
      mediaAuthEpoch_ = 0;
      hasMediaAuthEpoch_ = false;
      return;
    }
    if (hasMediaAuthEpoch_ && mediaAuthEpoch_ == crypto.epoch) return;
    mediaAuthEpoch_ = crypto.epoch;
    mediaAuthSequence_ = 0;
    hasMediaAuthEpoch_ = true;
    std::cerr << "SNMEDIA_AUTHSEQ_RESET media=" << media
              << " epoch=" << mediaAuthEpoch_
              << " generation=" << crypto.generation
              << "\n";
  }

  void paceDatagram(std::size_t datagramBytes) {
    if (!pacingEnabled_ || bitrate_ == 0 || datagramBytes == 0) return;

    const auto now = std::chrono::steady_clock::now();
    constexpr auto maxBurstWindow = std::chrono::milliseconds(6);
    if (nextSendAt_ + maxBurstWindow < now) {
      nextSendAt_ = now - maxBurstWindow;
    }
    if (nextSendAt_ > now) {
      const auto sleepFor = nextSendAt_ - now;
      pacedSleepMs_ += std::chrono::duration<double, std::milli>(sleepFor).count();
      if (sleepFor > std::chrono::milliseconds(2)) {
        std::this_thread::sleep_until(nextSendAt_ - std::chrono::microseconds(500));
      }
      while (std::chrono::steady_clock::now() < nextSendAt_) {
        std::this_thread::yield();
      }
    }

    const double datagramSeconds = (static_cast<double>(datagramBytes) * 8.0) /
      static_cast<double>(std::max<std::uint32_t>(bitrate_, 1000000));
    const auto spacing = std::chrono::duration_cast<std::chrono::steady_clock::duration>(
      std::chrono::duration<double>(datagramSeconds));
    nextSendAt_ = std::max(nextSendAt_, std::chrono::steady_clock::now()) + spacing;
  }

  SOCKET socket_ = INVALID_SOCKET;
  std::uint64_t nextPacketId_ = 1;
  std::map<std::uint64_t, PacketCacheEntry> packetCache_;
  std::deque<std::uint64_t> packetCacheOrder_;
  static constexpr std::size_t maxPacketCache_ = 180;
  std::uint64_t retransmittedPackets_ = 0;
  std::uint64_t packetCacheGeneration_ = 0;
  std::uint64_t windowCacheResets_ = 0;
  std::uint64_t windowCacheDroppedPackets_ = 0;
  std::uint32_t bitrate_ = 28000000;
  bool pacingEnabled_ = true;
  bool hasPacketCacheGeneration_ = false;
  std::shared_ptr<MediaCryptoState> mediaCrypto_;
  std::uint64_t mediaAuthSequence_ = 0;
  std::uint64_t mediaAuthEpoch_ = 0;
  bool hasMediaAuthEpoch_ = false;
  std::chrono::steady_clock::time_point nextSendAt_;
  double pacedSleepMs_ = 0.0;
};

using Microsoft::WRL::ComPtr;

enum class WasapiSampleFormat {
  Float32,
  Pcm16
};

struct WasapiFormatInfo {
  WasapiSampleFormat sampleFormat = WasapiSampleFormat::Float32;
  std::uint16_t channels = 0;
  std::uint32_t sampleRate = 0;
  std::uint16_t bitsPerSample = 0;
};

class UdpAudioClient {
public:
  explicit UdpAudioClient(const std::string& endpoint, std::shared_ptr<MediaCryptoState> mediaCrypto = {})
    : mediaCrypto_(std::move(mediaCrypto)) {
    const auto separator = endpoint.rfind(':');
    if (separator == std::string::npos || separator == 0 || separator == endpoint.size() - 1) {
      throw std::runtime_error("--audio-udp-connect must be HOST:PORT");
    }
    const std::string host = endpoint.substr(0, separator);
    const std::string port = endpoint.substr(separator + 1);

    addrinfo hints{};
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_DGRAM;
    hints.ai_protocol = IPPROTO_UDP;

    addrinfo* results = nullptr;
    const int lookup = getaddrinfo(host.c_str(), port.c_str(), &hints, &results);
    if (lookup != 0) {
      throw std::runtime_error("getaddrinfo failed for " + endpoint + ": " + std::to_string(lookup));
    }

    for (addrinfo* item = results; item; item = item->ai_next) {
      SOCKET candidate = socket(item->ai_family, item->ai_socktype, item->ai_protocol);
      if (candidate == INVALID_SOCKET) continue;

      if (connect(candidate, item->ai_addr, static_cast<int>(item->ai_addrlen)) == 0) {
        socket_ = candidate;
        break;
      }

      closesocket(candidate);
    }
    freeaddrinfo(results);

    if (socket_ == INVALID_SOCKET) {
      throw std::runtime_error("Could not connect --audio-udp-connect to " + endpoint +
                               ", WSA error " + std::to_string(WSAGetLastError()));
    }

    int sendBufferBytes = 1024 * 1024;
    setsockopt(socket_, SOL_SOCKET, SO_SNDBUF, reinterpret_cast<const char*>(&sendBufferBytes), sizeof(sendBufferBytes));
    const MediaCryptoSnapshot crypto = mediaCrypto_ ? mediaCrypto_->snapshot() : MediaCryptoSnapshot{};
    std::cerr << "SNA1 audio UDP connected " << endpoint
              << " mediaCrypto=" << (crypto.enabled ? "SNA2+ChaCha20" : "off")
              << " mediaEpoch=" << crypto.epoch
              << "\n";
  }

  ~UdpAudioClient() {
    if (socket_ != INVALID_SOCKET) {
      closesocket(socket_);
    }
  }

  UdpAudioClient(const UdpAudioClient&) = delete;
  UdpAudioClient& operator=(const UdpAudioClient&) = delete;

  std::uint32_t sendStereoFloat(std::uint32_t sampleRate,
                                const float* samples,
                                std::uint32_t frameCount) {
    if (socket_ == INVALID_SOCKET || !samples || frameCount == 0) return 0;

    constexpr std::uint16_t channels = 2;
    constexpr std::size_t maxPayloadBytes = 1152;
    constexpr std::size_t bytesPerFrame = channels * sizeof(float);
    constexpr std::uint32_t maxFramesPerPacket = static_cast<std::uint32_t>(maxPayloadBytes / bytesPerFrame);
    std::uint32_t sentPackets = 0;
    std::uint32_t offsetFrames = 0;
    while (offsetFrames < frameCount) {
      const std::uint32_t chunkFrames = std::min<std::uint32_t>(frameCount - offsetFrames, maxFramesPerPacket);
      const std::uint32_t payloadSize = chunkFrames * static_cast<std::uint32_t>(bytesPerFrame);

      std::array<std::uint8_t, sizeof(AudioAuthenticatedPacketHeader) + maxPayloadBytes> datagram{};
      int datagramSize = 0;
      const MediaCryptoSnapshot crypto = mediaCrypto_ ? mediaCrypto_->snapshot() : MediaCryptoSnapshot{};
      syncMediaAuthEpoch(crypto, "audio");
      if (crypto.enabled) {
        AudioAuthenticatedPacketHeader header{};
        header.channels = channels;
        header.sampleRate = sampleRate;
        header.frameCount = chunkFrames;
        header.sequence = nextSequence_++;
        header.hostUnixMicros = unixMicros();
        header.payloadSize = payloadSize;
        header.authSeq = ++mediaAuthSequence_;
        header.authEpoch = crypto.epoch;

        std::memcpy(datagram.data(), &header, sizeof(header));
        std::memcpy(datagram.data() + sizeof(header),
                    samples + static_cast<std::size_t>(offsetFrames) * channels,
                    payloadSize);
        datagramSize = static_cast<int>(sizeof(header) + payloadSize);
        chacha20Xor(datagram.data() + sizeof(header), payloadSize, crypto.audioCryptoKey, "audio", header.authSeq);
        writeUdpAuthTag(crypto.audioAuthKey,
                        "audio",
                        datagram,
                        static_cast<std::size_t>(datagramSize),
                        offsetof(AudioAuthenticatedPacketHeader, authTag));
      } else {
        AudioPacketHeader header{};
        header.channels = channels;
        header.sampleRate = sampleRate;
        header.frameCount = chunkFrames;
        header.sequence = nextSequence_++;
        header.hostUnixMicros = unixMicros();
        header.payloadSize = payloadSize;

        std::memcpy(datagram.data(), &header, sizeof(header));
        std::memcpy(datagram.data() + sizeof(header),
                    samples + static_cast<std::size_t>(offsetFrames) * channels,
                    payloadSize);
        datagramSize = static_cast<int>(sizeof(header) + payloadSize);
      }
      const int sent = send(socket_,
                            reinterpret_cast<const char*>(datagram.data()),
                            datagramSize,
                            0);
      if (sent == SOCKET_ERROR || sent != datagramSize) {
        throw std::runtime_error("SNA1 audio UDP send failed, WSA error " + std::to_string(WSAGetLastError()));
      }
      sentPackets += 1;
      offsetFrames += chunkFrames;
    }
    return sentPackets;
  }

private:
  void syncMediaAuthEpoch(const MediaCryptoSnapshot& crypto, const char* media) {
    if (!crypto.enabled) {
      mediaAuthSequence_ = 0;
      mediaAuthEpoch_ = 0;
      hasMediaAuthEpoch_ = false;
      return;
    }
    if (hasMediaAuthEpoch_ && mediaAuthEpoch_ == crypto.epoch) return;
    const std::uint64_t previousNextSequence = nextSequence_;
    mediaAuthEpoch_ = crypto.epoch;
    mediaAuthSequence_ = 0;
    nextSequence_ = 1;
    hasMediaAuthEpoch_ = true;
    std::cerr << "SNMEDIA_AUTHSEQ_RESET media=" << media
              << " epoch=" << mediaAuthEpoch_
              << " generation=" << crypto.generation
              << "\n";
    std::cerr << "SNA1_AUDIO_SEQUENCE_RESET epoch=" << mediaAuthEpoch_
              << " generation=" << crypto.generation
              << " previousNextSequence=" << previousNextSequence
              << " nextSequence=" << nextSequence_
              << "\n";
  }

  SOCKET socket_ = INVALID_SOCKET;
  std::uint64_t nextSequence_ = 1;
  std::shared_ptr<MediaCryptoState> mediaCrypto_;
  std::uint64_t mediaAuthSequence_ = 0;
  std::uint64_t mediaAuthEpoch_ = 0;
  bool hasMediaAuthEpoch_ = false;
};

class WasapiLoopbackAudioSender {
public:
  explicit WasapiLoopbackAudioSender(std::string endpoint, std::shared_ptr<MediaCryptoState> mediaCrypto = {})
    : endpoint_(std::move(endpoint)),
      mediaCrypto_(std::move(mediaCrypto)),
      worker_(&WasapiLoopbackAudioSender::run, this) {}

  ~WasapiLoopbackAudioSender() {
    running_ = false;
    HANDLE event = wakeEvent_.load();
    if (event) SetEvent(event);
    if (worker_.joinable()) {
      worker_.join();
    }
  }

  WasapiLoopbackAudioSender(const WasapiLoopbackAudioSender&) = delete;
  WasapiLoopbackAudioSender& operator=(const WasapiLoopbackAudioSender&) = delete;

private:
  static void requireHr(HRESULT hr, const char* label) {
    if (FAILED(hr)) {
      std::ostringstream out;
      out << label << " failed hr=0x" << std::hex << static_cast<unsigned long>(hr);
      throw std::runtime_error(out.str());
    }
  }

  static WasapiFormatInfo inspectFormat(const WAVEFORMATEX* format) {
    if (!format) throw std::runtime_error("WASAPI mix format is null.");

    WasapiFormatInfo info;
    info.channels = format->nChannels;
    info.sampleRate = format->nSamplesPerSec;
    info.bitsPerSample = format->wBitsPerSample;

    if (format->wFormatTag == WAVE_FORMAT_IEEE_FLOAT && format->wBitsPerSample == 32) {
      info.sampleFormat = WasapiSampleFormat::Float32;
      return info;
    }
    if (format->wFormatTag == WAVE_FORMAT_PCM && format->wBitsPerSample == 16) {
      info.sampleFormat = WasapiSampleFormat::Pcm16;
      return info;
    }
    if (format->wFormatTag == WAVE_FORMAT_EXTENSIBLE &&
        format->cbSize >= sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX)) {
      const auto* extensible = reinterpret_cast<const WAVEFORMATEXTENSIBLE*>(format);
      if (IsEqualGUID(extensible->SubFormat, KSDATAFORMAT_SUBTYPE_IEEE_FLOAT) &&
          format->wBitsPerSample == 32) {
        info.sampleFormat = WasapiSampleFormat::Float32;
        return info;
      }
      if (IsEqualGUID(extensible->SubFormat, KSDATAFORMAT_SUBTYPE_PCM) &&
          format->wBitsPerSample == 16) {
        info.sampleFormat = WasapiSampleFormat::Pcm16;
        return info;
      }
    }

    throw std::runtime_error("Unsupported WASAPI mix format tag=" + std::to_string(format->wFormatTag) +
                             " bits=" + std::to_string(format->wBitsPerSample));
  }

  static const char* formatLabel(WasapiSampleFormat format) {
    switch (format) {
      case WasapiSampleFormat::Float32: return "float32";
      case WasapiSampleFormat::Pcm16: return "pcm16";
    }
    return "unknown";
  }

  static float sampleAt(const BYTE* data,
                        WasapiSampleFormat format,
                        std::uint16_t channels,
                        std::uint32_t frame,
                        std::uint16_t channel) {
    const std::size_t index = static_cast<std::size_t>(frame) * channels + channel;
    if (format == WasapiSampleFormat::Float32) {
      return reinterpret_cast<const float*>(data)[index];
    }
    return static_cast<float>(reinterpret_cast<const std::int16_t*>(data)[index]) / 32768.0f;
  }

  static void convertToStereoFloat(const BYTE* data,
                                   std::uint32_t frameCount,
                                   const WasapiFormatInfo& format,
                                   bool silent,
                                   std::vector<float>& output) {
    output.assign(static_cast<std::size_t>(frameCount) * 2, 0.0f);
    if (silent || !data) return;

    for (std::uint32_t frame = 0; frame < frameCount; ++frame) {
      const float left = sampleAt(data, format.sampleFormat, format.channels, frame, 0);
      const float right = format.channels > 1
        ? sampleAt(data, format.sampleFormat, format.channels, frame, 1)
        : left;
      output[static_cast<std::size_t>(frame) * 2] = std::clamp(left, -1.0f, 1.0f);
      output[static_cast<std::size_t>(frame) * 2 + 1] = std::clamp(right, -1.0f, 1.0f);
    }
  }

  void run() {
    HRESULT coinit = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    const bool comInitialized = SUCCEEDED(coinit);
    if (coinit == RPC_E_CHANGED_MODE) {
      coinit = S_OK;
    }

    HANDLE event = nullptr;
    HANDLE avrtHandle = nullptr;
    DWORD avrtTaskIndex = 0;
    WAVEFORMATEX* mixFormat = nullptr;
    try {
      requireHr(coinit, "CoInitializeEx");

      UdpAudioClient audioClient(endpoint_, mediaCrypto_);
      ComPtr<IMMDeviceEnumerator> enumerator;
      requireHr(CoCreateInstance(__uuidof(MMDeviceEnumerator),
                                 nullptr,
                                 CLSCTX_ALL,
                                 __uuidof(IMMDeviceEnumerator),
                                 reinterpret_cast<void**>(enumerator.GetAddressOf())),
                "CoCreateInstance(MMDeviceEnumerator)");

      ComPtr<IMMDevice> device;
      requireHr(enumerator->GetDefaultAudioEndpoint(eRender, eConsole, device.GetAddressOf()),
                "GetDefaultAudioEndpoint");

      ComPtr<IAudioClient> wasapiClient;
      requireHr(device->Activate(__uuidof(IAudioClient),
                                 CLSCTX_ALL,
                                 nullptr,
                                 reinterpret_cast<void**>(wasapiClient.GetAddressOf())),
                "IMMDevice::Activate(IAudioClient)");

      requireHr(wasapiClient->GetMixFormat(&mixFormat), "IAudioClient::GetMixFormat");
      const WasapiFormatInfo format = inspectFormat(mixFormat);
      if (format.channels == 0 || format.channels > 8 || format.sampleRate == 0) {
        throw std::runtime_error("Unsupported WASAPI channel/rate combination.");
      }

      REFERENCE_TIME defaultPeriod = 0;
      REFERENCE_TIME minimumPeriod = 0;
      requireHr(wasapiClient->GetDevicePeriod(&defaultPeriod, &minimumPeriod), "IAudioClient::GetDevicePeriod");
      const REFERENCE_TIME bufferDuration = defaultPeriod > 0 ? defaultPeriod : 100000;
      requireHr(wasapiClient->Initialize(AUDCLNT_SHAREMODE_SHARED,
                                         AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                                         bufferDuration,
                                         0,
                                         mixFormat,
                                         nullptr),
                "IAudioClient::Initialize(loopback)");

      event = CreateEventW(nullptr, FALSE, FALSE, nullptr);
      if (!event) {
        throw std::runtime_error("CreateEventW for WASAPI audio failed.");
      }
      wakeEvent_ = event;
      requireHr(wasapiClient->SetEventHandle(event), "IAudioClient::SetEventHandle");

      ComPtr<IAudioCaptureClient> captureClient;
      requireHr(wasapiClient->GetService(__uuidof(IAudioCaptureClient),
                                         reinterpret_cast<void**>(captureClient.GetAddressOf())),
                "IAudioClient::GetService(IAudioCaptureClient)");

      avrtHandle = AvSetMmThreadCharacteristicsW(L"Audio", &avrtTaskIndex);
      requireHr(wasapiClient->Start(), "IAudioClient::Start");
      std::cerr << "SNA1_AUDIO_CAPTURE sampleRate=" << format.sampleRate
                << " sourceChannels=" << format.channels
                << " sendChannels=2"
                << " format=" << formatLabel(format.sampleFormat)
                << "\n";

      std::vector<float> stereoSamples;
      auto statsStartedAt = std::chrono::steady_clock::now();
      std::uint64_t statsPackets = 0;
      std::uint64_t statsFrames = 0;
      std::uint64_t statsBytes = 0;
      std::uint64_t statsSilentFrames = 0;

      while (running_) {
        const DWORD wait = WaitForSingleObject(event, 200);
        if (!running_) break;
        if (wait != WAIT_OBJECT_0 && wait != WAIT_TIMEOUT) {
          throw std::runtime_error("WASAPI audio wait failed.");
        }

        UINT32 packetFrames = 0;
        requireHr(captureClient->GetNextPacketSize(&packetFrames), "IAudioCaptureClient::GetNextPacketSize");
        while (packetFrames > 0 && running_) {
          BYTE* data = nullptr;
          UINT32 frameCount = 0;
          DWORD flags = 0;
          requireHr(captureClient->GetBuffer(&data, &frameCount, &flags, nullptr, nullptr),
                    "IAudioCaptureClient::GetBuffer");
          const bool silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT) != 0;
          convertToStereoFloat(data, frameCount, format, silent, stereoSamples);
          requireHr(captureClient->ReleaseBuffer(frameCount), "IAudioCaptureClient::ReleaseBuffer");
          statsPackets += audioClient.sendStereoFloat(format.sampleRate, stereoSamples.data(), frameCount);
          statsFrames += frameCount;
          statsBytes += static_cast<std::uint64_t>(frameCount) * 2 * sizeof(float);
          if (silent) statsSilentFrames += frameCount;
          requireHr(captureClient->GetNextPacketSize(&packetFrames), "IAudioCaptureClient::GetNextPacketSize");
        }

        const auto statsNow = std::chrono::steady_clock::now();
        const double statsSeconds = std::chrono::duration<double>(statsNow - statsStartedAt).count();
        if (statsSeconds >= 1.0) {
          const double kbps = static_cast<double>(statsBytes) * 8.0 / statsSeconds / 1000.0;
          std::cerr << "SNA1_AUDIO_STATS packets=" << statsPackets
                    << " frames=" << statsFrames
                    << " kbps=" << std::fixed << std::setprecision(1) << kbps
                    << " silentFrames=" << statsSilentFrames
                    << "\n";
          statsStartedAt = statsNow;
          statsPackets = 0;
          statsFrames = 0;
          statsBytes = 0;
          statsSilentFrames = 0;
        }
      }

      wasapiClient->Stop();
    } catch (const std::exception& error) {
      std::cerr << "SNA1_AUDIO_CAPTURE error: " << error.what() << "\n";
    }

    wakeEvent_ = nullptr;
    if (avrtHandle) {
      AvRevertMmThreadCharacteristics(avrtHandle);
    }
    if (event) {
      CloseHandle(event);
    }
    if (mixFormat) {
      CoTaskMemFree(mixFormat);
    }
    if (comInitialized) {
      CoUninitialize();
    }
  }

  std::string endpoint_;
  std::shared_ptr<MediaCryptoState> mediaCrypto_;
  std::atomic<bool> running_{true};
  std::atomic<HANDLE> wakeEvent_{nullptr};
  std::thread worker_;
};
#endif

void writePipeFrame(const FrameBgra& frame) {
  PipeFrameHeader header;
  header.width = frame.width;
  header.height = frame.height;
  header.stride = frame.stride;
  header.timestampMicros = nowMicros();
  header.payloadSize = static_cast<std::uint32_t>(frame.pixels.size());

  std::cout.write(reinterpret_cast<const char*>(&header), sizeof(header));
  std::cout.write(reinterpret_cast<const char*>(frame.pixels.data()), frame.pixels.size());
  std::cout.flush();
}

std::vector<std::uint8_t> makeEncodedPacketBytes(const EncodedVideoPacket& packet,
                                                 std::uint32_t width,
                                                 std::uint32_t height,
                                                 std::uint64_t sequence,
                                                 bool hardware) {
  EncodedPacketHeader header;
  header.width = width;
  header.height = height;
  header.sequence = sequence;
  header.timestampMicros = packet.timestampMicros;
  header.durationMicros = packet.durationMicros;
  header.flags = (packet.keyframe ? 1u : 0u) | (hardware ? 2u : 0u);
  header.payloadSize = static_cast<std::uint32_t>(packet.payload.size());
  header.hostUnixMicros = unixMicros();

  std::vector<std::uint8_t> bytes(sizeof(header) + packet.payload.size());
  std::memcpy(bytes.data(), &header, sizeof(header));
  if (!packet.payload.empty()) {
    std::memcpy(bytes.data() + sizeof(header), packet.payload.data(), packet.payload.size());
  }
  return bytes;
}

void writeEncodedPacket(std::ostream& output,
                        const EncodedVideoPacket& packet,
                        std::uint32_t width,
                        std::uint32_t height,
                        std::uint64_t sequence,
                        bool hardware) {
  const auto bytes = makeEncodedPacketBytes(packet, width, height, sequence, hardware);
  output.write(reinterpret_cast<const char*>(bytes.data()), bytes.size());
}

int runPipeMode(DesktopDuplicator& duplicator, const Options& options) {
#ifdef _WIN32
  _setmode(_fileno(stdout), _O_BINARY);
#endif
  const auto frameDelay = options.fps > 0
    ? std::chrono::microseconds(1000000 / options.fps)
    : std::chrono::microseconds(0);
  while (true) {
    const auto frameStartedAt = std::chrono::steady_clock::now();
    FrameBgra frame;
    if (duplicator.captureFrame(frame, 1000)) {
      writePipeFrame(frame);
    }

    if (frameDelay.count() > 0) {
      std::this_thread::sleep_until(frameStartedAt + frameDelay);
    }
  }
}

int runEncodeMode(DesktopDuplicator& duplicator, const Options& options) {
  if (options.outputFile.has_parent_path()) {
    std::filesystem::create_directories(options.outputFile.parent_path());
  }

  VideoEncodeOptions encodeOptions;
  encodeOptions.codec = options.codec;
  encodeOptions.outputFile = options.outputFile;
  encodeOptions.fps = options.fps;
  encodeOptions.bitrate = options.bitrate;
  encodeOptions.hardware = options.hardwareEncoder;

  MfVideoFileEncoder encoder(duplicator.width(), duplicator.height(), encodeOptions);

  std::cout << "Encoding " << options.frames << " frame(s) to "
            << options.outputFile.string() << " as " << videoCodecName(options.codec)
            << " at " << options.fps << " FPS, " << options.bitrate << " bps"
            << (options.hardwareEncoder ? " with hardware transforms requested" : " with software encoder")
            << "\n";

  std::uint32_t encoded = 0;
  while (encoded < options.frames) {
    const auto frameStartedAt = std::chrono::steady_clock::now();
    FrameBgra frame;
    if (!duplicator.captureFrame(frame, 1000)) {
      std::cout << "Timed out waiting for a changed frame.\n";
      continue;
    }

    encoder.writeFrame(frame);
    ++encoded;

    if (options.intervalMs > 0) {
      std::this_thread::sleep_for(std::chrono::milliseconds(options.intervalMs));
    } else if (options.fps > 0) {
      std::this_thread::sleep_until(frameStartedAt + std::chrono::microseconds(1000000 / options.fps));
    }
  }

  encoder.finalize();
  std::cout << "Wrote " << options.outputFile.string() << "\n";
  return 0;
}

#ifdef _WIN32
void putPixelBgra(FrameBgra& frame, int x, int y, std::uint8_t r, std::uint8_t g, std::uint8_t b) {
  if (x < 0 || y < 0 || x >= static_cast<int>(frame.width) || y >= static_cast<int>(frame.height)) return;
  auto* pixel = frame.pixels.data()
    + static_cast<std::size_t>(y) * frame.stride
    + static_cast<std::size_t>(x) * 4;
  pixel[0] = b;
  pixel[1] = g;
  pixel[2] = r;
  pixel[3] = 255;
}

void drawSoftwareCursor(FrameBgra& frame, const DesktopDuplicator& duplicator) {
  CURSORINFO cursorInfo{};
  cursorInfo.cbSize = sizeof(cursorInfo);
  if (!GetCursorInfo(&cursorInfo) || (cursorInfo.flags & CURSOR_SHOWING) == 0) return;

  const int originX = cursorInfo.ptScreenPos.x - static_cast<int>(duplicator.left());
  const int originY = cursorInfo.ptScreenPos.y - static_cast<int>(duplicator.top());
  if (originX < -24 || originY < -24 ||
      originX >= static_cast<int>(frame.width) ||
      originY >= static_cast<int>(frame.height)) {
    return;
  }

  std::vector<std::pair<int, int>> white;
  for (int y = 0; y <= 16; ++y) {
    const int maxX = std::min(8, y / 2 + 1);
    for (int x = 0; x <= maxX; ++x) white.emplace_back(x, y);
  }
  for (int y = 11; y <= 22; ++y) {
    for (int x = 3; x <= 5; ++x) white.emplace_back(x, y);
  }

  for (const auto& [x, y] : white) {
    for (int dy = -1; dy <= 1; ++dy) {
      for (int dx = -1; dx <= 1; ++dx) {
        putPixelBgra(frame, originX + x + dx, originY + y + dy, 0, 0, 0);
      }
    }
  }
  for (const auto& [x, y] : white) {
    putPixelBgra(frame, originX + x, originY + y, 255, 255, 255);
  }
}
#endif

int runEncodedPipeMode(DesktopDuplicator& duplicator, const Options& options) {
  if (options.codec != VideoCodec::H264) {
    throw std::runtime_error("--encode-pipe currently supports h264 only.");
  }
  const bool hasNetworkVideo = !options.tcpConnect.empty() || !options.udpConnect.empty();
  if (hasNetworkVideo && !options.packetFile.empty()) {
    throw std::runtime_error("Use network video or --packet-file, not both.");
  }
  if (!options.tcpConnect.empty() && !options.udpConnect.empty()) {
    throw std::runtime_error("Use either --tcp-connect or --udp-connect, not both.");
  }
  if (!options.audioUdpConnect.empty() && !hasNetworkVideo) {
    throw std::runtime_error("--audio-udp-connect requires --tcp-connect or --udp-connect video.");
  }

  std::unique_ptr<std::ofstream> packetFile;
  std::ostream* output = &std::cout;
#ifdef _WIN32
  std::unique_ptr<WinsockRuntime> winsock;
  std::unique_ptr<TcpClient> tcpClient;
  std::unique_ptr<TcpClient> controlClient;
  std::unique_ptr<UdpVideoClient> udpClient;
  std::unique_ptr<WasapiLoopbackAudioSender> audioSender;
  std::shared_ptr<MediaCryptoState> mediaCrypto = std::make_shared<MediaCryptoState>(options.sessionToken);
#endif

  if (!options.tcpConnect.empty()) {
#ifdef _WIN32
    winsock = std::make_unique<WinsockRuntime>();
    tcpClient = std::make_unique<TcpClient>(options.tcpConnect, options.controlConnect, options.sessionToken, mediaCrypto);
    tcpClient->startControlReceiver();
#else
    throw std::runtime_error("--tcp-connect is currently implemented for Windows host builds.");
#endif
  } else if (!options.udpConnect.empty()) {
#ifdef _WIN32
    winsock = std::make_unique<WinsockRuntime>();
    udpClient = std::make_unique<UdpVideoClient>(options.udpConnect,
                                                 options.bitrate,
                                                 options.udpPacing,
                                                 mediaCrypto);
    if (!options.controlConnect.empty()) {
      controlClient = std::make_unique<TcpClient>("", options.controlConnect, options.sessionToken, mediaCrypto);
      controlClient->startControlReceiver();
    } else {
      std::cerr << "SNU1 UDP video has no --control-connect; native input/stats feedback disabled.\n";
    }
#else
    throw std::runtime_error("--udp-connect is currently implemented for Windows host builds.");
#endif
  } else if (!options.packetFile.empty()) {
    if (options.packetFile.has_parent_path()) {
      std::filesystem::create_directories(options.packetFile.parent_path());
    }
    packetFile = std::make_unique<std::ofstream>(options.packetFile, std::ios::binary);
    if (!packetFile->is_open()) {
      throw std::runtime_error("Could not open packet output file: " + options.packetFile.string());
    }
    output = packetFile.get();
  } else {
#ifdef _WIN32
    _setmode(_fileno(stdout), _O_BINARY);
#endif
  }

#ifdef _WIN32
  if (!options.audioUdpConnect.empty()) {
    if (!winsock) {
      winsock = std::make_unique<WinsockRuntime>();
    }
    audioSender = std::make_unique<WasapiLoopbackAudioSender>(options.audioUdpConnect, mediaCrypto);
  }
#else
  if (!options.audioUdpConnect.empty()) {
    throw std::runtime_error("--audio-udp-connect is currently implemented for Windows host builds.");
  }
#endif

  VideoPacketEncodeOptions encodeOptions;
  encodeOptions.codec = VideoCodec::H264;
  encodeOptions.fps = options.fps;
  encodeOptions.bitrate = options.bitrate;
  encodeOptions.hardware = options.hardwareEncoder;
  encodeOptions.lowLatency = options.lowLatencyEncoder;
  encodeOptions.keyframeIntervalSeconds = options.keyframeIntervalSeconds;

  auto encoder = std::make_unique<MfVideoPacketEncoder>(duplicator.width(), duplicator.height(), encodeOptions);
  auto restartEncoder = [&](std::uint32_t bitrate, const char* reason) -> bool {
    auto nextOptions = encodeOptions;
    nextOptions.bitrate = bitrate;
    try {
      auto replacement = std::make_unique<MfVideoPacketEncoder>(duplicator.width(), duplicator.height(), nextOptions);
      encoder = std::move(replacement);
      encodeOptions = nextOptions;
      std::cerr << "SNV1_ENCODER_RESTART reason=" << reason
                << " bitrate=" << bitrate
                << " hardware=" << (encoder->usingHardware() ? "yes" : "no")
                << "\n";
      return true;
    } catch (const std::exception& error) {
      std::cerr << "SNV1_ENCODER_RESTART failed reason=" << reason
                << " bitrate=" << bitrate
                << " error=" << error.what()
                << "\n";
      return false;
    }
  };

  std::cerr << "SNV1 H.264 packet stream: "
            << duplicator.width() << "x" << duplicator.height()
            << " @ " << options.fps << " FPS, " << options.bitrate << " bps, "
            << (encoder->usingHardware() ? "hardware encoder" : "software encoder")
            << ", lowLatency=" << (options.lowLatencyEncoder ? "yes" : "no")
            << ", keyframeInterval=" << options.keyframeIntervalSeconds << "s"
            << (!options.udpConnect.empty()
                  ? " -> udp " + options.udpConnect
                  : (!options.tcpConnect.empty()
                  ? " -> tcp " + options.tcpConnect
                  : (options.packetFile.empty() ? " -> stdout" : " -> " + options.packetFile.string())))
            << (options.audioUdpConnect.empty() ? "" : " + audio " + options.audioUdpConnect)
            << (!options.udpConnect.empty()
                  ? std::string(" udpPacing=") + (options.udpPacing ? "on" : "off")
                  : "")
            << "\n";

  auto frameDelayForFps = [](std::uint32_t fps) {
    return fps > 0 ? std::chrono::microseconds(1000000 / fps) : std::chrono::microseconds(0);
  };
  auto captureTimeoutForFps = [](std::uint32_t fps) {
    return fps > 0 ? std::max<std::uint32_t>(1, 1000 / fps) : 16;
  };
  const std::uint32_t requestedFps = std::max<std::uint32_t>(options.fps, 1);
  const bool adaptiveFramePacing = hasNetworkVideo && options.intervalMs == 0 && options.fps > 0;
  const std::uint32_t maxAdaptiveFps = requestedFps;
  const std::uint32_t minAdaptiveFps = std::min<std::uint32_t>(
    maxAdaptiveFps,
    std::max<std::uint32_t>(15, maxAdaptiveFps / 2));
  std::uint32_t currentAdaptiveFps = requestedFps;
  auto adaptiveFrameDelay = frameDelayForFps(currentAdaptiveFps);
  std::uint32_t adaptiveCaptureTimeoutMs = captureTimeoutForFps(currentAdaptiveFps);
  std::uint32_t clearFrameFeedbackWindows = 0;
  auto currentStreamDurationMicros = [&]() -> std::uint64_t {
    if (options.intervalMs > 0) {
      return static_cast<std::uint64_t>(options.intervalMs) * 1000;
    }
    if (adaptiveFrameDelay.count() > 0) {
      return static_cast<std::uint64_t>(adaptiveFrameDelay.count());
    }
    return 16667;
  };
  std::cerr << "SNV1_FRAME_ADAPT_CONFIG enabled=" << (adaptiveFramePacing ? "yes" : "no")
            << " minFps=" << minAdaptiveFps
            << " maxFps=" << maxAdaptiveFps
            << " initialFps=" << currentAdaptiveFps
            << "\n";
  const std::uint64_t targetFrames = (!options.framesProvided || options.frames == 0)
    ? std::numeric_limits<std::uint64_t>::max()
    : options.frames;

  std::uint64_t capturedFrames = 0;
  std::uint64_t sequence = 0;
  std::uint64_t lastStreamTimestampMicros = 0;
  bool hasLastStreamTimestamp = false;
  auto normalizePacketTimestamp = [&](EncodedVideoPacket packet, std::uint64_t streamDurationMicros) {
    const std::uint64_t safeDurationMicros = std::clamp<std::uint64_t>(
      streamDurationMicros > 0
        ? streamDurationMicros
        : (packet.durationMicros > 0 ? packet.durationMicros : 16667),
      1000,
      1000000);
    std::uint64_t adjustedTimestamp = packet.timestampMicros;
    if (hasLastStreamTimestamp) {
      adjustedTimestamp = lastStreamTimestampMicros + safeDurationMicros;
    }
    packet.timestampMicros = adjustedTimestamp;
    packet.durationMicros = static_cast<std::uint32_t>(safeDurationMicros);
    lastStreamTimestampMicros = adjustedTimestamp;
    hasLastStreamTimestamp = true;
    return packet;
  };
  auto statsStartedAt = std::chrono::steady_clock::now();
  auto lastTimeoutLogAt = statsStartedAt;
  std::uint64_t statsFrames = 0;
  std::uint64_t statsPackets = 0;
  std::uint64_t statsBytes = 0;
  std::uint64_t statsUdpFragments = 0;
  double statsUdpPacedMs = 0.0;
  std::uint64_t statsKeyframes = 0;
  const std::uint32_t maxAdaptiveBitrate = std::max<std::uint32_t>(options.bitrate, 1);
  const std::uint32_t minAdaptiveBitrate = std::min<std::uint32_t>(
    maxAdaptiveBitrate,
    std::max<std::uint32_t>(1000000, maxAdaptiveBitrate / 4));
  std::uint32_t currentAdaptiveBitrate = options.bitrate;
  std::uint64_t seenFeedbackSequence = 0;
  std::uint64_t seenUdpRepairFeedbackSequence = 0;
  std::uint64_t seenKeyframeRequestSequence = 0;
  std::uint64_t seenVideoNackSequence = 0;
  std::uint32_t clearFeedbackWindows = 0;
  std::uint32_t clearUdpRepairWindows = 0;
  std::uint32_t renderRecoveryHoldWindows = 0;
  std::uint64_t statsFeedbackWindowSequence = 0;
  std::uint64_t renderRecoveryHoldConsumedWindow = 0;
  std::uint64_t recoveryIncreaseAppliedWindow = 0;
  std::uint64_t lastStreamFeedbackWindow = 0;
  std::uint64_t lastUdpRepairFeedbackWindow = 0;
  std::uint64_t lastFeedbackStaleLogWindow = 0;
  constexpr std::uint64_t feedbackFreshWindowLimit = 3;
  constexpr std::uint64_t feedbackStaleLogIntervalWindows = 3;
  constexpr std::uint32_t recoveryCrossClearWindows = 2;
  constexpr std::uint32_t udpRepairBitrateClearWindows = 5;
  constexpr std::uint32_t udpRepairFrameClearWindows = 7;
  constexpr std::uint32_t streamBitrateClearWindows = 4;
  constexpr std::uint32_t streamFrameClearWindows = 5;
  auto lastEncoderRestartAt = std::chrono::steady_clock::time_point{};
#ifdef _WIN32
  FrameBgra lastCleanFrame;
  bool hasLastCleanFrame = false;
  POINT lastCursor{};
  bool hasLastCursor = false;
  std::uint64_t statsCursorFrames = 0;
  std::uint64_t statsRefreshFrames = 0;
  std::uint64_t statsUdpNackRequests = 0;
  std::uint64_t statsUdpNackMisses = 0;
  std::uint64_t statsUdpRetransmits = 0;
  std::uint64_t statsUdpRetransmitFragments = 0;
  std::uint64_t statsUdpCacheResets = 0;
  std::uint64_t statsUdpCacheDropped = 0;
  std::uint64_t seenMediaCryptoGeneration = mediaCrypto ? mediaCrypto->snapshot().generation : 0;
#endif

  while (capturedFrames < targetFrames) {
    const auto frameStartedAt = std::chrono::steady_clock::now();
#ifdef _WIN32
    bool hasPendingMediaEpochKeyframe = false;
    std::uint64_t pendingMediaEpoch = 0;
    std::uint64_t pendingMediaGeneration = 0;
    if (mediaCrypto) {
      const MediaCryptoSnapshot mediaSnapshot = mediaCrypto->snapshot();
      if (mediaSnapshot.enabled &&
          mediaSnapshot.generation != 0 &&
          mediaSnapshot.generation != seenMediaCryptoGeneration) {
        hasPendingMediaEpochKeyframe = true;
        pendingMediaEpoch = mediaSnapshot.epoch;
        pendingMediaGeneration = mediaSnapshot.generation;
      }
    }
    const auto pendingVideoNack = latestVideoNack();
    if (pendingVideoNack.sequence != 0 && pendingVideoNack.sequence != seenVideoNackSequence) {
      seenVideoNackSequence = pendingVideoNack.sequence;
      std::uint64_t retransmittedPackets = 0;
      std::uint64_t retransmittedFragments = 0;
      std::uint64_t cacheMisses = 0;
      if (udpClient) {
        for (const std::uint64_t packetId : pendingVideoNack.packetIds) {
          const std::uint32_t fragments = udpClient->resendPacket(packetId);
          if (fragments > 0) {
            retransmittedPackets += 1;
            retransmittedFragments += fragments;
          } else {
            cacheMisses += 1;
          }
        }
        statsUdpNackRequests += pendingVideoNack.packetIds.size();
        statsUdpNackMisses += cacheMisses;
        statsUdpRetransmits += retransmittedPackets;
        statsUdpRetransmitFragments += retransmittedFragments;
      } else {
        cacheMisses = pendingVideoNack.packetIds.size();
      }
      std::cerr << "SNU1_NACK_APPLY sequence=" << pendingVideoNack.sequence
                << " reason=" << pendingVideoNack.reason
                << " requested=" << pendingVideoNack.packetIds.size()
                << " resent=" << retransmittedPackets
                << " missed=" << cacheMisses
                << " fragments=" << retransmittedFragments
                << "\n";
    }
#endif
    const auto pendingKeyframeRequest = latestKeyframeRequest();
    const bool hasPendingKeyframeRequest = pendingKeyframeRequest.sequence != 0 &&
                                           pendingKeyframeRequest.sequence != seenKeyframeRequestSequence;
    FrameBgra frame;
    bool cursorOnlyFrame = false;
    if (!duplicator.captureFrame(frame, adaptiveCaptureTimeoutMs)) {
#ifdef _WIN32
      POINT cursor{};
      const bool cursorOk = GetCursorPos(&cursor) != 0;
      const bool cursorChanged = cursorOk
        && (!hasLastCursor || cursor.x != lastCursor.x || cursor.y != lastCursor.y);
      if (hasLastCleanFrame && (hasPendingKeyframeRequest || hasPendingMediaEpochKeyframe)) {
        frame = lastCleanFrame;
        ++statsRefreshFrames;
      } else if (hasLastCleanFrame && cursorChanged) {
        frame = lastCleanFrame;
        cursorOnlyFrame = true;
      } else
#endif
      {
        const auto now = std::chrono::steady_clock::now();
        if (now - lastTimeoutLogAt > std::chrono::seconds(2)) {
          lastTimeoutLogAt = now;
          std::cerr << "SNV1 waiting for changed frame.\n";
        }
        continue;
      }
    } else {
#ifdef _WIN32
      lastCleanFrame = frame;
      hasLastCleanFrame = true;
#endif
    }

#ifdef _WIN32
    if (cursorOnlyFrame) {
      ++statsCursorFrames;
    }
    drawSoftwareCursor(frame, duplicator);
    POINT currentCursor{};
    if (GetCursorPos(&currentCursor)) {
      lastCursor = currentCursor;
      hasLastCursor = true;
    }
#else
    if (cursorOnlyFrame) {
      continue;
    }
#endif

#ifdef _WIN32
    if (hasPendingMediaEpochKeyframe) {
      bool forced = encoder->requestKeyframe();
      bool restarted = false;
      if (!forced) {
        const auto now = std::chrono::steady_clock::now();
        const bool enoughCooldown = lastEncoderRestartAt.time_since_epoch().count() == 0
          || now - lastEncoderRestartAt > std::chrono::seconds(2);
        if (enoughCooldown && restartEncoder(currentAdaptiveBitrate, "media-epoch")) {
          lastEncoderRestartAt = now;
          restarted = true;
          forced = true;
        }
      }
      seenMediaCryptoGeneration = pendingMediaGeneration;
      std::cerr << "SNV1_KEYFRAME_APPLY reason=media-epoch"
                << " mediaEpoch=" << pendingMediaEpoch
                << " mediaGeneration=" << pendingMediaGeneration
                << " forced=" << (forced ? "yes" : "no")
                << " restarted=" << (restarted ? "yes" : "no")
                << "\n";
    }
#endif

    if (hasPendingKeyframeRequest) {
      bool forced = encoder->requestKeyframe();
      bool restarted = false;
      if (!forced) {
        const auto now = std::chrono::steady_clock::now();
        const bool enoughCooldown = lastEncoderRestartAt.time_since_epoch().count() == 0
          || now - lastEncoderRestartAt > std::chrono::seconds(2);
        if (enoughCooldown && restartEncoder(currentAdaptiveBitrate, "keyframe-request")) {
          lastEncoderRestartAt = now;
          restarted = true;
          forced = true;
        }
      }
      seenKeyframeRequestSequence = pendingKeyframeRequest.sequence;
      std::cerr << "SNV1_KEYFRAME_APPLY sequence=" << pendingKeyframeRequest.sequence
                << " reason=" << pendingKeyframeRequest.reason
                << " videoSequence=" << pendingKeyframeRequest.videoSequence
                << " forced=" << (forced ? "yes" : "no")
                << " restarted=" << (restarted ? "yes" : "no")
                << "\n";
    }

    auto packets = encoder->encodeFrame(frame);
    for (const auto& packet : packets) {
      const auto streamPacket = normalizePacketTimestamp(
        packet,
        currentStreamDurationMicros());
      const auto bytes = makeEncodedPacketBytes(streamPacket,
                                                duplicator.width(),
                                                duplicator.height(),
                                                sequence++,
                                                encoder->usingHardware());
      ++statsPackets;
      statsBytes += bytes.size();
      if (packet.keyframe) ++statsKeyframes;
#ifdef _WIN32
      if (tcpClient) {
        tcpClient->sendAll(bytes.data(), bytes.size());
      } else if (udpClient) {
        statsUdpFragments += udpClient->sendPacket(bytes);
      } else
#endif
      {
        output->write(reinterpret_cast<const char*>(bytes.data()), bytes.size());
      }
    }
#ifdef _WIN32
    if (!tcpClient && !udpClient)
#endif
    {
      output->flush();
    }
    ++capturedFrames;
    ++statsFrames;

    const auto statsNow = std::chrono::steady_clock::now();
    const double statsSeconds = std::chrono::duration<double>(statsNow - statsStartedAt).count();
    if (statsSeconds >= 1.0) {
      ++statsFeedbackWindowSequence;
      auto renderRecoveryHoldActive = [&]() {
        return renderRecoveryHoldWindows > 0 ||
          (renderRecoveryHoldConsumedWindow != 0 &&
           renderRecoveryHoldConsumedWindow == statsFeedbackWindowSequence);
      };
      auto renderRecoveryHoldWindowsForLog = [&]() {
        return std::max<std::uint32_t>(renderRecoveryHoldWindows, 1);
      };
      auto consumeRenderRecoveryHoldWindow = [&]() {
        if (renderRecoveryHoldWindows == 0 ||
            renderRecoveryHoldConsumedWindow == statsFeedbackWindowSequence) {
          return;
        }
        --renderRecoveryHoldWindows;
        renderRecoveryHoldConsumedWindow = statsFeedbackWindowSequence;
      };
      auto feedbackAgeWindows = [&](std::uint64_t lastWindow) -> std::uint64_t {
        return lastWindow == 0 || statsFeedbackWindowSequence < lastWindow
          ? 0
          : statsFeedbackWindowSequence - lastWindow;
      };
      auto streamFeedbackFresh = [&]() {
        return seenFeedbackSequence == 0 ||
          (lastStreamFeedbackWindow != 0 &&
           feedbackAgeWindows(lastStreamFeedbackWindow) <= feedbackFreshWindowLimit);
      };
      auto udpRepairFeedbackFresh = [&]() {
        return options.udpConnect.empty() ||
          seenUdpRepairFeedbackSequence == 0 ||
          (lastUdpRepairFeedbackWindow != 0 &&
           feedbackAgeWindows(lastUdpRepairFeedbackWindow) <= feedbackFreshWindowLimit);
      };
      const bool streamFeedbackStale = seenFeedbackSequence != 0 && !streamFeedbackFresh();
      const bool udpRepairFeedbackStale = !options.udpConnect.empty() &&
                                          seenUdpRepairFeedbackSequence != 0 &&
                                          !udpRepairFeedbackFresh();
      if (streamFeedbackStale) {
        clearFeedbackWindows = 0;
        clearFrameFeedbackWindows = 0;
      }
      if (udpRepairFeedbackStale) {
        clearUdpRepairWindows = 0;
      }
      if ((streamFeedbackStale || udpRepairFeedbackStale) &&
          (lastFeedbackStaleLogWindow == 0 ||
           statsFeedbackWindowSequence - lastFeedbackStaleLogWindow >= feedbackStaleLogIntervalWindows)) {
        lastFeedbackStaleLogWindow = statsFeedbackWindowSequence;
        std::cerr << "SNV1_FEEDBACK_STALE"
                  << " stream=" << (streamFeedbackStale ? "yes" : "no")
                  << " streamAgeWindows=" << feedbackAgeWindows(lastStreamFeedbackWindow)
                  << " udpRepair=" << (udpRepairFeedbackStale ? "yes" : "no")
                  << " udpRepairAgeWindows=" << feedbackAgeWindows(lastUdpRepairFeedbackWindow)
                  << " streamClearWindows=" << clearFeedbackWindows
                  << " udpClearWindows=" << clearUdpRepairWindows
                  << "\n";
      }
      auto streamFeedbackClearEnough = [&]() {
        return seenFeedbackSequence == 0 ||
          (streamFeedbackFresh() && clearFeedbackWindows >= recoveryCrossClearWindows);
      };
      auto udpRepairClearEnough = [&]() {
        return options.udpConnect.empty() ||
          seenUdpRepairFeedbackSequence == 0 ||
          (udpRepairFeedbackFresh() && clearUdpRepairWindows >= recoveryCrossClearWindows);
      };
      auto recoveryGateOpen = [&](bool fromUdpRepair) {
        if (renderRecoveryHoldActive()) return false;
        if (recoveryIncreaseAppliedWindow == statsFeedbackWindowSequence) return false;
        return fromUdpRepair ? streamFeedbackClearEnough() : udpRepairClearEnough();
      };
      auto recoveryGateReason = [&](bool fromUdpRepair) -> const char* {
        if (renderRecoveryHoldActive()) return "render-recovery";
        if (recoveryIncreaseAppliedWindow == statsFeedbackWindowSequence) return "window-increase-used";
        if (fromUdpRepair && !streamFeedbackFresh()) return "stream-feedback-stale";
        if (!fromUdpRepair && !udpRepairFeedbackFresh()) return "udp-repair-feedback-stale";
        if (fromUdpRepair && !streamFeedbackClearEnough()) return "await-stream-clear";
        if (!fromUdpRepair && !udpRepairClearEnough()) return "await-udp-repair-clear";
        return "unknown";
      };
      auto logRecoveryGate = [&](const char* source, bool fromUdpRepair) {
        std::cerr << "SNV1_RECOVERY_GATE source=" << source
                  << " reason=" << recoveryGateReason(fromUdpRepair)
                  << " udpClearWindows=" << clearUdpRepairWindows
                  << " streamClearWindows=" << clearFeedbackWindows
                  << " frameClearWindows=" << clearFrameFeedbackWindows
                  << " renderHoldWindows=" << renderRecoveryHoldWindows
                  << " streamAgeWindows=" << feedbackAgeWindows(lastStreamFeedbackWindow)
                  << " udpRepairAgeWindows=" << feedbackAgeWindows(lastUdpRepairFeedbackWindow)
                  << " bitrateMbps=" << (static_cast<double>(currentAdaptiveBitrate) / 1000000.0)
                  << " fps=" << currentAdaptiveFps
                  << "\n";
      };
      const double fps = static_cast<double>(statsFrames) / statsSeconds;
      const double mbps = (static_cast<double>(statsBytes) * 8.0) / statsSeconds / 1000000.0;
#ifdef _WIN32
      if (udpClient) {
        statsUdpPacedMs += udpClient->takePacedSleepMs();
        const auto cacheStats = udpClient->takeRetransmitCacheStats();
        statsUdpCacheResets += cacheStats.resets;
        statsUdpCacheDropped += cacheStats.droppedPackets;
      }
#endif
      std::cerr << "SNV1_STATS fps=" << std::fixed << std::setprecision(1) << fps
                << " targetFps=" << currentAdaptiveFps
                << " requestedFps=" << requestedFps
                << " frameDelayMs=" << (static_cast<double>(currentStreamDurationMicros()) / 1000.0)
                << " mbps=" << mbps
                << " packets=" << statsPackets
                << " keyframes=" << statsKeyframes
#ifdef _WIN32
                << " udpFragments=" << statsUdpFragments
                << " udpPacedMs=" << statsUdpPacedMs
                << " cursorFrames=" << statsCursorFrames
                << " refreshFrames=" << statsRefreshFrames
                << " udpNackReq=" << statsUdpNackRequests
                << " udpNackMiss=" << statsUdpNackMisses
                << " udpRetransmits=" << statsUdpRetransmits
                << " udpRetransmitFragments=" << statsUdpRetransmitFragments
                << " udpCacheReset=" << statsUdpCacheResets
                << " udpCacheDropped=" << statsUdpCacheDropped
#endif
                << "\n";

      const auto repairFeedback = latestUdpRepairFeedback();
      if (repairFeedback.sequence != 0 && repairFeedback.sequence != seenUdpRepairFeedbackSequence) {
        seenUdpRepairFeedbackSequence = repairFeedback.sequence;
        lastUdpRepairFeedbackWindow = statsFeedbackWindowSequence;
        std::uint32_t nextBitrate = currentAdaptiveBitrate;
        std::uint32_t nextFps = currentAdaptiveFps;
        if (repairFeedback.pressure >= 2) {
          clearUdpRepairWindows = 0;
          clearFeedbackWindows = 0;
          clearFrameFeedbackWindows = 0;
          nextBitrate = static_cast<std::uint32_t>(static_cast<double>(currentAdaptiveBitrate) * 0.76);
          if (adaptiveFramePacing) {
            nextFps = static_cast<std::uint32_t>(
              std::floor(static_cast<double>(currentAdaptiveFps) * 0.84));
          }
        } else if (repairFeedback.pressure == 1) {
          clearUdpRepairWindows = 0;
          clearFeedbackWindows = 0;
          clearFrameFeedbackWindows = 0;
          nextBitrate = static_cast<std::uint32_t>(static_cast<double>(currentAdaptiveBitrate) * 0.90);
          if (adaptiveFramePacing) {
            nextFps = static_cast<std::uint32_t>(
              std::floor(static_cast<double>(currentAdaptiveFps) * 0.95));
          }
        } else if (repairFeedback.pressure < 0) {
          if (renderRecoveryHoldActive()) {
            clearUdpRepairWindows = 0;
            std::cerr << "SNV1_ADAPT_HOLD source=udp-repair"
                      << " reason=render-recovery"
                      << " holdWindows=" << renderRecoveryHoldWindowsForLog()
                      << " bitrateMbps=" << (static_cast<double>(currentAdaptiveBitrate) / 1000000.0)
                      << " fps=" << currentAdaptiveFps
                      << "\n";
            consumeRenderRecoveryHoldWindow();
          } else {
            clearUdpRepairWindows += 1;
            const bool wantsBitrateRecovery = clearUdpRepairWindows >= udpRepairBitrateClearWindows;
            const bool wantsFrameRecovery = adaptiveFramePacing &&
                                            clearUdpRepairWindows >= udpRepairFrameClearWindows;
            if (wantsBitrateRecovery || wantsFrameRecovery) {
              if (recoveryGateOpen(true)) {
                if (wantsBitrateRecovery) {
                  nextBitrate = static_cast<std::uint32_t>(static_cast<double>(currentAdaptiveBitrate) * 1.05);
                }
                if (wantsFrameRecovery) {
                  const std::uint32_t step = std::max<std::uint32_t>(1, maxAdaptiveFps / 15);
                  nextFps = std::min<std::uint32_t>(maxAdaptiveFps, currentAdaptiveFps + step);
                  clearUdpRepairWindows = 0;
                }
                recoveryIncreaseAppliedWindow = statsFeedbackWindowSequence;
              } else {
                logRecoveryGate("udp-repair", true);
                clearUdpRepairWindows = std::min(clearUdpRepairWindows, udpRepairFrameClearWindows);
              }
            }
          }
        } else {
          clearUdpRepairWindows = 0;
        }

        if (adaptiveFramePacing) {
          nextFps = std::clamp(nextFps, minAdaptiveFps, maxAdaptiveFps);
          if (nextFps != currentAdaptiveFps) {
            currentAdaptiveFps = nextFps;
            adaptiveFrameDelay = frameDelayForFps(currentAdaptiveFps);
            adaptiveCaptureTimeoutMs = captureTimeoutForFps(currentAdaptiveFps);
            std::cerr << "SNV1_FRAME_ADAPT source=udp-repair"
                      << " pressure=" << pressureLabel(repairFeedback.pressure)
                      << " currentFps=" << currentAdaptiveFps
                      << " minFps=" << minAdaptiveFps
                      << " maxFps=" << maxAdaptiveFps
                      << " frameDelayMs=" << std::fixed << std::setprecision(1)
                      << (static_cast<double>(adaptiveFrameDelay.count()) / 1000.0)
                      << " captureTimeoutMs=" << adaptiveCaptureTimeoutMs
                      << " nackSent=" << repairFeedback.nackSent
                      << " nackTimedOut=" << repairFeedback.nackTimedOut
                      << " jitterSkipped=" << repairFeedback.jitterSkipped
                      << "\n";
          }
        }

        nextBitrate = std::clamp(nextBitrate, minAdaptiveBitrate, maxAdaptiveBitrate);
        const std::uint32_t repairDelta = nextBitrate > currentAdaptiveBitrate
          ? nextBitrate - currentAdaptiveBitrate
          : currentAdaptiveBitrate - nextBitrate;
        if (repairDelta >= 250000) {
          bool applied = encoder->setBitrate(nextBitrate);
          bool restarted = false;
          if (applied) {
            currentAdaptiveBitrate = encoder->bitrate();
          } else {
            const bool enoughCooldown = lastEncoderRestartAt.time_since_epoch().count() == 0
              || statsNow - lastEncoderRestartAt > std::chrono::seconds(3);
            if (enoughCooldown && restartEncoder(nextBitrate, "udp-repair-adapt")) {
              currentAdaptiveBitrate = nextBitrate;
              lastEncoderRestartAt = statsNow;
              restarted = true;
              applied = true;
            }
          }
#ifdef _WIN32
          if (applied && udpClient) {
            udpClient->setBitrate(currentAdaptiveBitrate);
          }
#endif
          std::cerr << "SNV1_ADAPT source=udp-repair"
                    << " pressure=" << pressureLabel(repairFeedback.pressure)
                    << " requestedMbps=" << (static_cast<double>(nextBitrate) / 1000000.0)
                    << " currentMbps=" << (static_cast<double>(currentAdaptiveBitrate) / 1000000.0)
                    << " applied=" << (applied ? "yes" : "no")
                    << " restarted=" << (restarted ? "yes" : "no")
                    << " nackSent=" << repairFeedback.nackSent
                    << " nackRecovered=" << repairFeedback.nackRecovered
                    << " nackTimedOut=" << repairFeedback.nackTimedOut
                    << " nackPending=" << repairFeedback.nackPending
                    << " retransmitCompleted=" << repairFeedback.retransmitCompleted
                    << " jitterSkipped=" << repairFeedback.jitterSkipped
                    << " malformed=" << repairFeedback.malformed
                    << "\n";
        }
      }

      const auto feedback = latestStreamFeedback();
      if (feedback.sequence != 0 && feedback.sequence != seenFeedbackSequence) {
        seenFeedbackSequence = feedback.sequence;
        lastStreamFeedbackWindow = statsFeedbackWindowSequence;
        std::uint32_t nextBitrate = currentAdaptiveBitrate;
        std::uint32_t nextFps = currentAdaptiveFps;
        const bool hasRenderFeedback = feedback.source == "render" || feedback.rendered > 0;
        if (hasRenderFeedback &&
            (feedback.renderAdaptiveUp > 0 || feedback.renderAdaptiveDelayMs >= 10.0)) {
          const std::uint32_t holdWindows = feedback.renderAdaptiveDelayMs >= 16.0 ? 5 : 3;
          renderRecoveryHoldWindows = std::max(renderRecoveryHoldWindows, holdWindows);
        }
        if (feedback.pressure >= 2) {
          clearFeedbackWindows = 0;
          clearFrameFeedbackWindows = 0;
          nextBitrate = static_cast<std::uint32_t>(static_cast<double>(currentAdaptiveBitrate) * 0.72);
          if (adaptiveFramePacing) {
            nextFps = static_cast<std::uint32_t>(
              std::floor(static_cast<double>(currentAdaptiveFps) * 0.80));
          }
        } else if (feedback.pressure == 1) {
          clearFeedbackWindows = 0;
          clearFrameFeedbackWindows = 0;
          nextBitrate = static_cast<std::uint32_t>(static_cast<double>(currentAdaptiveBitrate) * 0.88);
          if (adaptiveFramePacing) {
            nextFps = static_cast<std::uint32_t>(
              std::floor(static_cast<double>(currentAdaptiveFps) * 0.92));
          }
        } else if (feedback.pressure < 0) {
          if (hasRenderFeedback && renderRecoveryHoldActive()) {
            std::cerr << "SNV1_ADAPT_HOLD source=" << feedback.source
                      << " reason=render-recovery"
                      << " holdWindows=" << renderRecoveryHoldWindowsForLog()
                      << " adaptiveDelayMs=" << feedback.renderAdaptiveDelayMs
                      << " adaptiveUp=" << feedback.renderAdaptiveUp
                      << " adaptiveDown=" << feedback.renderAdaptiveDown
                      << " bitrateMbps=" << (static_cast<double>(currentAdaptiveBitrate) / 1000000.0)
                      << " fps=" << currentAdaptiveFps
                      << "\n";
            consumeRenderRecoveryHoldWindow();
            clearFeedbackWindows = 0;
            clearFrameFeedbackWindows = 0;
          } else {
            clearFeedbackWindows += 1;
            clearFrameFeedbackWindows += 1;
            const bool wantsBitrateRecovery = clearFeedbackWindows >= streamBitrateClearWindows;
            const bool wantsFrameRecovery = adaptiveFramePacing &&
                                            clearFrameFeedbackWindows >= streamFrameClearWindows;
            if (wantsBitrateRecovery || wantsFrameRecovery) {
              if (recoveryGateOpen(false)) {
                if (wantsBitrateRecovery) {
                  nextBitrate = static_cast<std::uint32_t>(static_cast<double>(currentAdaptiveBitrate) * 1.08);
                  clearFeedbackWindows = 0;
                }
                if (wantsFrameRecovery) {
                  const std::uint32_t step = std::max<std::uint32_t>(1, maxAdaptiveFps / 12);
                  nextFps = std::min<std::uint32_t>(maxAdaptiveFps, currentAdaptiveFps + step);
                  clearFrameFeedbackWindows = 0;
                }
                recoveryIncreaseAppliedWindow = statsFeedbackWindowSequence;
              } else {
                logRecoveryGate(feedback.source.c_str(), false);
                clearFeedbackWindows = std::min(clearFeedbackWindows, streamBitrateClearWindows);
                clearFrameFeedbackWindows = std::min(clearFrameFeedbackWindows, streamFrameClearWindows);
              }
            }
          }
        } else {
          clearFeedbackWindows = 0;
          clearFrameFeedbackWindows = 0;
        }

        if (adaptiveFramePacing) {
          nextFps = std::clamp(nextFps, minAdaptiveFps, maxAdaptiveFps);
          if (nextFps != currentAdaptiveFps) {
            currentAdaptiveFps = nextFps;
            adaptiveFrameDelay = frameDelayForFps(currentAdaptiveFps);
            adaptiveCaptureTimeoutMs = captureTimeoutForFps(currentAdaptiveFps);
            std::cerr << "SNV1_FRAME_ADAPT source=" << feedback.source
                      << " pressure=" << pressureLabel(feedback.pressure)
                      << " currentFps=" << currentAdaptiveFps
                      << " minFps=" << minAdaptiveFps
                      << " maxFps=" << maxAdaptiveFps
                      << " frameDelayMs=" << std::fixed << std::setprecision(1)
                      << (static_cast<double>(adaptiveFrameDelay.count()) / 1000.0)
                      << " captureTimeoutMs=" << adaptiveCaptureTimeoutMs;
            if (feedback.source == "render" || feedback.rendered > 0) {
              std::cerr << " renderQueue=" << feedback.renderQueueDepth
                        << " renderDropQ=" << feedback.renderDroppedQueue
                        << " renderDropLate=" << feedback.renderDroppedLate
                        << " renderAdaptiveDelayMs=" << feedback.renderAdaptiveDelayMs
                        << " renderMaxLateMs=" << feedback.renderMaxPresentLateMs;
            }
            std::cerr << "\n";
          }
        }

        nextBitrate = std::clamp(nextBitrate, minAdaptiveBitrate, maxAdaptiveBitrate);
        const std::uint32_t delta = nextBitrate > currentAdaptiveBitrate
          ? nextBitrate - currentAdaptiveBitrate
          : currentAdaptiveBitrate - nextBitrate;
        if (delta >= 250000) {
          bool applied = encoder->setBitrate(nextBitrate);
          bool restarted = false;
          if (applied) {
            currentAdaptiveBitrate = encoder->bitrate();
          } else {
            const bool enoughCooldown = lastEncoderRestartAt.time_since_epoch().count() == 0
              || statsNow - lastEncoderRestartAt > std::chrono::seconds(3);
            if (enoughCooldown && restartEncoder(nextBitrate, "bitrate-adapt")) {
              currentAdaptiveBitrate = nextBitrate;
              lastEncoderRestartAt = statsNow;
              restarted = true;
              applied = true;
            }
          }
#ifdef _WIN32
          if (applied && udpClient) {
            udpClient->setBitrate(currentAdaptiveBitrate);
          }
#endif
          std::cerr << "SNV1_ADAPT source=" << feedback.source
                    << " pressure=" << pressureLabel(feedback.pressure)
                    << " requestedMbps=" << (static_cast<double>(nextBitrate) / 1000000.0)
                    << " currentMbps=" << (static_cast<double>(currentAdaptiveBitrate) / 1000000.0)
                    << " applied=" << (applied ? "yes" : "no")
                    << " restarted=" << (restarted ? "yes" : "no")
                    << " jitterMs=" << feedback.jitterMs
                    << " avgAgeMs=" << feedback.avgAgeMs
                    << " dropped=" << feedback.dropped;
          if (feedback.source == "render" || feedback.rendered > 0) {
            std::cerr << " renderQueue=" << feedback.renderQueueDepth
                      << " renderDropQ=" << feedback.renderDroppedQueue
                      << " renderDropLate=" << feedback.renderDroppedLate
                      << " renderAdaptiveDelayMs=" << feedback.renderAdaptiveDelayMs
                      << " renderAvgLateMs=" << feedback.renderAvgPresentLateMs
                      << " renderMaxLateMs=" << feedback.renderMaxPresentLateMs;
          }
          std::cerr << "\n";
        }
      }

      statsStartedAt = statsNow;
      statsFrames = 0;
      statsPackets = 0;
      statsBytes = 0;
      statsUdpFragments = 0;
      statsUdpPacedMs = 0.0;
      statsKeyframes = 0;
#ifdef _WIN32
      statsCursorFrames = 0;
      statsRefreshFrames = 0;
      statsUdpNackRequests = 0;
      statsUdpNackMisses = 0;
      statsUdpRetransmits = 0;
      statsUdpRetransmitFragments = 0;
      statsUdpCacheResets = 0;
      statsUdpCacheDropped = 0;
#endif
    }

    if (options.intervalMs > 0) {
      std::this_thread::sleep_for(std::chrono::milliseconds(options.intervalMs));
    } else if (adaptiveFrameDelay.count() > 0) {
      std::this_thread::sleep_until(frameStartedAt + adaptiveFrameDelay);
    }
  }

  auto packets = encoder->finish();
  for (const auto& packet : packets) {
    const auto streamPacket = normalizePacketTimestamp(
      packet,
      currentStreamDurationMicros());
    const auto bytes = makeEncodedPacketBytes(streamPacket,
                                              duplicator.width(),
                                              duplicator.height(),
                                              sequence++,
                                              encoder->usingHardware());
#ifdef _WIN32
    if (tcpClient) {
      tcpClient->sendAll(bytes.data(), bytes.size());
    } else if (udpClient) {
      udpClient->sendPacket(bytes);
    } else
#endif
    {
      output->write(reinterpret_cast<const char*>(bytes.data()), bytes.size());
    }
  }
#ifdef _WIN32
  if (!tcpClient && !udpClient)
#endif
  {
    output->flush();
  }

  std::cerr << "SNV1 packet stream wrote " << sequence << " packet(s).\n";
  return 0;
}

int main(int argc, char** argv) {
  try {
    makeProcessDpiAware();
    Options options = parseOptions(argc, argv);
    if (options.sessionToken.empty()) {
      if (const char* token = std::getenv("SANSER_NATIVE_SESSION_TOKEN")) {
        options.sessionToken = token;
      }
    }

    if (options.listEncoders) {
      listHardwareVideoEncoders(std::cout);
      return 0;
    }

    DesktopDuplicator duplicator;
    duplicator.initialize(options.adapterIndex, options.outputIndex);
    {
      std::lock_guard<std::mutex> lock(gInputBoundsMutex);
      gInputBounds = {
        static_cast<int>(duplicator.left()),
        static_cast<int>(duplicator.top()),
        static_cast<int>(duplicator.width()),
        static_cast<int>(duplicator.height())
      };
    }
    std::cerr << "SNINPUT target bounds left=" << gInputBounds.left
              << " top=" << gInputBounds.top
              << " width=" << gInputBounds.width
              << " height=" << gInputBounds.height
              << "\n";

    if (options.pipe) {
      return runPipeMode(duplicator, options);
    }

    if (options.encodePipe) {
      return runEncodedPipeMode(duplicator, options);
    }

    if (options.encode) {
      return runEncodeMode(duplicator, options);
    }

    std::filesystem::create_directories(options.outputDir);

    std::cout << "Desktop Duplication initialized: "
              << duplicator.width() << "x" << duplicator.height() << "\n";

    std::uint32_t saved = 0;
    while (saved < options.frames) {
      FrameBgra frame;
      if (!duplicator.captureFrame(frame, 1000)) {
        std::cout << "Timed out waiting for a changed frame.\n";
        continue;
      }

      auto file = options.outputDir / ("frame_" + std::to_string(saved) + ".bmp");
      writeBmp(file, frame);
      std::cout << "Wrote " << file.string() << "\n";
      ++saved;

      if (options.intervalMs > 0) {
        std::this_thread::sleep_for(std::chrono::milliseconds(options.intervalMs));
      }
    }

    return 0;
  } catch (const std::exception& error) {
    std::cerr << "sanser-native-host error: " << error.what() << "\n";
    return 1;
  }
}
