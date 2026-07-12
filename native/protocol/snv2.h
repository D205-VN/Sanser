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
inline constexpr std::size_t kAcknowledgementPayloadSize = 16;

inline constexpr std::uint16_t kFlagKeyFrame = 1U << 0;
inline constexpr std::uint16_t kFlagEndOfFrame = 1U << 1;
inline constexpr std::uint16_t kFlagRetransmitted = 1U << 2;
inline constexpr std::uint16_t kFlagAcknowledgementRequired = 1U << 3;
inline constexpr std::uint16_t kFlagDiscontinuity = 1U << 4;
inline constexpr std::uint16_t kFlagStartOfFrame = 1U << 5;
inline constexpr std::uint16_t kKnownPacketFlags =
    kFlagKeyFrame | kFlagEndOfFrame | kFlagRetransmitted |
    kFlagAcknowledgementRequired | kFlagDiscontinuity | kFlagStartOfFrame;

using SessionId = std::array<std::uint8_t, 16>;
using AuthTag = std::array<std::uint8_t, kAuthTagSize>;
using WireHeader = std::array<std::uint8_t, kHeaderSize>;
using AcknowledgementPayload = std::array<std::uint8_t, kAcknowledgementPayloadSize>;

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
  Acknowledgement = 18,
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
  UnknownFlags,
  EmptySession,
  PayloadTooLarge,
  TypePayloadTooLarge,
  InvalidPayloadLength,
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

// Cumulatively acknowledges cumulativeSequence and every earlier sequence in
// one (session, stream, direction, key generation) tuple. Selective mask bit i
// (bit zero is the least-significant bit) acknowledges
// cumulativeSequence + 1 + i. Sequence numbers do not wrap within a key
// generation. A receiver emits no acknowledgement until it has a cumulative
// base sequence.
struct Acknowledgement {
  std::uint64_t cumulativeSequence = 0;
  std::uint64_t selectiveMask = 0;

  bool acknowledges(std::uint64_t sequence) const;
};

Priority canonicalPriority(PacketType type);
std::uint32_t maxPayloadFor(PacketType type);
bool isKnownPacketType(std::uint8_t value);
AcknowledgementPayload encodeAcknowledgement(const Acknowledgement& acknowledgement);
bool decodeAcknowledgement(std::span<const std::uint8_t> payload,
                           Acknowledgement& acknowledgement);
WireHeader encodeHeader(const Header& header);
DecodeResult decodePacket(std::span<const std::uint8_t> packet);
AuthenticationInput authenticationInput(const WireHeader& header,
                                        std::span<const std::uint8_t> payload);
std::string_view decodeErrorMessage(DecodeError error);

} // namespace sanser::snv2
