#pragma once

#include <cstdint>
#include <string_view>

#ifndef SANSER_VERSION
#define SANSER_VERSION "2.0.5"
#endif

#ifndef SANSER_PROTOCOL_VERSION
#define SANSER_PROTOCOL_VERSION 2
#endif

namespace sanser {

inline constexpr std::string_view kProductName = "Sanser";
inline constexpr std::string_view kVersion = SANSER_VERSION;
inline constexpr std::uint8_t kProtocolVersion = SANSER_PROTOCOL_VERSION;

static_assert(!kVersion.empty(), "Native engine version must not be empty.");
static_assert(kProtocolVersion == 2, "Native engine protocol must be v2.");

} // namespace sanser
