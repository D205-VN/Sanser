#include "snv2.h"
#include "snv2_auth.h"

#include <algorithm>
#include <cassert>
#include <vector>

using namespace sanser::snv2;

int main() {
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
  assert(decoded);
  assert(decoded.header.sequence == header.sequence);
  assert(decoded.header.priority == Priority::ReliableInput);
  assert(decoded.payload.size() == 3 && decoded.payload[2] == 3);

  DirectionKey key{};
  key[0] = 9;
  Header unsignedHeader = header;
  unsignedHeader.authTag.fill(0);
  WireHeader authenticatedWire = encodeHeader(unsignedHeader);
  const std::array<std::uint8_t, 3> payload{1, 2, 3};
  const auto input = authenticationInput(authenticatedWire, payload);
  const AuthTag tag = computeAuthTag(key, input.headerPrefix, input.payload);
  assert(verifyAuthTag(key, input.headerPrefix, input.payload, tag));
  AuthTag wrongTag = tag;
  wrongTag[0] ^= 1;
  assert(!verifyAuthTag(key, input.headerPrefix, input.payload, wrongTag));

  packet[4] = 3;
  assert(decodePacket(packet).error == DecodeError::UnsupportedVersion);
  packet[4] = kProtocolVersion;
  packet[7] = static_cast<std::uint8_t>(Priority::VideoPayload);
  assert(decodePacket(packet).error == DecodeError::PriorityMismatch);
  packet[7] = static_cast<std::uint8_t>(Priority::ReliableInput);
  packet.push_back(0);
  assert(decodePacket(packet).error == DecodeError::LengthMismatch);
}
