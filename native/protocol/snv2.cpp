#include "snv2.h"

#include <algorithm>

namespace sanser::snv2 {
namespace {

void writeU16(std::uint8_t* target, std::uint16_t value) {
  target[0] = static_cast<std::uint8_t>(value >> 8);
  target[1] = static_cast<std::uint8_t>(value);
}

void writeU32(std::uint8_t* target, std::uint32_t value) {
  for (int index = 3; index >= 0; --index) {
    target[3 - index] = static_cast<std::uint8_t>(value >> (index * 8));
  }
}

void writeU64(std::uint8_t* target, std::uint64_t value) {
  for (int index = 7; index >= 0; --index) {
    target[7 - index] = static_cast<std::uint8_t>(value >> (index * 8));
  }
}

std::uint16_t readU16(const std::uint8_t* source) {
  return static_cast<std::uint16_t>((static_cast<std::uint16_t>(source[0]) << 8) |
                                    static_cast<std::uint16_t>(source[1]));
}

std::uint32_t readU32(const std::uint8_t* source) {
  std::uint32_t value = 0;
  for (int index = 0; index < 4; ++index) value = (value << 8) | source[index];
  return value;
}

std::uint64_t readU64(const std::uint8_t* source) {
  std::uint64_t value = 0;
  for (int index = 0; index < 8; ++index) value = (value << 8) | source[index];
  return value;
}

bool emptySession(const SessionId& sessionId) {
  return std::all_of(sessionId.begin(), sessionId.end(), [](std::uint8_t value) {
    return value == 0;
  });
}

PacketType packetTypeFrom(std::uint8_t value) {
  return static_cast<PacketType>(value);
}

} // namespace

Priority canonicalPriority(PacketType type) {
  switch (type) {
    case PacketType::MouseButton:
    case PacketType::Keyboard:
    case PacketType::Disconnect:
      return Priority::ReliableInput;
    case PacketType::MouseMove:
    case PacketType::Gamepad:
      return Priority::RealtimeInput;
    case PacketType::Audio:
      return Priority::Audio;
    case PacketType::Handshake:
    case PacketType::Authentication:
    case PacketType::NetworkFeedback:
    case PacketType::Nack:
    case PacketType::KeyframeRequest:
    case PacketType::Keepalive:
      return Priority::VideoControl;
    case PacketType::Video:
      return Priority::VideoPayload;
    case PacketType::Clipboard:
    case PacketType::EncoderFeedback:
    case PacketType::DecoderFeedback:
    case PacketType::Error:
      return Priority::Diagnostics;
  }
  return Priority::Diagnostics;
}

std::uint32_t maxPayloadFor(PacketType type) {
  switch (type) {
    case PacketType::Video: return kMaxPayloadSize;
    case PacketType::Audio: return 64U * 1024U;
    case PacketType::Clipboard: return 256U * 1024U;
    case PacketType::Handshake:
    case PacketType::Authentication: return 16U * 1024U;
    case PacketType::MouseMove:
    case PacketType::MouseButton:
    case PacketType::Keyboard:
    case PacketType::Gamepad: return 4U * 1024U;
    default: return 64U * 1024U;
  }
}

bool isKnownPacketType(std::uint8_t value) {
  return value >= static_cast<std::uint8_t>(PacketType::Handshake) &&
         value <= static_cast<std::uint8_t>(PacketType::Error);
}

WireHeader encodeHeader(const Header& header) {
  WireHeader wire{};
  std::copy(kMagic.begin(), kMagic.end(), wire.begin());
  wire[4] = kProtocolVersion;
  wire[5] = static_cast<std::uint8_t>(kHeaderSize);
  wire[6] = static_cast<std::uint8_t>(header.packetType);
  wire[7] = static_cast<std::uint8_t>(header.priority);
  writeU16(wire.data() + 8, header.flags);
  std::copy(header.sessionId.begin(), header.sessionId.end(), wire.begin() + 12);
  writeU32(wire.data() + 28, header.streamId);
  writeU64(wire.data() + 32, header.sequence);
  writeU64(wire.data() + 40, header.frameId);
  writeU64(wire.data() + 48, header.timestampMicros);
  writeU32(wire.data() + 56, header.payloadLength);
  writeU32(wire.data() + 60, header.keyId);
  std::copy(header.authTag.begin(), header.authTag.end(), wire.begin() + 64);
  return wire;
}

DecodeResult decodePacket(std::span<const std::uint8_t> packet) {
  DecodeResult result;
  if (packet.size() < kHeaderSize) {
    result.error = DecodeError::Truncated;
    return result;
  }
  if (!std::equal(kMagic.begin(), kMagic.end(), packet.begin())) {
    result.error = DecodeError::BadMagic;
    return result;
  }
  if (packet[4] != kProtocolVersion) {
    result.error = DecodeError::UnsupportedVersion;
    return result;
  }
  if (packet[5] != kHeaderSize) {
    result.error = DecodeError::BadHeaderLength;
    return result;
  }
  if (readU16(packet.data() + 10) != 0) {
    result.error = DecodeError::ReservedBitsSet;
    return result;
  }
  if (!isKnownPacketType(packet[6])) {
    result.error = DecodeError::UnknownPacketType;
    return result;
  }

  result.header.packetType = packetTypeFrom(packet[6]);
  result.header.priority = static_cast<Priority>(packet[7]);
  if (result.header.priority != canonicalPriority(result.header.packetType)) {
    result.error = DecodeError::PriorityMismatch;
    return result;
  }
  result.header.flags = readU16(packet.data() + 8);
  std::copy_n(packet.begin() + 12, result.header.sessionId.size(), result.header.sessionId.begin());
  if (emptySession(result.header.sessionId)) {
    result.error = DecodeError::EmptySession;
    return result;
  }
  result.header.streamId = readU32(packet.data() + 28);
  result.header.sequence = readU64(packet.data() + 32);
  result.header.frameId = readU64(packet.data() + 40);
  result.header.timestampMicros = readU64(packet.data() + 48);
  result.header.payloadLength = readU32(packet.data() + 56);
  result.header.keyId = readU32(packet.data() + 60);
  std::copy_n(packet.begin() + 64, result.header.authTag.size(), result.header.authTag.begin());

  if (result.header.payloadLength > kMaxPayloadSize) {
    result.error = DecodeError::PayloadTooLarge;
    return result;
  }
  if (result.header.payloadLength > maxPayloadFor(result.header.packetType)) {
    result.error = DecodeError::TypePayloadTooLarge;
    return result;
  }
  if (packet.size() != kHeaderSize + result.header.payloadLength) {
    result.error = DecodeError::LengthMismatch;
    return result;
  }
  result.payload = packet.subspan(kHeaderSize, result.header.payloadLength);
  return result;
}

AuthenticationInput authenticationInput(const WireHeader& header,
                                        std::span<const std::uint8_t> payload) {
  return {std::span<const std::uint8_t>(header.data(), kAuthenticatedHeaderSize), payload};
}

std::string_view decodeErrorMessage(DecodeError error) {
  switch (error) {
    case DecodeError::None: return "ok";
    case DecodeError::Truncated: return "packet is shorter than the SNV2 header";
    case DecodeError::BadMagic: return "invalid SNV2 magic";
    case DecodeError::UnsupportedVersion: return "unsupported protocol version";
    case DecodeError::BadHeaderLength: return "invalid header length";
    case DecodeError::ReservedBitsSet: return "reserved header bits must be zero";
    case DecodeError::UnknownPacketType: return "unknown packet type";
    case DecodeError::PriorityMismatch: return "packet priority does not match packet type";
    case DecodeError::EmptySession: return "session id must not be empty";
    case DecodeError::PayloadTooLarge: return "payload exceeds the global limit";
    case DecodeError::TypePayloadTooLarge: return "payload exceeds the packet-type limit";
    case DecodeError::LengthMismatch: return "packet length does not match payload length";
  }
  return "unknown decode error";
}

} // namespace sanser::snv2
