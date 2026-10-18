#pragma once
#include <array>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <functional>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <span>
#include <thread>
#include <vector>

namespace sanser::audio {
inline constexpr unsigned rate=48000,channels=2,frames=240;
using Samples=std::array<std::int16_t,frames*channels>;
using Bytes=std::vector<std::uint8_t>;
class Encoder {
public:
  Encoder(); ~Encoder();
  Encoder(const Encoder&)=delete; Encoder& operator=(const Encoder&)=delete;
  Bytes encode(const Samples& samples);
  void reset();
private:
  struct State; std::unique_ptr<State> state_;
};
class Decoder {
public:
  Decoder(); ~Decoder();
  Decoder(const Decoder&)=delete; Decoder& operator=(const Decoder&)=delete;
  std::optional<Samples> decode(std::span<const std::uint8_t> packet);
  Samples conceal();
  void reset();
private:
  struct State; std::unique_ptr<State> state_;
};
// Independent audio numbering: the transport's control sequence is NOT an
// audio loss counter. Timestamp is a 48kHz sample index, never a remote clock.
struct Packet { std::uint64_t sequence=0,timestamp=0,unixMicros=0; Bytes payload; };
Bytes pack(const Packet& packet);
std::optional<Packet> unpack(std::span<const std::uint8_t> bytes);
struct Decoded { Samples samples; std::uint64_t sequence=0,unixMicros=0,generation=0; bool concealed=false; };
struct Stats { std::uint64_t decoded=0,concealed=0,late=0,reordered=0,dropped=0,errors=0; };

// Pure playout policy, called under ReceiveWorker's mutex (or with a virtual
// clock in tests). 10ms reorder reserve, 40ms freshness, <=30ms consecutive PLC.
class Jitter {
public:
  using Clock=std::chrono::steady_clock;
  bool push(Packet packet,Clock::time_point now=Clock::now());
  std::optional<Decoded> poll(Clock::time_point now=Clock::now());
  void reset();
  Stats stats() const { return stats_; }
  bool active() const { return started_ || !packets_.empty(); }
private:
  struct Entry { Packet packet; Clock::time_point received; };
  Decoder decoder_;
  std::map<std::uint64_t,Entry> packets_;
  Clock::time_point first_{},due_{};
  std::uint64_t expected_=0,lastSequence_=0,lastUnix_=0;
  unsigned missing_=0; bool started_=false;
  Stats stats_;
};
class ReceiveWorker {
public:
  using Sink=std::function<void(Decoded)>;
  explicit ReceiveWorker(Sink sink);
  ~ReceiveWorker();
  bool push(Packet packet,std::uint64_t generation);
  void reset(std::uint64_t generation);
  Stats stats() const;
private:
  void run();
  Sink sink_; mutable std::mutex mutex_; std::condition_variable wake_;
  Jitter jitter_; std::uint64_t generation_=0; bool stopped_=false;
  std::thread worker_;
};
// Streaming float stereo -> 48kHz -> fixed 5ms blocks. Carries fractional
// resampling position and partial blocks across capture callback boundaries.
class Packetizer {
public:
  explicit Packetizer(std::function<void(const Samples&)> sink):sink_(std::move(sink)) {}
  bool push(std::span<const float> stereo,unsigned inputRate);
  void reset();
private:
  void sample(float left,float right);
  std::function<void(const Samples&)> sink_;
  Samples samples_{}; unsigned used_=0,inputRate_=0;
  std::array<float,2> previous_{}; bool hasPrevious_=false;
  double untilNext_=0;
};
}
