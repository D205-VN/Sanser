#pragma once

#include "snv2.h"

#include <array>
#include <cstdint>
#include <span>

namespace sanser::snv2 {

using DirectionKey = std::array<std::uint8_t, 32>;

AuthTag computeAuthTag(const DirectionKey& key,
                       std::span<const std::uint8_t> authenticatedHeader,
                       std::span<const std::uint8_t> payload);

bool verifyAuthTag(const DirectionKey& key,
                   std::span<const std::uint8_t> authenticatedHeader,
                   std::span<const std::uint8_t> payload,
                   const AuthTag& expected);

} // namespace sanser::snv2
