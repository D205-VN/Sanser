#pragma once
#include "snv2.h"
#include "snv2_auth.h"
#include <atomic>
#include <chrono>
#include <functional>
#include <map>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <vector>
#include <bitset>
#include <set>

namespace sanser::desktop {
using Bytes = std::vector<std::uint8_t>;
using Clock = std::chrono::steady_clock;
struct Options {
  bool host = false;
  bool input = true;
  std::string peer;
  std::uint16_t port = 0;
  std::uint32_t width = 1920, height = 1080, fps = 60, bitrate = 20000000;
  std::string token;
};
Options parseOptions(bool host, int argc, char** argv);
struct Input {
  // Absolute position uses [0,65535]; key codes use USB HID keyboard usages.
  enum Kind : std::uint8_t { Move, Button, Key, Scroll, Reset };
  Kind kind = Reset;
  std::uint16_t x = 0, y = 0, code = 0;
  std::int16_t delta = 0;
  bool down = false;
};
Bytes encodeInput(const Input& input);
std::optional<Input> decodeInput(const Bytes& bytes);
struct Frame { Bytes data; std::uint32_t width = 0, height = 0; bool keyframe = false; };
class PacketCodec {
public:
  PacketCodec(const std::string& token, bool host);
  Bytes seal(snv2::Header header, const Bytes& payload) const;
  std::optional<std::pair<snv2::Header, Bytes>> open(const Bytes& packet) const;
private:
  snv2::DirectionKey sendAuth_{}, receiveAuth_{}, sendCipher_{}, receiveCipher_{};
  snv2::SessionId session_{};
};
class ReplayWindow {
public:
  bool accept(std::uint64_t sequence);
private:
  std::uint64_t newest_ = 0;
  std::bitset<2048> seen_;
};
class Reassembler {
public:
  std::optional<Frame> push(const snv2::Header& header, const Bytes& payload);
private:
  struct Partial { Bytes data; std::vector<bool> seen; std::uint16_t received = 0; Clock::time_point started; bool keyframe = false; };
  std::map<std::uint64_t, Partial> frames_;
  std::uint64_t lastFrame_ = 0;
};
class Peer {
public:
  explicit Peer(const Options& options);
  ~Peer();
  Peer(const Peer&) = delete;
  Peer& operator=(const Peer&) = delete;
  void start();
  void stop();
  bool running() const { return running_.load(); }
  bool ready() const { return ready_.load(); }
  void video(const Frame& frame);
  void input(const Input& input);
  void requestKeyframe();
  std::function<void(Frame)> onFrame;
  std::function<void(Input)> onInput;
  std::function<void()> onKeyframe;
  std::function<void()> onStopped;
private:
  void loop();
  void transmit(snv2::PacketType type, const Bytes& payload, std::uint32_t stream = 0,
                std::uint64_t frame = 0, std::uint16_t flags = 0, bool reliable = false);
  void datagram(const Bytes& bytes);
  void process(const Bytes& bytes);
  Options options_;
  PacketCodec codec_;
  std::intptr_t socket_ = -1;
  std::thread worker_;
  std::atomic<bool> running_{false}, ready_{false};
  std::mutex sendMutex_, videoMutex_;
  std::uint32_t generation_ = 0, peerGeneration_ = 0;
  std::set<std::uint32_t> previousGenerations_;
  std::uint64_t lastVideo_ = 0, lastMouse_ = 0;
  std::uint64_t sequences_[3]{}, frame_ = 0, deliveredInput_ = 0;
  ReplayWindow replay_[3];
  struct Pending { Bytes bytes; Clock::time_point sent; unsigned attempts; };
  std::map<std::uint64_t, Pending> pending_;
  std::map<std::uint64_t, Input> receivedInputs_;
  Reassembler reassembler_;
  Clock::time_point lastReceived_ = Clock::now(), lastKeyframe_{};
  bool needKeyframe_ = true;
};
struct PeerStop { Peer& peer; ~PeerStop() { peer.stop(); } };
}
