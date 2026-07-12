#include "stun.h"

#include <algorithm>
#include <array>
#include <cstdint>
#include <iostream>
#include <stdexcept>
#include <vector>

using namespace sanser::stun;

namespace {

void require(bool condition, const char* message) {
  if (!condition) throw std::runtime_error(message);
}

constexpr TransactionId kTransactionId{
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05,
    0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
};

// Fixed wire vector: an optional attribute with non-zero padding precedes an
// IPv4 XOR-MAPPED-ADDRESS for 203.0.113.7:50000. Receivers must ignore padding.
constexpr std::array<std::uint8_t, 40> kIpv4Response{
    0x01, 0x01, 0x00, 0x14, 0x21, 0x12, 0xA4, 0x42,
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    0x08, 0x09, 0x0A, 0x0B,
    0x80, 0x22, 0x00, 0x03, 0x61, 0x62, 0x63, 0xA5,
    0x00, 0x20, 0x00, 0x08, 0x00, 0x01, 0xE2, 0x42,
    0xEA, 0x12, 0xD5, 0x45,
};

// Fixed wire vector for [2001:db8:1:2:3:4:5:6]:3478. The IPv6 mask is the
// concatenation of the magic cookie and the 96-bit transaction ID.
constexpr std::array<std::uint8_t, 44> kIpv6Response{
    0x01, 0x01, 0x00, 0x18, 0x21, 0x12, 0xA4, 0x42,
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    0x08, 0x09, 0x0A, 0x0B,
    0x00, 0x20, 0x00, 0x14, 0x00, 0x02, 0x2C, 0x84,
    0x01, 0x13, 0xA9, 0xFA, 0x00, 0x00, 0x02, 0x01,
    0x04, 0x06, 0x06, 0x03, 0x08, 0x0C, 0x0A, 0x0D,
};

// RFC 5769 Sections 2.2 and 2.3 interoperability vectors. RFC 8489 retains
// the same header and XOR-MAPPED-ADDRESS wire representation.
constexpr TransactionId kRfc5769TransactionId{
    0xB7, 0xE7, 0xA7, 0x01, 0xBC, 0x34,
    0xD6, 0x86, 0xFA, 0x87, 0xDF, 0xAE,
};

constexpr std::array<std::uint8_t, 80> kRfc5769Ipv4Response{
    0x01, 0x01, 0x00, 0x3C, 0x21, 0x12, 0xA4, 0x42,
    0xB7, 0xE7, 0xA7, 0x01, 0xBC, 0x34, 0xD6, 0x86,
    0xFA, 0x87, 0xDF, 0xAE,
    0x80, 0x22, 0x00, 0x0B, 0x74, 0x65, 0x73, 0x74,
    0x20, 0x76, 0x65, 0x63, 0x74, 0x6F, 0x72, 0x20,
    0x00, 0x20, 0x00, 0x08, 0x00, 0x01, 0xA1, 0x47,
    0xE1, 0x12, 0xA6, 0x43,
    0x00, 0x08, 0x00, 0x14, 0x2B, 0x91, 0xF5, 0x99,
    0xFD, 0x9E, 0x90, 0xC3, 0x8C, 0x74, 0x89, 0xF9,
    0x2A, 0xF9, 0xBA, 0x53, 0xF0, 0x6B, 0xE7, 0xD7,
    0x80, 0x28, 0x00, 0x04, 0xC0, 0x7D, 0x4C, 0x96,
};

constexpr std::array<std::uint8_t, 92> kRfc5769Ipv6Response{
    0x01, 0x01, 0x00, 0x48, 0x21, 0x12, 0xA4, 0x42,
    0xB7, 0xE7, 0xA7, 0x01, 0xBC, 0x34, 0xD6, 0x86,
    0xFA, 0x87, 0xDF, 0xAE,
    0x80, 0x22, 0x00, 0x0B, 0x74, 0x65, 0x73, 0x74,
    0x20, 0x76, 0x65, 0x63, 0x74, 0x6F, 0x72, 0x20,
    0x00, 0x20, 0x00, 0x14, 0x00, 0x02, 0xA1, 0x47,
    0x01, 0x13, 0xA9, 0xFA, 0xA5, 0xD3, 0xF1, 0x79,
    0xBC, 0x25, 0xF4, 0xB5, 0xBE, 0xD2, 0xB9, 0xD9,
    0x00, 0x08, 0x00, 0x14, 0xA3, 0x82, 0x95, 0x4E,
    0x4B, 0xE6, 0x7B, 0xF1, 0x17, 0x84, 0xC9, 0x7C,
    0x82, 0x92, 0xC2, 0x75, 0xBF, 0xE3, 0xED, 0x41,
    0x80, 0x28, 0x00, 0x04, 0xC8, 0xFB, 0x0B, 0x4C,
};

void testBindingRequestVector() {
  constexpr BindingRequest expected{
      0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42,
      0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
      0x08, 0x09, 0x0A, 0x0B,
  };
  require(encodeBindingRequest(kTransactionId) == expected,
          "Binding Request wire vector changed");
}

void testIpv4Vector() {
  const BindingSuccessResult result = parseBindingSuccess(kIpv4Response, kTransactionId);
  require(static_cast<bool>(result), "valid IPv4 response was rejected");
  require(result.mappedAddress.family == AddressFamily::Ipv4, "wrong IPv4 family");
  require(result.mappedAddress.port == 50000U, "wrong IPv4 port");
  constexpr std::array<std::uint8_t, 4> expectedAddress{203, 0, 113, 7};
  require(std::equal(expectedAddress.begin(), expectedAddress.end(),
                     result.mappedAddress.addressBytes().begin()),
          "wrong IPv4 address");
}

void testIpv6Vector() {
  const BindingSuccessResult result = parseBindingSuccess(kIpv6Response, kTransactionId);
  require(static_cast<bool>(result), "valid IPv6 response was rejected");
  require(result.mappedAddress.family == AddressFamily::Ipv6, "wrong IPv6 family");
  require(result.mappedAddress.port == 3478U, "wrong IPv6 port");
  constexpr std::array<std::uint8_t, 16> expectedAddress{
      0x20, 0x01, 0x0D, 0xB8, 0x00, 0x01, 0x00, 0x02,
      0x00, 0x03, 0x00, 0x04, 0x00, 0x05, 0x00, 0x06,
  };
  require(result.mappedAddress.address == expectedAddress, "wrong IPv6 address");
}

void testRfc5769Vectors() {
  const BindingSuccessResult ipv4 =
      parseBindingSuccess(kRfc5769Ipv4Response, kRfc5769TransactionId);
  require(static_cast<bool>(ipv4), "RFC 5769 IPv4 response was rejected");
  require(ipv4.mappedAddress.family == AddressFamily::Ipv4, "wrong RFC IPv4 family");
  require(ipv4.mappedAddress.port == 32853U, "wrong RFC IPv4 port");
  constexpr std::array<std::uint8_t, 4> expectedIpv4{192, 0, 2, 1};
  require(std::equal(expectedIpv4.begin(), expectedIpv4.end(),
                     ipv4.mappedAddress.addressBytes().begin()),
          "wrong RFC IPv4 address");

  const BindingSuccessResult ipv6 =
      parseBindingSuccess(kRfc5769Ipv6Response, kRfc5769TransactionId);
  require(static_cast<bool>(ipv6), "RFC 5769 IPv6 response was rejected");
  require(ipv6.mappedAddress.family == AddressFamily::Ipv6, "wrong RFC IPv6 family");
  require(ipv6.mappedAddress.port == 32853U, "wrong RFC IPv6 port");
  constexpr std::array<std::uint8_t, 16> expectedIpv6{
      0x20, 0x01, 0x0D, 0xB8, 0x12, 0x34, 0x56, 0x78,
      0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
  };
  require(ipv6.mappedAddress.address == expectedIpv6, "wrong RFC IPv6 address");
}

void testHeaderValidation() {
  std::vector<std::uint8_t> message(kIpv4Response.begin(), kIpv4Response.end());

  for (std::size_t size = 0; size < kHeaderSize; ++size) {
    require(parseBindingSuccess(std::span<const std::uint8_t>(message).first(size),
                                kTransactionId)
                .error == ParseError::TruncatedHeader,
            "truncated header was accepted");
  }

  message[0] = 0x00;
  require(parseBindingSuccess(message, kTransactionId).error ==
              ParseError::UnexpectedMessageType,
          "wrong message type was accepted");
  message[0] = 0x01;

  message[0] = 0x81;
  require(parseBindingSuccess(message, kTransactionId).error ==
              ParseError::UnexpectedMessageType,
          "message with non-zero leading type bits was accepted");
  message[0] = 0x01;

  message[4] ^= 0x01U;
  require(parseBindingSuccess(message, kTransactionId).error == ParseError::BadMagicCookie,
          "wrong magic cookie was accepted");
  message[4] ^= 0x01U;

  message[3] = 0x13;
  require(parseBindingSuccess(message, kTransactionId).error ==
              ParseError::InvalidMessageLength,
          "unaligned message length was accepted");
  message[3] = 0x14;

  message.push_back(0);
  require(parseBindingSuccess(message, kTransactionId).error == ParseError::LengthMismatch,
          "trailing datagram bytes were accepted");
  message.pop_back();

  message.pop_back();
  require(parseBindingSuccess(message, kTransactionId).error == ParseError::LengthMismatch,
          "truncated datagram body was accepted");
  message.push_back(kIpv4Response.back());

  TransactionId otherTransaction = kTransactionId;
  otherTransaction.back() ^= 0x01U;
  require(parseBindingSuccess(message, otherTransaction).error ==
              ParseError::TransactionMismatch,
          "response for another transaction was accepted");
}

void testAttributeBoundsAndPolicy() {
  std::vector<std::uint8_t> message(kIpv4Response.begin(), kIpv4Response.end());

  // The first attribute claims a value whose padded extent crosses the body.
  message[22] = 0x00;
  message[23] = 0x11;
  require(parseBindingSuccess(message, kTransactionId).error == ParseError::MalformedAttribute,
          "out-of-bounds attribute was accepted");

  message.assign(kIpv4Response.begin(), kIpv4Response.end());
  message[20] = 0xC1;
  message[21] = 0x23;
  require(static_cast<bool>(parseBindingSuccess(message, kTransactionId)),
          "unknown comprehension-optional attribute was rejected");

  message.assign(kIpv4Response.begin(), kIpv4Response.end());
  message[32] = 0xFF;
  require(static_cast<bool>(parseBindingSuccess(message, kTransactionId)),
          "reserved XOR-MAPPED-ADDRESS byte was not ignored");

  message.assign(kIpv4Response.begin(), kIpv4Response.end());
  message[20] = 0x00;
  message[21] = 0x7F;
  require(parseBindingSuccess(message, kTransactionId).error ==
              ParseError::UnknownRequiredAttribute,
          "unknown comprehension-required attribute was ignored");

  message.assign(kIpv4Response.begin(), kIpv4Response.end());
  message[30] = 0x00;
  message[31] = 0x04;
  require(parseBindingSuccess(message, kTransactionId).error ==
              ParseError::InvalidXorMappedAddressLength,
          "short XOR-MAPPED-ADDRESS was accepted");

  message.assign(kIpv4Response.begin(), kIpv4Response.end());
  message[33] = 0x03;
  require(parseBindingSuccess(message, kTransactionId).error ==
              ParseError::UnsupportedAddressFamily,
          "unknown address family was accepted");

  constexpr std::array<std::uint8_t, kHeaderSize> noAttributes{
      0x01, 0x01, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42,
      0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
      0x08, 0x09, 0x0A, 0x0B,
  };
  require(parseBindingSuccess(noAttributes, kTransactionId).error ==
              ParseError::MissingXorMappedAddress,
          "response without XOR-MAPPED-ADDRESS was accepted");

  // RFC 8489 permits receivers to process the first duplicate occurrence.
  message.assign(kIpv4Response.begin(), kIpv4Response.end());
  message[2] = 0x00;
  message[3] = 0x20;
  message.insert(message.end(), kIpv4Response.begin() + 28, kIpv4Response.end());
  message[message.size() - 2U] ^= 0xFFU;
  const BindingSuccessResult duplicate = parseBindingSuccess(message, kTransactionId);
  require(static_cast<bool>(duplicate), "duplicate XOR-MAPPED-ADDRESS was rejected");
  require(duplicate.mappedAddress.port == 50000U,
          "duplicate XOR-MAPPED-ADDRESS replaced the first occurrence");
}

void testArbitraryInputIsBounded() {
  std::uint32_t state = 0xA5A55A5AU;
  for (std::size_t size = 0; size <= 256U; ++size) {
    std::vector<std::uint8_t> bytes(size);
    for (std::uint8_t& byte : bytes) {
      state = state * 1664525U + 1013904223U;
      byte = static_cast<std::uint8_t>(state >> 24U);
    }
    static_cast<void>(parseBindingSuccess(bytes, kTransactionId));
  }
}

} // namespace

int main() try {
  testBindingRequestVector();
  testIpv4Vector();
  testIpv6Vector();
  testRfc5769Vectors();
  testHeaderValidation();
  testAttributeBoundsAndPolicy();
  testArbitraryInputIsBounded();
  return 0;
} catch (const std::exception& error) {
  std::cerr << "sanser-stun-test: " << error.what() << '\n';
  return 1;
}
