#include "snv2_auth.h"

#include <algorithm>
#include <array>
#include <stdexcept>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <bcrypt.h>
#elif defined(__APPLE__)
#include <CommonCrypto/CommonHMAC.h>
#else
#error "SNV2 native authentication requires BCrypt on Windows or CommonCrypto on macOS."
#endif

namespace sanser::snv2 {
namespace {

using FullTag = std::array<std::uint8_t, 32>;

#if defined(_WIN32)
class AlgorithmHandle {
public:
  AlgorithmHandle() {
    const NTSTATUS status = BCryptOpenAlgorithmProvider(
      &handle_, BCRYPT_SHA256_ALGORITHM, nullptr, BCRYPT_ALG_HANDLE_HMAC_FLAG);
    if (status < 0) throw std::runtime_error("BCryptOpenAlgorithmProvider(SHA256/HMAC) failed.");
  }
  ~AlgorithmHandle() {
    if (handle_) BCryptCloseAlgorithmProvider(handle_, 0);
  }
  BCRYPT_ALG_HANDLE get() const { return handle_; }

private:
  BCRYPT_ALG_HANDLE handle_ = nullptr;
};

FullTag hmacSha256(const DirectionKey& key,
                   std::span<const std::uint8_t> first,
                   std::span<const std::uint8_t> second) {
  AlgorithmHandle algorithm;
  BCRYPT_HASH_HANDLE hash = nullptr;
  NTSTATUS status = BCryptCreateHash(
    algorithm.get(),
    &hash,
    nullptr,
    0,
    const_cast<PUCHAR>(key.data()),
    static_cast<ULONG>(key.size()),
    0);
  if (status < 0) throw std::runtime_error("BCryptCreateHash(HMAC) failed.");

  const auto destroy = [&]() {
    if (hash) BCryptDestroyHash(hash);
    hash = nullptr;
  };
  const auto update = [&](std::span<const std::uint8_t> bytes) {
    if (bytes.empty()) return;
    status = BCryptHashData(hash,
                            const_cast<PUCHAR>(bytes.data()),
                            static_cast<ULONG>(bytes.size()),
                            0);
    if (status < 0) {
      destroy();
      throw std::runtime_error("BCryptHashData(HMAC) failed.");
    }
  };

  update(first);
  update(second);
  FullTag result{};
  status = BCryptFinishHash(hash, result.data(), static_cast<ULONG>(result.size()), 0);
  destroy();
  if (status < 0) throw std::runtime_error("BCryptFinishHash(HMAC) failed.");
  return result;
}
#else
FullTag hmacSha256(const DirectionKey& key,
                   std::span<const std::uint8_t> first,
                   std::span<const std::uint8_t> second) {
  CCHmacContext context;
  CCHmacInit(&context, kCCHmacAlgSHA256, key.data(), key.size());
  if (!first.empty()) CCHmacUpdate(&context, first.data(), first.size());
  if (!second.empty()) CCHmacUpdate(&context, second.data(), second.size());
  FullTag result{};
  CCHmacFinal(&context, result.data());
  return result;
}
#endif

bool constantTimeEqual(const AuthTag& left, const AuthTag& right) {
  std::uint8_t difference = 0;
  for (std::size_t index = 0; index < left.size(); ++index) {
    difference |= static_cast<std::uint8_t>(left[index] ^ right[index]);
  }
  return difference == 0;
}

} // namespace

AuthTag computeAuthTag(const DirectionKey& key,
                       std::span<const std::uint8_t> authenticatedHeader,
                       std::span<const std::uint8_t> payload) {
  if (authenticatedHeader.size() != kAuthenticatedHeaderSize) {
    throw std::invalid_argument("SNV2 authenticated header must contain exactly 64 bytes.");
  }
  const FullTag full = hmacSha256(key, authenticatedHeader, payload);
  AuthTag tag{};
  std::copy_n(full.begin(), tag.size(), tag.begin());
  return tag;
}

bool verifyAuthTag(const DirectionKey& key,
                   std::span<const std::uint8_t> authenticatedHeader,
                   std::span<const std::uint8_t> payload,
                   const AuthTag& expected) {
  return constantTimeEqual(computeAuthTag(key, authenticatedHeader, payload), expected);
}

} // namespace sanser::snv2
