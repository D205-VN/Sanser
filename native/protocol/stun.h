#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <span>
#include <string_view>

namespace sanser::stun {

inline constexpr std::size_t kHeaderSize = 20;
inline constexpr std::uint32_t kMagicCookie = 0x2112A442U;
inline constexpr std::uint16_t kBindingRequestType = 0x0001U;
inline constexpr std::uint16_t kBindingSuccessType = 0x0101U;
inline constexpr std::uint16_t kXorMappedAddressType = 0x0020U;

using TransactionId = std::array<std::uint8_t, 12>;
using BindingRequest = std::array<std::uint8_t, kHeaderSize>;

enum class AddressFamily : std::uint8_t {
  Ipv4 = 0x01,
  Ipv6 = 0x02,
};

struct MappedAddress {
  AddressFamily family = AddressFamily::Ipv4;
  std::uint16_t port = 0;
  std::array<std::uint8_t, 16> address{};

  [[nodiscard]] constexpr std::size_t addressSize() const noexcept {
    return family == AddressFamily::Ipv4 ? 4U : 16U;
  }

  [[nodiscard]] constexpr std::span<const std::uint8_t> addressBytes() const noexcept {
    return std::span<const std::uint8_t>(address.data(), addressSize());
  }
};

enum class ParseError {
  None,
  TruncatedHeader,
  UnexpectedMessageType,
  BadMagicCookie,
  InvalidMessageLength,
  LengthMismatch,
  TransactionMismatch,
  MalformedAttribute,
  UnknownRequiredAttribute,
  MissingXorMappedAddress,
  InvalidXorMappedAddressLength,
  UnsupportedAddressFamily,
};

struct BindingSuccessResult {
  MappedAddress mappedAddress{};
  ParseError error = ParseError::None;

  explicit operator bool() const noexcept { return error == ParseError::None; }
};

// The caller owns transaction-ID generation. RFC 8489 requires a uniformly
// distributed, cryptographically random 96-bit ID for each new transaction.
[[nodiscard]] BindingRequest encodeBindingRequest(const TransactionId& transactionId) noexcept;

// Parses exactly one STUN Binding Success response. The expected transaction
// ID ties the response to an outstanding request and prevents accepting a
// response for another transaction.
[[nodiscard]] BindingSuccessResult parseBindingSuccess(
    std::span<const std::uint8_t> message,
    const TransactionId& expectedTransactionId) noexcept;

[[nodiscard]] std::string_view parseErrorMessage(ParseError error) noexcept;

} // namespace sanser::stun
