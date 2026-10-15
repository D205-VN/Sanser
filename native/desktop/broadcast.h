#pragma once
#include "transport.h"
#include <memory>

namespace sanser::desktop {
struct ViewerSpec {
  std::string id;
  std::string endpoint;
  std::uint16_t localPort=0;
  std::string token;
  // Send budget per viewer. Does not create an additional encoder.
  std::uint32_t bitrate=0;
};
struct ViewerStats {
  std::string id;
  bool ready=false, running=false;
  std::uint64_t submitted=0, dispatched=0, replaced=0, referenceDrops=0, sendQueueDrops=0, repaired=0, repairCacheMisses=0;
  std::uint64_t keyframeSignals=0, keyframeRequests=0;
  std::uint32_t recommendedBitrate=0;
  std::string videoState="active";
  std::uint64_t pauses=0;
  unsigned backoffMs=0;
};

// One prepared encoded payload, independent crypto/queues/workers per viewer.
// add/remove/stop are serialized; publish never waits for a peer's socket.
class Broadcaster {
public:
  explicit Broadcaster(Options source);
  ~Broadcaster();
  Broadcaster(const Broadcaster&)=delete;
  Broadcaster& operator=(const Broadcaster&)=delete;
  void add(ViewerSpec viewer);
  void remove(const std::string& id);
  void stop();
  void publish(Frame frame);
  void updateHealth(Clock::time_point now=Clock::now());
  bool ready() const;
  bool running() const { return running_.load(); }
  bool takeKeyframeRequest();
  std::uint32_t bitrate() const;
  std::vector<ViewerStats> stats() const;
  std::uint64_t publishedFrames() const { return published_.load(); }
private:
  struct Delivery;
  Options source_;
  mutable std::mutex peersMutex_;
  std::mutex lifecycleMutex_, publishMutex_, keyframeMutex_;
  std::map<std::string,std::shared_ptr<Delivery>> peers_;
  std::atomic<bool> running_{true};
  std::atomic<std::uint64_t> published_{0};
  Clock::time_point lastKeyframeRequest_{};
};

// Single-peer control and multi-viewer share hosts use the same capture/codec.
class HostOutput {
public:
  explicit HostOutput(const Options& options);
  ~HostOutput();
  void start();
  void stop();
  bool ready() const;
  bool running() const;
  void video(Frame frame);
  void audio(const AudioSamples& samples);
  bool takeKeyframeRequest();
  void pumpControl();
  std::uint32_t bitrate() const;
  std::function<void(Input)> onInput;
private:
  Options options_;
  std::unique_ptr<Peer> single_;
  std::unique_ptr<Broadcaster> broadcast_;
  std::atomic<bool> keyframeRequested_{true};
  Clock::time_point lastStats_{};
};
struct HostOutputStop { HostOutput& output; ~HostOutputStop() { output.stop(); } };

struct SharingCommand {
  enum Kind { Add, Remove } kind;
  std::string requestId;
  ViewerSpec viewer;
};
std::optional<SharingCommand> parseSharingCommand(const std::string& line, std::uint32_t bitrate);
std::vector<std::string> takeNativeCommands();
bool nativeStopRequested();
}
