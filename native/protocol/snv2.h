#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <span>
#include <string_view>

namespace sanser::snv2 {

inline constexpr std::array<std::uint8_t, 4> kMagic{'S', 'N', 'V', '2'};
inline constexpr std::uint8_t kProtocolVersion = 2;
inline constexpr std::size_t kHeaderSize = 80;
inline constexpr std::size_t kAuthenticatedHeaderSize = 64;
inline constexpr std::size_t kAuthTagSize = 16;
inline constexpr std::uint32_t kMaxPayloadSize = 1U << 20;

using SessionId = std::array<std::uint8_t, 16>;
using AuthTag = std::array<std::uint8_t, kAuthTagSize>;
using WireHeader = std::array<std::uint8_t, kHeaderSize>;

enum class PacketType : std::uint8_t {
  Handshake = 1,
  Authentication = 2,
  Video = 3,
  Audio = 4,
  MouseMove = 5,
  MouseButton = 6,
  Keyboard = 7,
  Gamepad = 8,
  Clipboard = 9,
  NetworkFeedback = 10,
  EncoderFeedback = 11,
  DecoderFeedback = 12,
  Nack = 13,
  KeyframeRequest = 14,
  Keepalive = 15,
  Disconnect = 16,
  Error = 17,
};

enum class Priority : std::uint8_t {
  ReliableInput = 1,
  RealtimeInput = 2,
  Audio = 3,
  VideoControl = 4,
  VideoPayload = 5,
  Diagnostics = 6,
};

struct Header {
  PacketType packetType = PacketType::Keepalive;
  Priority priority = Priority::VideoControl;
  std::uint16_t flags = 0;
  SessionId sessionId{};
  std::uint32_t streamId = 0;
  std::uint64_t sequence = 0;
  std::uint64_t frameId = 0;
  std::uint64_t timestampMicros = 0;
  std::uint32_t payloadLength = 0;
  std::uint32_t keyId = 0;
  AuthTag authTag{};
};

enum class DecodeError {
  None,
  Truncated,
  BadMagic,
  UnsupportedVersion,
  BadHeaderLength,
  ReservedBitsSet,
  UnknownPacketType,
  PriorityMismatch,
  EmptySession,
  PayloadTooLarge,
  TypePayloadTooLarge,
  LengthMismatch,
};

struct DecodeResult {
  Header header{};
  std::span<const std::uint8_t> payload{};
  DecodeError error = DecodeError::None;

  explicit operator bool() const { return error == DecodeError::None; }
};

struct AuthenticationInput {
  std::span<const std::uint8_t> headerPrefix;
  std::span<const std::uint8_t> payload;
};

Priority canonicalPriority(PacketType type);
std::uint32_t maxPayloadFor(PacketType type);
bool isKnownPacketType(std::uint8_t value);
WireHeader encodeHeader(const Header& header);
DecodeResult decodePacket(std::span<const std::uint8_t> packet);
AuthenticationInput authenticationInput(const WireHeader& header,
                                        std::span<const std::uint8_t> payload);
std::string_view decodeErrorMessage(DecodeError error);

} // namespace sanser::snv2
