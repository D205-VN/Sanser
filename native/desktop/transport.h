#pragma once
#include "snv2.h"
#include "snv2_auth.h"
#include "tx_scheduler.h"
#include "input_delivery_policy.h"
#include "audio.h"
#include <condition_variable>
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
#include <memory>

namespace sanser::desktop {
using Bytes = std::vector<std::uint8_t>;
using Clock = std::chrono::steady_clock;
struct Options {
  bool host = false;
  bool ultraLowLatency = false;
  bool input = true;
  bool audio = true;
  bool sharing = false;
  std::string viewerId;
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
struct Frame { Bytes data; std::uint32_t width = 0, height = 0; bool keyframe = false; std::uint64_t id = 0; };
// Prefix and encoded payload are prepared once, then authenticated per peer.
struct PreparedFrame { Bytes payload; bool keyframe = false; };
PreparedFrame prepareFrame(Frame frame);
// Serialization is work, not a fixed jitter buffer. Large IDRs need enough
// time to pass through the configured pacer before they can be declared stale.
std::chrono::milliseconds videoSendBudget(std::size_t bytes,std::uint32_t bitrate,bool ultra);
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
  explicit Reassembler(std::uint32_t bitrate=20000000,bool ultra=false):bitrate_(bitrate),ultra_(ultra) {}
  std::optional<Frame> push(const snv2::Header& header, const Bytes& payload);
  // Drain dependent pictures in order after a delayed reference is repaired.
  std::optional<Frame> take();
  std::uint64_t lostFragments() const { return lostFragments_; }
  std::uint64_t receivedBytes() const { return receivedBytes_; }
  bool awaitingReference() const { return !haveReference_; }
  struct Repair { std::uint64_t frame; std::vector<std::uint16_t> missing; };
  std::vector<Repair> repairs(bool& expired,Clock::time_point now=Clock::now(),std::chrono::milliseconds retry=std::chrono::milliseconds(20));
  bool assemblingKeyframe() const {
    for(const auto& [id,partial]:frames_) { (void)id; if(partial.keyframe) return true; }
    return false;
  }
private:
  struct Partial { Bytes data; std::vector<bool> seen; std::uint16_t received = 0; Clock::time_point started; bool keyframe = false; Clock::time_point lastRepair{}; unsigned repairAttempts=0; std::size_t highestSeen=0; };
  std::uint32_t bitrate_;
  bool ultra_;
  std::map<std::uint64_t, Partial> frames_;
  std::size_t bufferedBytes_=0;
  std::uint64_t lastFrame_ = 0;
  std::uint64_t lostFragments_=0;
  std::uint64_t receivedBytes_=0;
  bool haveReference_=false;
  void discardThrough(std::uint64_t id,bool lost=false);
};
class Peer {
public:
  explicit Peer(const Options& options);
  ~Peer();
  Peer(const Peer&) = delete;
  Peer& operator=(const Peer&) = delete;
  void start();
  void stop();
  void cancel() { running_=false; sendReady_.notify_all(); }
  bool running() const { return running_.load(); }
  bool ready() const { return ready_.load(); }
  void video(const Frame& frame);
  void videoPrepared(std::shared_ptr<const PreparedFrame> frame);
  void pauseVideo(bool paused);
  struct VideoFeedback {
    std::uint64_t serial=0, deliveredBytes=0;
    std::uint64_t completedFrame=0, sentFrame=0;
    bool congested=false;
    Clock::time_point received{};
  };
  VideoFeedback videoFeedback() const;
  void input(const Input& input);
  void audio(const AudioSamples& samples);
  bool audioReady() const { return options_.audio && options_.host && peerAudio_ && ready_ && running_; }
  void requestKeyframe();
  std::function<void(Frame)> onFrame;
  std::function<void(Input)> onInput;
  std::function<void(AudioSamples)> onAudio;
  std::function<void()> onKeyframe;
  std::function<void()> onStopped;
  std::function<void()> onReady;
  std::uint64_t videoQueueDrops() const { return videoQueueDrops_.load(); }
  std::uint64_t repairedDatagrams() const { return repairedDatagrams_.load(); }
  std::uint64_t repairCacheMisses() const { return repairCacheMisses_.load(); }
  std::uint32_t recommendedBitrate() const { return videoBudget_.load(); }
private:
  void loop();
  void sendLoop();
  void acknowledgeInput();
  void logHealth();
  void transmit(snv2::PacketType type, const Bytes& payload, std::uint32_t stream = 0,
                std::uint64_t frame = 0, std::uint16_t flags = 0, bool reliable = false,
                Clock::time_point deadline = {});
  void datagram(const Bytes& bytes);
  void process(const Bytes& bytes);
  void deliverVideo(std::optional<Frame> frame = {});
  void queueVideoRepair(const Bytes& bytes);
  void serviceVideoRepairs();
  void sendVideoFragment(const PreparedFrame& frame,std::uint64_t id,std::size_t index,bool repair,Clock::time_point deadline = {});
  Options options_;
  PacketCodec codec_;
  std::intptr_t socket_ = -1;
  bool timerResolution_=false;
  std::thread worker_;
  std::thread sender_;
  std::atomic<bool> running_{false}, ready_{false};
  std::mutex sendMutex_, videoMutex_;
  std::condition_variable sendReady_;
  TxScheduler tx_;
  std::atomic<bool> peerSack_{false}, outgoingNeedsKeyframe_{false};
  std::atomic<bool> peerAudio_{false};
  std::atomic<std::uint64_t> queuedKeys_{0},queuedButtons_{0};
  std::uint64_t deliveredKeys_=0,deliveredButtons_=0;
  std::uint64_t lastAudio_=0;
  std::atomic<bool> videoPaused_{false};
  mutable std::mutex feedbackMutex_;
  VideoFeedback feedback_{};
  std::atomic<std::uint64_t> videoQueueDrops_{0};
  std::atomic<std::uint64_t> repairedDatagrams_{0}, repairCacheMisses_{0};
  std::atomic<std::uint64_t> offeredBytes_{0};
  std::atomic<std::uint32_t> videoBudget_{0};
  std::uint64_t receivedVideoDatagrams_=0, lastReportedBytes_=0, lastReportedLoss_=0, lastFeedbackDrops_=0;
  unsigned congestionWindows_=0, recoveryWindows_=0, busyWindows_=0;
  bool peerVideoProgress_=false;
  Clock::time_point lastVideoFeedback_{};
  Clock::time_point lastVideoProgress_{};
  std::uint32_t generation_ = 0, peerGeneration_ = 0;
  std::set<std::uint32_t> previousGenerations_;
  std::uint64_t lastVideo_ = 0, lastMouse_ = 0;
  std::uint64_t sequences_[3]{}, deliveredInput_ = 0, completedVideoFrame_=0;
  std::atomic<std::uint64_t> frame_{0};
  ReplayWindow replay_[3];
  struct Pending { Bytes bytes; Clock::time_point sent, created; unsigned attempts; bool sacked = false; };
  std::map<std::uint64_t, Pending> pending_;
  InputDeliveryPolicy inputDelivery_;
  std::mutex keyframeRequestMutex_;
  std::map<std::uint64_t, Input> receivedInputs_;
  Reassembler reassembler_;
  struct CachedVideo { std::shared_ptr<const PreparedFrame> frame; Clock::time_point sent; std::deque<std::uint16_t> repair; Clock::time_point lastRequest{}; };
  std::map<std::uint64_t,CachedVideo> videoCache_;
  std::size_t videoCacheBytes_=0;
  Clock::time_point lastReceived_ = Clock::now(), lastKeyframe_{};
  Clock::time_point lastVideoReceived_ = Clock::now();
  bool needKeyframe_ = true;
  std::uint64_t rawDatagrams_=0, authRejected_=0, videoDatagrams_=0, completedFrames_=0, keyframeWaitDropped_=0, nackSent_=0;
  std::atomic<unsigned> stopReason_{0};
};
struct PeerStop { Peer& peer; ~PeerStop() { peer.stop(); } };
}
