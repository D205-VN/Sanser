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

  const Acknowledgement acknowledgement{
      0x0102030405060708ULL,
      0x8000000000000005ULL,
  };
  constexpr AcknowledgementPayload expectedAcknowledgement{
      0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
      0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05,
  };
  require(encodeAcknowledgement(acknowledgement) == expectedAcknowledgement,
          "ACK/SACK golden vector changed");
  Acknowledgement decodedAcknowledgement;
  require(decodeAcknowledgement(expectedAcknowledgement, decodedAcknowledgement),
          "valid ACK/SACK payload did not decode");
  require(decodedAcknowledgement.cumulativeSequence == acknowledgement.cumulativeSequence &&
              decodedAcknowledgement.selectiveMask == acknowledgement.selectiveMask,
          "ACK/SACK payload did not round-trip");
  require(decodedAcknowledgement.acknowledges(acknowledgement.cumulativeSequence - 1U),
          "sequence below the cumulative base was not acknowledged");
  require(decodedAcknowledgement.acknowledges(acknowledgement.cumulativeSequence),
          "cumulative sequence was not acknowledged");
  require(decodedAcknowledgement.acknowledges(acknowledgement.cumulativeSequence + 1U),
          "selective bit zero did not acknowledge cumulative + 1");
  require(!decodedAcknowledgement.acknowledges(acknowledgement.cumulativeSequence + 2U),
          "clear selective bit acknowledged a missing sequence");
  require(decodedAcknowledgement.acknowledges(acknowledgement.cumulativeSequence + 3U),
          "selective bit two did not acknowledge cumulative + 3");
  require(decodedAcknowledgement.acknowledges(acknowledgement.cumulativeSequence + 64U),
          "selective bit 63 did not acknowledge cumulative + 64");
  require(!decodedAcknowledgement.acknowledges(acknowledgement.cumulativeSequence + 65U),
          "selective window acknowledged an out-of-window sequence");
  require(!decodeAcknowledgement(
              std::span<const std::uint8_t>(expectedAcknowledgement.data(),
                                            expectedAcknowledgement.size() - 1),
              decodedAcknowledgement),
          "short ACK/SACK payload was accepted");

  Header videoHeader = header;
  videoHeader.packetType = PacketType::Video;
  videoHeader.priority = canonicalPriority(videoHeader.packetType);
  videoHeader.flags = kFlagKeyFrame | kFlagStartOfFrame | kFlagEndOfFrame;
  videoHeader.payloadLength = 0;
  const WireHeader videoWire = encodeHeader(videoHeader);
  std::vector<std::uint8_t> videoPacket(videoWire.begin(), videoWire.end());
  const DecodeResult decodedVideo = decodePacket(videoPacket);
  require(static_cast<bool>(decodedVideo), "START_OF_FRAME was rejected as unknown");
  require((decodedVideo.header.flags & kFlagStartOfFrame) != 0U,
          "START_OF_FRAME did not round-trip");
  videoPacket[9] |= static_cast<std::uint8_t>(1U << 6);
  require(decodePacket(videoPacket).error == DecodeError::UnknownFlags,
          "unknown packet flags were accepted");

  Header acknowledgementHeader = header;
  acknowledgementHeader.packetType = PacketType::Acknowledgement;
  acknowledgementHeader.priority = canonicalPriority(acknowledgementHeader.packetType);
  acknowledgementHeader.flags = 0;
  acknowledgementHeader.payloadLength = static_cast<std::uint32_t>(expectedAcknowledgement.size());
  const WireHeader acknowledgementWire = encodeHeader(acknowledgementHeader);
  std::vector<std::uint8_t> acknowledgementPacket(acknowledgementWire.begin(),
                                                  acknowledgementWire.end());
  acknowledgementPacket.insert(acknowledgementPacket.end(), expectedAcknowledgement.begin(),
                               expectedAcknowledgement.end());
  require(static_cast<bool>(decodePacket(acknowledgementPacket)),
          "valid acknowledgement packet did not decode");
  acknowledgementHeader.payloadLength--;
  const WireHeader shortAcknowledgementWire = encodeHeader(acknowledgementHeader);
  std::vector<std::uint8_t> shortAcknowledgementPacket(shortAcknowledgementWire.begin(),
                                                       shortAcknowledgementWire.end());
  shortAcknowledgementPacket.resize(kHeaderSize + acknowledgementHeader.payloadLength);
  require(decodePacket(shortAcknowledgementPacket).error == DecodeError::InvalidPayloadLength,
          "non-canonical acknowledgement payload length was accepted");

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
