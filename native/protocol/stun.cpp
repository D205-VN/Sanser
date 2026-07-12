#include "stun.h"

#include <algorithm>

namespace sanser::stun {
namespace {

constexpr std::size_t kMessageTypeOffset = 0;
constexpr std::size_t kMessageLengthOffset = 2;
constexpr std::size_t kMagicCookieOffset = 4;
constexpr std::size_t kTransactionIdOffset = 8;
constexpr std::size_t kAttributeHeaderSize = 4;

void writeU16(std::uint8_t* target, std::uint16_t value) noexcept {
  target[0] = static_cast<std::uint8_t>(value >> 8U);
  target[1] = static_cast<std::uint8_t>(value);
}

void writeU32(std::uint8_t* target, std::uint32_t value) noexcept {
  target[0] = static_cast<std::uint8_t>(value >> 24U);
  target[1] = static_cast<std::uint8_t>(value >> 16U);
  target[2] = static_cast<std::uint8_t>(value >> 8U);
  target[3] = static_cast<std::uint8_t>(value);
}

std::uint16_t readU16(const std::uint8_t* source) noexcept {
  return static_cast<std::uint16_t>(
      (static_cast<std::uint16_t>(source[0]) << 8U) |
      static_cast<std::uint16_t>(source[1]));
}

std::uint32_t readU32(const std::uint8_t* source) noexcept {
  return (static_cast<std::uint32_t>(source[0]) << 24U) |
         (static_cast<std::uint32_t>(source[1]) << 16U) |
         (static_cast<std::uint32_t>(source[2]) << 8U) |
         static_cast<std::uint32_t>(source[3]);
}

constexpr std::size_t paddedSize(std::uint16_t valueLength) noexcept {
  const std::size_t length = valueLength;
  return (length + 3U) & ~std::size_t{3U};
}

// Attributes defined by RFC 8489 are known even when this small Binding
// parser does not consume their values. Known-but-unexpected attributes are
// ignored; an unknown comprehension-required attribute fails the response.
constexpr bool isRfc8489Attribute(std::uint16_t type) noexcept {
  switch (type) {
    case 0x0001U: // MAPPED-ADDRESS
    case 0x0006U: // USERNAME
    case 0x0008U: // MESSAGE-INTEGRITY
    case 0x0009U: // ERROR-CODE
    case 0x000AU: // UNKNOWN-ATTRIBUTES
    case 0x0014U: // REALM
    case 0x0015U: // NONCE
    case 0x001CU: // MESSAGE-INTEGRITY-SHA256
    case 0x001DU: // PASSWORD-ALGORITHM
    case 0x001EU: // USERHASH
    case kXorMappedAddressType:
    case 0x8002U: // PASSWORD-ALGORITHMS
    case 0x8003U: // ALTERNATE-DOMAIN
    case 0x8022U: // SOFTWARE
    case 0x8023U: // ALTERNATE-SERVER
    case 0x8028U: // FINGERPRINT
      return true;
    default:
      return false;
  }
}

BindingSuccessResult failure(ParseError error) noexcept {
  BindingSuccessResult result;
  result.error = error;
  return result;
}

ParseError decodeXorMappedAddress(std::span<const std::uint8_t> value,
                                  const TransactionId& transactionId,
                                  MappedAddress& output) noexcept {
  if (value.size() < 4U) return ParseError::InvalidXorMappedAddressLength;

  const std::uint8_t familyValue = value[1];
  std::size_t addressLength = 0;
  if (familyValue == static_cast<std::uint8_t>(AddressFamily::Ipv4)) {
    output.family = AddressFamily::Ipv4;
    addressLength = 4U;
  } else if (familyValue == static_cast<std::uint8_t>(AddressFamily::Ipv6)) {
    output.family = AddressFamily::Ipv6;
    addressLength = 16U;
  } else {
    return ParseError::UnsupportedAddressFamily;
  }

  if (value.size() != 4U + addressLength) {
    return ParseError::InvalidXorMappedAddressLength;
  }

  output.port = static_cast<std::uint16_t>(readU16(value.data() + 2U) ^
                                           static_cast<std::uint16_t>(kMagicCookie >> 16U));

  constexpr std::array<std::uint8_t, 4> cookieBytes{
      static_cast<std::uint8_t>(kMagicCookie >> 24U),
      static_cast<std::uint8_t>(kMagicCookie >> 16U),
      static_cast<std::uint8_t>(kMagicCookie >> 8U),
      static_cast<std::uint8_t>(kMagicCookie),
  };

  output.address.fill(0);
  for (std::size_t index = 0; index < addressLength; ++index) {
    const std::uint8_t mask = index < cookieBytes.size()
                                  ? cookieBytes[index]
                                  : transactionId[index - cookieBytes.size()];
    output.address[index] = static_cast<std::uint8_t>(value[4U + index] ^ mask);
  }
  return ParseError::None;
}

} // namespace

BindingRequest encodeBindingRequest(const TransactionId& transactionId) noexcept {
  BindingRequest request{};
  writeU16(request.data() + kMessageTypeOffset, kBindingRequestType);
  writeU16(request.data() + kMessageLengthOffset, 0U);
  writeU32(request.data() + kMagicCookieOffset, kMagicCookie);
  std::copy(transactionId.begin(), transactionId.end(),
            request.begin() + static_cast<std::ptrdiff_t>(kTransactionIdOffset));
  return request;
}

BindingSuccessResult parseBindingSuccess(
    std::span<const std::uint8_t> message,
    const TransactionId& expectedTransactionId) noexcept {
  if (message.size() < kHeaderSize) return failure(ParseError::TruncatedHeader);
  if (readU16(message.data() + kMessageTypeOffset) != kBindingSuccessType) {
    return failure(ParseError::UnexpectedMessageType);
  }
  if (readU32(message.data() + kMagicCookieOffset) != kMagicCookie) {
    return failure(ParseError::BadMagicCookie);
  }

  const std::uint16_t bodyLength = readU16(message.data() + kMessageLengthOffset);
  if ((bodyLength & 0x0003U) != 0U) return failure(ParseError::InvalidMessageLength);
  if (message.size() != kHeaderSize + static_cast<std::size_t>(bodyLength)) {
    return failure(ParseError::LengthMismatch);
  }
  if (!std::equal(expectedTransactionId.begin(), expectedTransactionId.end(),
                  message.begin() + static_cast<std::ptrdiff_t>(kTransactionIdOffset))) {
    return failure(ParseError::TransactionMismatch);
  }

  BindingSuccessResult result;
  bool foundXorMappedAddress = false;
  std::size_t offset = kHeaderSize;
  while (offset < message.size()) {
    const std::size_t remaining = message.size() - offset;
    if (remaining < kAttributeHeaderSize) return failure(ParseError::MalformedAttribute);

    const std::uint16_t type = readU16(message.data() + offset);
    const std::uint16_t valueLength = readU16(message.data() + offset + 2U);
    const std::size_t paddedValueLength = paddedSize(valueLength);
    const std::size_t valueOffset = offset + kAttributeHeaderSize;
    if (paddedValueLength > message.size() - valueOffset) {
      return failure(ParseError::MalformedAttribute);
    }

    if (type == kXorMappedAddressType && !foundXorMappedAddress) {
      const ParseError addressError = decodeXorMappedAddress(
          message.subspan(valueOffset, static_cast<std::size_t>(valueLength)),
          expectedTransactionId, result.mappedAddress);
      if (addressError != ParseError::None) return failure(addressError);
      foundXorMappedAddress = true;
    } else if (!isRfc8489Attribute(type) && type < 0x8000U) {
      return failure(ParseError::UnknownRequiredAttribute);
    }

    offset = valueOffset + paddedValueLength;
  }

  if (!foundXorMappedAddress) return failure(ParseError::MissingXorMappedAddress);
  return result;
}

std::string_view parseErrorMessage(ParseError error) noexcept {
  switch (error) {
    case ParseError::None: return "ok";
    case ParseError::TruncatedHeader: return "message is shorter than the STUN header";
    case ParseError::UnexpectedMessageType: return "message is not a Binding Success response";
    case ParseError::BadMagicCookie: return "invalid STUN magic cookie";
    case ParseError::InvalidMessageLength: return "STUN message length is not 32-bit aligned";
    case ParseError::LengthMismatch: return "datagram size does not match STUN message length";
    case ParseError::TransactionMismatch: return "response transaction ID does not match request";
    case ParseError::MalformedAttribute: return "STUN attribute exceeds message bounds";
    case ParseError::UnknownRequiredAttribute:
      return "response contains an unknown comprehension-required attribute";
    case ParseError::MissingXorMappedAddress: return "response has no XOR-MAPPED-ADDRESS";
    case ParseError::InvalidXorMappedAddressLength:
      return "XOR-MAPPED-ADDRESS length does not match its address family";
    case ParseError::UnsupportedAddressFamily:
      return "XOR-MAPPED-ADDRESS uses an unsupported address family";
  }
  return "unknown STUN parse error";
}

} // namespace sanser::stun
