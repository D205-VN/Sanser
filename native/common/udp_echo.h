#pragma once
#include "snv2_auth.h"
#include <algorithm>
#include <array>
#include <cstdint>
#include <map>
#include <optional>
#include <span>
#include <string_view>

namespace sanser::echo {
// Dedicated diagnostic lane on the existing media socket. Fixed-size authenticated
// packets, no amplification, no media/control sequence number space is consumed.
constexpr std::size_t kHeaderSize = 64;
constexpr std::size_t kSize = kHeaderSize + 16;
constexpr std::uint8_t kLane = 3;
using Packet = std::array<std::uint8_t, kSize>;
inline void put64(Packet& p, std::size_t at, std::uint64_t value) {
  for (unsigned i = 0; i < 8; ++i) p[at + i] = static_cast<std::uint8_t>(value >> (i * 8));
}
inline std::uint64_t get64(std::span<const std::uint8_t> p, std::size_t at) {
  std::uint64_t value = 0;
  for (unsigned i = 0; i < 8; ++i) value |= std::uint64_t(p[at + i]) << (i * 8);
  return value;
}
inline auto bytes(std::string_view value) {
  return std::span(reinterpret_cast<const std::uint8_t*>(value.data()), value.size());
}
class Codec {
  snv2::DirectionKey request_{}, response_{};
  bool enabled_ = false;
public:
  explicit Codec(std::string_view token) : enabled_(!token.empty()) {
    if (!enabled_) return;
    const auto master = snv2::deriveKey({}, bytes(token));
    request_ = snv2::deriveKey(master, bytes("sanser-udp-echo-v1/request"));
    response_ = snv2::deriveKey(master, bytes("sanser-udp-echo-v1/response"));
  }
  bool enabled() const { return enabled_; }
  Packet encode(bool response, std::uint64_t nonce, std::uint64_t sequence,
                std::uint64_t hostHoldMicros = 0) const {
    Packet packet{kLane, 'S', 'N', 'E', '1', static_cast<std::uint8_t>(response ? 2 : 1), 0, 0};
    put64(packet, 8, sequence); put64(packet, 16, nonce); put64(packet, 24, hostHoldMicros);
    auto tag = snv2::computeAuthTag(response ? response_ : request_, std::span(packet).first(kHeaderSize), {});
    std::copy(tag.begin(), tag.end(), packet.begin() + kHeaderSize);
    return packet;
  }
  bool valid(std::span<const std::uint8_t> p, bool response) const {
    if (!enabled_ || p.size() != kSize || p[0] != kLane || p[1] != 'S' || p[2] != 'N' ||
        p[3] != 'E' || p[4] != '1' || p[5] != (response ? 2 : 1) || p[6] || p[7] ||
        !get64(p, 8) || !get64(p, 16) || (!response && get64(p, 24))) return false;
    if (std::any_of(p.begin() + 32, p.begin() + kHeaderSize, [](auto byte) { return byte != 0; })) return false;
    snv2::AuthTag tag{}; std::copy(p.begin() + kHeaderSize, p.end(), tag.begin());
    return snv2::verifyAuthTag(response ? response_ : request_, p.first(kHeaderSize), {}, tag);
  }
};

// Receiver-thread owned. Validate/rate-limit before touching the send socket.
class Responder {
  Codec codec_;
  std::uint64_t nonce_ = 0, sequence_ = 0, lastAttempt_ = 0;
public:
  explicit Responder(std::string_view token) : codec_(token) {}
  bool accept(std::span<const std::uint8_t> p, std::uint64_t received) {
    if (lastAttempt_ && received - lastAttempt_ < 250000) return false;
    lastAttempt_ = received;
    if (!codec_.valid(p, false)) return false;
    const auto nonce = get64(p, 16), sequence = get64(p, 8);
    if ((nonce_ && nonce != nonce_) || sequence <= sequence_) return false;
    nonce_ = nonce; sequence_ = sequence;
    return true;
  }
  Packet response(std::uint64_t hostHoldMicros) const {
    return codec_.encode(true, nonce_, sequence_, hostHoldMicros);
  }
};

struct Sample { double rttMs = 0, hostHoldMs = 0, residualMs = 0, sendCallMs = 0; };
class Client {
  struct Pending { std::uint64_t sent = 0, sendCall = 0; };
  Codec codec_;
  std::uint64_t nonce_, next_ = 0, lastAttempt_ = 0;
  std::map<std::uint64_t, Pending> pending_;
public:
  Client(std::string_view token, std::uint64_t nonce) : codec_(token), nonce_(nonce) {}
  std::optional<Packet> prepare(std::uint64_t now) {
    if (!codec_.enabled() || !nonce_ || (lastAttempt_ && now - lastAttempt_ < 1000000)) return {};
    lastAttempt_ = now;
    while (pending_.size() >= 4) pending_.erase(pending_.begin());
    return codec_.encode(false, nonce_, ++next_);
  }
  // Called after encoding, immediately before sendto; no control sender queue.
  void sending(std::uint64_t at) { pending_[next_] = {at, 0}; }
  void sent(bool success, std::uint64_t callMicros) {
    if (!success) pending_.erase(next_);
    else pending_[next_].sendCall = callMicros;
  }
  std::optional<Sample> receive(std::span<const std::uint8_t> p, std::uint64_t received) {
    if (!codec_.valid(p, true) || get64(p, 16) != nonce_) return {};
    auto it = pending_.find(get64(p, 8));
    if (it == pending_.end()) return {};
    auto pending = it->second; pending_.erase(it);
    if (!pending.sent || received < pending.sent || received - pending.sent > 3000000) return {};
    const auto roundTrip = received - pending.sent, hold = get64(p, 24);
    if (hold > roundTrip) return {};
    return Sample{roundTrip / 1000.0, hold / 1000.0, (roundTrip - hold) / 1000.0, pending.sendCall / 1000.0};
  }
};
} // namespace sanser::echo
