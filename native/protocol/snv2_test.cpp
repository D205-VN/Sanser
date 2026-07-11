#include "snv2.h"
#include "snv2_auth.h"

#include <algorithm>
#include <iostream>
#include <stdexcept>
#include <vector>

using namespace sanser::snv2;

namespace {

void require(bool condition, const char* message) {
  if (!condition) throw std::runtime_error(message);
}

} // namespace

int main() try {
  Header header;
  header.packetType = PacketType::MouseButton;
  header.priority = canonicalPriority(header.packetType);
  header.sessionId[0] = 1;
  header.streamId = 7;
  header.sequence = 99;
  header.frameId = 42;
  header.timestampMicros = 123456;
  header.payloadLength = 3;
  header.keyId = 5;
  header.authTag.fill(0xA5);

  const WireHeader wire = encodeHeader(header);
  std::vector<std::uint8_t> packet(wire.begin(), wire.end());
  packet.insert(packet.end(), {1, 2, 3});
  const DecodeResult decoded = decodePacket(packet);
  require(static_cast<bool>(decoded), "valid packet did not decode");
  require(decoded.header.sequence == header.sequence, "sequence did not round-trip");
  require(decoded.header.priority == Priority::ReliableInput, "priority did not round-trip");
  require(decoded.payload.size() == 3 && decoded.payload[2] == 3, "payload did not round-trip");

  DirectionKey key{};
  key[0] = 9;
  Header unsignedHeader = header;
  unsignedHeader.authTag.fill(0);
  WireHeader authenticatedWire = encodeHeader(unsignedHeader);
  const std::array<std::uint8_t, 3> payload{1, 2, 3};
  const auto input = authenticationInput(authenticatedWire, payload);
  const AuthTag tag = computeAuthTag(key, input.headerPrefix, input.payload);
  constexpr AuthTag expectedTag{
    0x02, 0xa2, 0x2a, 0x27, 0x7f, 0x31, 0xd8, 0x69,
    0x61, 0x7b, 0xda, 0x75, 0x02, 0x8e, 0xe5, 0x72,
  };
  require(tag == expectedTag, "cross-language authentication vector changed");
  require(verifyAuthTag(key, input.headerPrefix, input.payload, tag), "valid tag was rejected");
  AuthTag wrongTag = tag;
  wrongTag[0] ^= 1;
  require(!verifyAuthTag(key, input.headerPrefix, input.payload, wrongTag), "invalid tag was accepted");

  packet[4] = 3;
  require(decodePacket(packet).error == DecodeError::UnsupportedVersion,
          "unsupported version was accepted");
  packet[4] = kProtocolVersion;
  packet[7] = static_cast<std::uint8_t>(Priority::VideoPayload);
  require(decodePacket(packet).error == DecodeError::PriorityMismatch,
          "non-canonical priority was accepted");
  packet[7] = static_cast<std::uint8_t>(Priority::ReliableInput);
  packet.push_back(0);
  require(decodePacket(packet).error == DecodeError::LengthMismatch,
          "trailing packet bytes were accepted");
  return 0;
} catch (const std::exception& error) {
  std::cerr << "sanser-snv2-test: " << error.what() << '\n';
  return 1;
}
