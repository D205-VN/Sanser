#import <Cocoa/Cocoa.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Metal/Metal.h>
#import <MetalKit/MetalKit.h>
#import <VideoToolbox/VideoToolbox.h>
#import <dispatch/dispatch.h>

#include <chrono>
#include <algorithm>
#include <array>
#include <cerrno>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <memory>
#include <mutex>
#include <sstream>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

#include <arpa/inet.h>
#include <netinet/in.h>
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
    << "sanser-native-client --listen-render-snv PORT [--max-packets N] [--log-input] [--fullscreen] [--hide-cursor]\n"
    << "sanser-native-client --metal-test [--seconds 5]\n"
    << "sanser-native-client --clipboard-read\n"
    << "sanser-native-client --clipboard-write \"text\"\n"
    << "\n"
    << "Phase 5 prototype:\n"
    << "  --probe           Print VideoToolbox hardware decode and Metal availability\n"
    << "  --decode-snv P    Decode an SNV1 H.264 packet file with VideoToolbox\n"
    << "  --listen-snv P    Listen for SNV1 H.264 packets over TCP and decode them\n"
    << "  --listen-render-snv P Listen over TCP, render with Metal, send native input back\n"
    << "  --max-packets N   Stop SNV decode after N packets\n"
    << "  --log-input       Print each native input event for debugging\n"
    << "  --fullscreen      Open the Metal renderer fullscreen\n"
    << "  --hide-cursor     Hide the local cursor while the renderer is open\n"
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

void writeLe32Raw(std::uint8_t* target, std::uint32_t value) {
  target[0] = static_cast<std::uint8_t>(value & 0xff);
  target[1] = static_cast<std::uint8_t>((value >> 8) & 0xff);
  target[2] = static_cast<std::uint8_t>((value >> 16) & 0xff);
  target[3] = static_cast<std::uint8_t>((value >> 24) & 0xff);
}

class NativeInputSender {
public:
  void setFd(int fd) {
    std::lock_guard<std::mutex> lock(mutex_);
    fd_ = fd;
  }

  void clearFd(int fd) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (fd_ == fd) fd_ = -1;
  }

  bool sendJson(const std::string& json) {
    std::array<std::uint8_t, 12> header{};
    std::memcpy(header.data(), "SNI1", 4);
    writeLe32Raw(header.data() + 4, static_cast<std::uint32_t>(header.size()));
    writeLe32Raw(header.data() + 8, static_cast<std::uint32_t>(json.size()));

    std::lock_guard<std::mutex> lock(mutex_);
    if (fd_ < 0) return false;
    if (!sendAllLocked(header.data(), header.size())) {
      fd_ = -1;
      return false;
    }
    if (!json.empty() && !sendAllLocked(json.data(), json.size())) {
      fd_ = -1;
      return false;
    }
    return true;
  }

private:
  bool sendAllLocked(const void* data, std::size_t size) {
    const auto* cursor = static_cast<const std::uint8_t*>(data);
    std::size_t remaining = size;
    while (remaining > 0) {
      const ssize_t sent = send(fd_, cursor, remaining, 0);
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
  int fd_ = -1;
};

std::shared_ptr<NativeInputSender> gNativeInputSender;
bool gLogInputEvents = false;

bool sendNativeInputJson(const std::string& json) {
  const bool sent = gNativeInputSender && gNativeInputSender->sendJson(json);
  if (gLogInputEvents) {
    std::cout << (sent ? "SNINPUT_TX " : "SNINPUT_LOCAL ") << json << "\n";
  }
  return sent;
}

std::string pointerEventJson(const char* type, NSEvent* event, NSView* view) {
  const NSPoint point = [view convertPoint:[event locationInWindow] fromView:nil];
  const NSRect bounds = [view bounds];
  const double width = std::max<double>(bounds.size.width, 1.0);
  const double height = std::max<double>(bounds.size.height, 1.0);
  const double x = std::clamp(point.x / width, 0.0, 1.0);
  const double y = std::clamp(1.0 - (point.y / height), 0.0, 1.0);
  std::ostringstream out;
  out << "{\"type\":\"" << type
      << "\",\"x\":" << x
      << ",\"y\":" << y
      << ",\"button\":" << [event buttonNumber]
      << ",\"dx\":" << [event scrollingDeltaX]
      << ",\"dy\":" << [event scrollingDeltaY]
      << "}";
  return out.str();
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
  std::vector<std::uint8_t> payload;
};

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
  using FrameCallback = void (*)(void* context, CVImageBufferRef imageBuffer);

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

    VTDecodeInfoFlags infoFlags = 0;
    status = VTDecompressionSessionDecodeFrame(session_, sampleBuffer, 0, nullptr, &infoFlags);
    CFRelease(sampleBuffer);
    if (status != noErr) {
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

    checkStatus(VTDecompressionSessionCreate(kCFAllocatorDefault,
                                             format_,
                                             nullptr,
                                             (__bridge CFDictionaryRef)attributes,
                                             &callback,
                                             &session_),
                "VTDecompressionSessionCreate");
  }

  static void outputCallback(void* decompressionOutputRefCon,
                             void* sourceFrameRefCon,
                             OSStatus status,
                             VTDecodeInfoFlags infoFlags,
                             CVImageBufferRef imageBuffer,
                             CMTime presentationTimeStamp,
                             CMTime presentationDuration) {
    (void)sourceFrameRefCon;
    (void)infoFlags;
    (void)presentationTimeStamp;
    (void)presentationDuration;
    auto* self = static_cast<VtH264Decoder*>(decompressionOutputRefCon);
    if (status == noErr && imageBuffer) {
      self->decodedFrames_ += 1;
      if (self->frameCallback_) {
        self->frameCallback_(self->frameCallbackContext_, imageBuffer);
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
  }

  const std::uint32_t payloadSize = readLe32Raw(header.data() + 48);
  packet.payload.resize(payloadSize);
  if (payloadSize > 0 && !readExactFd(fd, packet.payload.data(), packet.payload.size())) {
    throw std::runtime_error("TCP stream ended inside SNV1 payload.");
  }
  return true;
}

class ScopedFd {
public:
  explicit ScopedFd(int fd = -1) : fd_(fd) {}
  ~ScopedFd() {
    if (fd_ >= 0) close(fd_);
  }

  ScopedFd(const ScopedFd&) = delete;
  ScopedFd& operator=(const ScopedFd&) = delete;

  int get() const { return fd_; }
  int release() {
    const int fd = fd_;
    fd_ = -1;
    return fd;
  }

private:
  int fd_ = -1;
};

int listenSnvTcp(std::uint16_t port, std::uint64_t maxPackets) {
  @autoreleasepool {
    ScopedFd server(socket(AF_INET, SOCK_STREAM, 0));
    if (server.get() < 0) {
      throw std::runtime_error("Could not create TCP listener socket.");
    }

    int reuse = 1;
    setsockopt(server.get(), SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));

    sockaddr_in address{};
    address.sin_family = AF_INET;
    address.sin_addr.s_addr = htonl(INADDR_ANY);
    address.sin_port = htons(port);
    if (bind(server.get(), reinterpret_cast<sockaddr*>(&address), sizeof(address)) != 0) {
      throw std::runtime_error("Could not bind TCP listener on port " + std::to_string(port));
    }
    if (listen(server.get(), 1) != 0) {
      throw std::runtime_error("Could not listen on TCP port " + std::to_string(port));
    }

    std::cout << "SNV1 TCP listener ready on 0.0.0.0:" << port << "\n";
    std::cout << "Start Windows host with: --encode-pipe h264 --tcp-connect 100.100.83.44:" << port << "\n";

    sockaddr_in peerAddress{};
    socklen_t peerLength = sizeof(peerAddress);
    ScopedFd client(accept(server.get(), reinterpret_cast<sockaddr*>(&peerAddress), &peerLength));
    if (client.get() < 0) {
      throw std::runtime_error("TCP accept failed.");
    }

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

@interface SanserMetalView : MTKView
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
  [[self window] makeFirstResponder:self];
  [[self window] setAcceptsMouseMovedEvents:YES];
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

@interface SanserVideoRenderer : NSObject <MTKViewDelegate>
- (instancetype)initWithDevice:(id<MTLDevice>)device view:(MTKView*)view;
- (void)submitPixelBuffer:(CVPixelBufferRef)pixelBuffer;
@end

@implementation SanserVideoRenderer {
  id<MTLDevice> _device;
  id<MTLCommandQueue> _commandQueue;
  id<MTLRenderPipelineState> _pipeline;
  CVMetalTextureCacheRef _textureCache;
  CVPixelBufferRef _latestPixelBuffer;
  NSLock* _lock;
  std::uint64_t _submittedBuffers;
}

- (instancetype)initWithDevice:(id<MTLDevice>)device view:(MTKView*)view {
  self = [super init];
  if (!self) return nil;

  _device = device;
  _commandQueue = [device newCommandQueue];
  _lock = [[NSLock alloc] init];

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

  return self;
}

- (void)dealloc {
  [_lock lock];
  if (_latestPixelBuffer) {
    CVPixelBufferRelease(_latestPixelBuffer);
    _latestPixelBuffer = nullptr;
  }
  [_lock unlock];
  if (_textureCache) {
    CFRelease(_textureCache);
    _textureCache = nullptr;
  }
}

- (void)submitPixelBuffer:(CVPixelBufferRef)pixelBuffer {
  if (!pixelBuffer) return;
  CVPixelBufferRetain(pixelBuffer);
  [_lock lock];
  if (_latestPixelBuffer) {
    CVPixelBufferRelease(_latestPixelBuffer);
  }
  _latestPixelBuffer = pixelBuffer;
  _submittedBuffers += 1;
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
  if (_latestPixelBuffer) {
    pixelBuffer = CVPixelBufferRetain(_latestPixelBuffer);
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
}

@end

namespace {

void submitFrameToVideoRenderer(void* context, CVImageBufferRef imageBuffer) {
  auto* renderer = (__bridge SanserVideoRenderer*)context;
  [renderer submitPixelBuffer:(CVPixelBufferRef)imageBuffer];
}

void decodeTcpStreamToRenderer(std::uint16_t port,
                               std::uint64_t maxPackets,
                               void* retainedRendererContext,
                               std::shared_ptr<NativeInputSender> inputSender) {
  @autoreleasepool {
    try {
      ScopedFd server(socket(AF_INET, SOCK_STREAM, 0));
      if (server.get() < 0) {
        throw std::runtime_error("Could not create TCP listener socket.");
      }

      int reuse = 1;
      setsockopt(server.get(), SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));

      sockaddr_in address{};
      address.sin_family = AF_INET;
      address.sin_addr.s_addr = htonl(INADDR_ANY);
      address.sin_port = htons(port);
      if (bind(server.get(), reinterpret_cast<sockaddr*>(&address), sizeof(address)) != 0) {
        throw std::runtime_error("Could not bind TCP render listener on port " + std::to_string(port));
      }
      if (listen(server.get(), 1) != 0) {
        throw std::runtime_error("Could not listen on TCP render port " + std::to_string(port));
      }

      std::cout << "SNV1 Metal render listener ready on 0.0.0.0:" << port << "\n";
      std::cout << "Start Windows host with: --encode-pipe h264 --tcp-connect 100.100.83.44:" << port << "\n";

      sockaddr_in peerAddress{};
      socklen_t peerLength = sizeof(peerAddress);
      ScopedFd client(accept(server.get(), reinterpret_cast<sockaddr*>(&peerAddress), &peerLength));
      if (client.get() < 0) {
        throw std::runtime_error("TCP accept failed.");
      }
#ifdef SO_NOSIGPIPE
      int noSigPipe = 1;
      setsockopt(client.get(), SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, sizeof(noSigPipe));
#endif

      char peerIp[INET_ADDRSTRLEN]{};
      inet_ntop(AF_INET, &peerAddress.sin_addr, peerIp, sizeof(peerIp));
      std::cout << "SNV1 render client connected from " << peerIp << ":" << ntohs(peerAddress.sin_port) << "\n";
      if (inputSender) {
        inputSender->setFd(client.get());
        std::cout << "SNINPUT native control enabled on render TCP socket.\n";
      }

      VtH264Decoder decoder(submitFrameToVideoRenderer, retainedRendererContext);
      DecodeSummary summary;
      while (maxPackets == 0 || summary.packets < maxPackets) {
        SnvPacket packet;
        if (!readSnvPacketFromFd(client.get(), packet)) {
          break;
        }
        processSnvPacket(decoder, summary, packet);
        if (summary.packets % 60 == 0) {
          std::cout << "SNV1 render packets=" << summary.packets
                    << " decoded=" << decoder.decodedFrames()
                    << " errors=" << decoder.decodeErrors()
                    << "\n";
        }
      }

      decoder.flush();
      printDecodeSummary("tcp-render:" + std::to_string(port), summary, decoder);
      if (inputSender) {
        inputSender->clearFd(client.get());
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

int runVideoRenderTcp(std::uint16_t port, std::uint64_t maxPackets, bool fullscreen, bool hideCursor) {
  @autoreleasepool {
    id<MTLDevice> device = defaultMetalDevice();
    if (!device) {
      throw std::runtime_error("Metal is not available on this Mac.");
    }

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
    [view setPreferredFramesPerSecond:60];
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

    auto inputSender = std::make_shared<NativeInputSender>();
    gNativeInputSender = inputSender;

    void* rendererContext = (__bridge_retained void*)renderer;
    std::thread worker([port, maxPackets, rendererContext, inputSender]() {
      decodeTcpStreamToRenderer(port, maxPackets, rendererContext, inputSender);
    });
    worker.detach();

    std::cout << "Metal video renderer running on " << nsStringToUtf8([device name]) << "\n";
    std::cout << "Native renderer mode fullscreen=" << boolText(fullscreen)
              << " hideCursor=" << boolText(hideCursor) << "\n";
    [NSApp run];
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
  bool help = false;
  std::uint64_t maxPackets = 0;
  std::uint16_t listenPort = 0;
  std::uint16_t listenRenderPort = 0;
  double seconds = 5;
  std::string snvFile;
  std::string clipboardText;
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
    } else if (arg == "--metal-test") {
      options.metalTest = true;
    } else if (arg == "--log-input") {
      options.logInput = true;
    } else if (arg == "--fullscreen") {
      options.fullscreen = true;
    } else if (arg == "--hide-cursor") {
      options.hideCursor = true;
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

} // namespace

int main(int argc, char** argv) {
  try {
    const Options options = parseOptions(argc, argv);
    gLogInputEvents = options.logInput;
    if (options.help) {
      printHelp();
      return 0;
    }
    if (options.probe) return runProbe();
    if (options.decodeSnv) return decodeSnvFile(options.snvFile, options.maxPackets);
    if (options.listenSnv) return listenSnvTcp(options.listenPort, options.maxPackets);
    if (options.listenRenderSnv) return runVideoRenderTcp(options.listenRenderPort, options.maxPackets, options.fullscreen, options.hideCursor);
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
