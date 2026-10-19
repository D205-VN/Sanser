#pragma once
#include "audio.h"
#include <array>
#include <chrono>
#include <functional>
#include <map>
#include <memory>
#include <optional>
#include <set>
#include <span>
#include <string>
#include <vector>

namespace sanser::voice {
using Clock=std::chrono::steady_clock;
using Bytes=std::vector<std::uint8_t>;
inline constexpr unsigned maxSpeakers=4,packetFrames=960;
struct State { bool joined=false,microphone=false,deafened=false; };
std::optional<State> command(const std::string& line);
struct Packet { unsigned speaker=0; std::uint32_t epoch=0; std::uint64_t sequence=0; Bytes opus; };
Bytes pack(const Packet& packet);
std::optional<Packet> unpack(std::span<const std::uint8_t> packet);
bool isPacket(std::span<const std::uint8_t> packet);
class Encoder {
public:
  Encoder(); ~Encoder();
  Encoder(const Encoder&)=delete; Encoder& operator=(const Encoder&)=delete;
  void reset();
  std::optional<Packet> push(const desktop::AudioSamples& samples,Clock::time_point now=Clock::now());
private:
  struct Impl; std::unique_ptr<Impl> impl_;
};
struct Stats { std::uint64_t decoded=0,concealed=0,fecAttempts=0,late=0,dropped=0,invalid=0; };
// One decoder/timeline per speaker; no clock offset assumptions and no global
// sequence number shared with screen sound, video, control or another talker.
class Mixer {
public:
  Mixer(); ~Mixer();
  Mixer(const Mixer&)=delete; Mixer& operator=(const Mixer&)=delete;
  bool receive(Packet packet,Clock::time_point now=Clock::now());
  std::optional<desktop::AudioSamples> poll(Clock::time_point now=Clock::now());
  void mute(unsigned speaker,bool muted);
  void clear();
  Stats stats() const;
private:
  struct Impl; std::unique_ptr<Impl> impl_;
};
} // namespace sanser::voice
