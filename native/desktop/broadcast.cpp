#include "broadcast.h"
#include "broadcast_recovery.h"
#include "broadcast_policy.h"
#include <algorithm>
#include <cctype>
#include <iostream>
#include <sstream>
#include <stdexcept>

namespace sanser::desktop {
namespace {
bool validId(const std::string& id) {
  return !id.empty() && id.size()<=64 && std::all_of(id.begin(),id.end(),[](unsigned char c) { return std::isalnum(c)||c=='-'||c=='_'; });
}
bool validToken(const std::string& token) {
  return token.size()>=32 && token.size()<=256 && std::all_of(token.begin(),token.end(),[](unsigned char c) { return std::isalnum(c)||c=='-'||c=='_'; });
}
}

struct Broadcaster::Delivery {
  ViewerSpec spec;
  Peer peer;
  std::mutex mutex;
  std::condition_variable wake;
  std::shared_ptr<const PreparedFrame> pending;
  std::thread worker;
  std::atomic<bool> running{true};
  bool awaitingKeyframe=true;
  std::atomic<std::uint64_t> submitted{0}, dispatched{0}, replaced{0}, referenceDrops{0};
  SharingKeyframeRecovery recovery;
  SharingDeliveryPolicy policy;
  Delivery(ViewerSpec viewer,Options options)
      :spec(std::move(viewer)),peer(options) {
    const auto request=[this] {
      std::lock_guard lock(mutex);
      if(!policy.sendsVideo()) return;
      awaitingKeyframe=true; recovery.request(Clock::now());
    };
    peer.onKeyframe=request;
    peer.onReady=request;
    // No input injector callback is installed. Peer also enforces input=false.
  }
  ~Delivery() { stop(); }
  void start() {
    peer.start();
    try { worker=std::thread([this] { loop(); }); }
    catch(...) { peer.stop(); throw; }
  }
  void stop() {
    running=false; wake.notify_all();
    if(worker.joinable()) worker.join();
    peer.stop();
    std::lock_guard lock(mutex); pending.reset();
  }
  void submit(const std::shared_ptr<const PreparedFrame>& frame) {
    if(!running || !peer.running() || !peer.ready()) return;
    std::lock_guard lock(mutex);
    if(!policy.sendsVideo()) return;
    ++submitted;
    if(pending) {
      ++replaced;
      // Replacing a reference with an IDR heals the chain; it must not demand
      // yet another group-wide IDR. A P-frame replacement still needs recovery.
      if(!frame->keyframe) { awaitingKeyframe=true; recovery.request(Clock::now()); }
    }
    if(frame->keyframe) recovery.keyframe();
    pending=frame;
    wake.notify_one();
  }
  void loop() {
    try {
      while(running && peer.running()) {
        std::shared_ptr<const PreparedFrame> frame;
        {
          std::unique_lock lock(mutex);
          wake.wait_for(lock,std::chrono::milliseconds(20),[&] { return !running || pending; });
          if(!running) break;
          frame=std::move(pending);
          if(!frame) continue;
          if(awaitingKeyframe && !frame->keyframe) { ++referenceDrops; recovery.request(Clock::now()); continue; }
          if(frame->keyframe) awaitingKeyframe=false;
        }
        peer.videoPrepared(frame);
        ++dispatched;
      }
    } catch(...) {}
    running=false; peer.cancel();
  }
  bool takeKeyframeRequest(Clock::time_point now) {
    std::lock_guard lock(mutex);
    return policy.sendsVideo() && running && peer.running() && peer.ready() && recovery.take(now);
  }
  void updateHealth(Clock::time_point now) {
    std::lock_guard lock(mutex);
    if(!running || !peer.running() || !peer.ready()) return;
    const auto feedback=peer.videoFeedback();
    const auto previous=policy.state();
    policy.observe(now,{submitted.load(),replaced.load(),referenceDrops.load(),peer.videoQueueDrops(),
      recovery.backoffMs(now),feedback.serial,feedback.deliveredBytes,
      feedback.completedFrame,feedback.sentFrame,
      feedback.serial>0 && now-feedback.received<std::chrono::seconds(2),feedback.congested});
    if(previous==policy.state()) return;
    recovery.restart();
    if(policy.state()==SharingDeliveryPolicy::Active) return;
    pending.reset(); awaitingKeyframe=true;
    peer.pauseVideo(policy.state()==SharingDeliveryPolicy::Paused);
    if(policy.state()==SharingDeliveryPolicy::Probing) recovery.request(now);
  }
  bool affectsBitrate() {
    std::lock_guard lock(mutex);
    return policy.affectsBitrate() && running && peer.running() && peer.ready();
  }
  bool probing() {
    std::lock_guard lock(mutex);
    return policy.state()==SharingDeliveryPolicy::Probing && running && peer.running() && peer.ready();
  }
  bool videoReady() {
    std::lock_guard lock(mutex);
    return policy.sendsVideo() && running && peer.running() && peer.ready();
  }
  ViewerStats stats() {
    std::lock_guard lock(mutex);
    return {spec.id,peer.ready(),peer.running() && running,
      submitted.load(),dispatched.load(),replaced.load(),referenceDrops.load(),
      peer.videoQueueDrops(),peer.repairedDatagrams(),peer.repairCacheMisses(),
      recovery.signals(),recovery.requests(),peer.recommendedBitrate(),policy.label(),policy.pauses(),recovery.backoffMs(Clock::now())};
  }
};

Broadcaster::Broadcaster(Options source):source_(std::move(source)) {
  if(!source_.host || source_.input || source_.bitrate<1000000 || source_.bitrate>200000000)
    throw std::runtime_error("Sharing host must be view-only with a bounded bitrate");
}
Broadcaster::~Broadcaster() { stop(); }
void Broadcaster::add(ViewerSpec viewer) {
  std::lock_guard lifecycle(lifecycleMutex_);
  if(!running_ || !validId(viewer.id) || !validToken(viewer.token) || viewer.localPort==0 ||
     viewer.bitrate<1000000 || viewer.bitrate>source_.bitrate) throw std::runtime_error("Invalid sharing peer configuration");
  std::lock_guard lock(peersMutex_);
  if(peers_.size()>=3 || peers_.contains(viewer.id)) throw std::runtime_error("Sharing viewer limit or duplicate viewer");
  for(const auto& [id,peer]:peers_) {
    (void)id;
    if(peer->spec.localPort==viewer.localPort || peer->spec.endpoint==viewer.endpoint || peer->spec.token==viewer.token)
      throw std::runtime_error("Each sharing viewer requires an independent endpoint, port and credential");
  }
  auto options=source_; options.token=viewer.token; options.peer=viewer.endpoint; options.port=viewer.localPort; options.bitrate=viewer.bitrate;
  auto delivery=std::make_shared<Delivery>(std::move(viewer),options);
  delivery->start();
  try { peers_.emplace(delivery->spec.id,delivery); }
  catch(...) { delivery->stop(); throw; }
}
void Broadcaster::remove(const std::string& id) {
  std::lock_guard lifecycle(lifecycleMutex_);
  std::shared_ptr<Delivery> removed;
  { std::lock_guard lock(peersMutex_); const auto it=peers_.find(id); if(it==peers_.end()) return; removed=it->second; peers_.erase(it); }
  removed->stop();
}
void Broadcaster::stop() {
  std::lock_guard lifecycle(lifecycleMutex_);
  running_=false;
  std::map<std::string,std::shared_ptr<Delivery>> peers;
  { std::lock_guard lock(peersMutex_); peers.swap(peers_); }
  for(auto& [id,peer]:peers) { (void)id; peer->stop(); }
}
void Broadcaster::publish(Frame frame) {
  if(!running_) return;
  std::lock_guard order(publishMutex_);
  const auto prepared=std::make_shared<const PreparedFrame>(prepareFrame(std::move(frame)));
  std::lock_guard lock(peersMutex_);
  ++published_;
  for(const auto& [id,peer]:peers_) { (void)id; peer->submit(prepared); }
}
void Broadcaster::updateHealth(Clock::time_point now) {
  std::lock_guard lock(peersMutex_);
  for(const auto& [id,peer]:peers_) { (void)id; peer->updateHealth(now); }
}
bool Broadcaster::ready() const {
  std::lock_guard lock(peersMutex_);
  return std::any_of(peers_.begin(),peers_.end(),[](const auto& entry) { return entry.second->videoReady(); });
}
bool Broadcaster::takeKeyframeRequest() {
  std::lock_guard keyframeLock(keyframeMutex_);
  const auto now=Clock::now();
  if(now-lastKeyframeRequest_<std::chrono::milliseconds(source_.ultraLowLatency ? 100 : 300)) return false;
  std::lock_guard peersLock(peersMutex_);
  bool requested=false;
  // Consume all eligible viewers together, coalescing them into one encode.
  for(const auto& [id,peer]:peers_) { (void)id; requested=peer->takeKeyframeRequest(now) || requested; }
  if(requested) lastKeyframeRequest_=now;
  return requested;
}
std::vector<ViewerStats> Broadcaster::stats() const {
  std::lock_guard lock(peersMutex_); std::vector<ViewerStats> result;
  for(const auto& [id,peer]:peers_) { (void)id; result.push_back(peer->stats()); }
  return result;
}
std::uint32_t Broadcaster::bitrate() const {
  std::lock_guard lock(peersMutex_); auto budget=source_.bitrate; bool active=false;
  for(const auto& [id,peer]:peers_) {
    (void)id;
    if(peer->affectsBitrate()) { active=true; budget=std::min(budget,peer->peer.recommendedBitrate()); }
  }
  // With no healthy viewers to protect, probe at the last known usable budget
  // rather than repeatedly saturating every weak link with the source target.
  if(!active) for(const auto& [id,peer]:peers_) {
    (void)id;
    if(peer->probing()) budget=std::min(budget,peer->peer.recommendedBitrate());
  }
  return budget;
}

std::optional<SharingCommand> parseSharingCommand(const std::string& line,std::uint32_t bitrate) {
  if(line.size()>1024 || line.find('\0')!=std::string::npos) return std::nullopt;
  std::vector<std::string> parts; std::istringstream input(line); std::string part;
  while(std::getline(input,part,'\t')) parts.push_back(part);
  if(parts.size()==3 && parts[0]=="share-remove" && validId(parts[1]) && validId(parts[2])) return SharingCommand{SharingCommand::Remove,parts[1],{parts[2],{},0,{},0}};
  if(parts.size()!=6 || parts[0]!="share-add" || !validId(parts[1]) || !validId(parts[2]) || !validToken(parts[5])) return std::nullopt;
  try {
    std::size_t used=0; const auto port=std::stoul(parts[3],&used);
    if(used!=parts[3].size() || port==0 || port>65535) return std::nullopt;
    return SharingCommand{SharingCommand::Add,parts[1],{parts[2],parts[4],static_cast<std::uint16_t>(port),parts[5],bitrate}};
  } catch(...) { return std::nullopt; }
}
HostOutput::HostOutput(const Options& options):options_(options) {
  if(options.sharing) {
    broadcast_=std::make_unique<Broadcaster>(options);
  } else {
    single_=std::make_unique<Peer>(options);
    single_->onInput=[this](Input input) { if(onInput) onInput(input); };
    single_->onKeyframe=[this] { keyframeRequested_=true; };
    single_->onReady=[this] { keyframeRequested_=true; };
  }
}
HostOutput::~HostOutput() { stop(); }
void HostOutput::start() {
  if(broadcast_) {
    broadcast_->add({options_.viewerId,options_.peer,options_.port,options_.token,options_.bitrate});
    std::cout<<"SNSHARE_ACK request="<<options_.viewerId<<" accepted=1"<<std::endl;
  }
  else single_->start();
}
void HostOutput::stop() { if(broadcast_) broadcast_->stop(); else single_->stop(); }
bool HostOutput::ready() const { return broadcast_ ? broadcast_->ready() : single_->ready(); }
bool HostOutput::running() const { return !nativeStopRequested() && (broadcast_ ? broadcast_->running() : single_->running()); }
void HostOutput::video(Frame frame) { if(broadcast_) broadcast_->publish(std::move(frame)); else single_->video(frame); }
void HostOutput::audio(const AudioSamples& samples) { if(single_) single_->audio(samples); }
std::uint32_t HostOutput::bitrate() const { return broadcast_ ? broadcast_->bitrate() : single_->recommendedBitrate(); }
bool HostOutput::takeKeyframeRequest() { return broadcast_ ? broadcast_->takeKeyframeRequest() : keyframeRequested_.exchange(false); }
void HostOutput::pumpControl() {
  if(broadcast_) broadcast_->updateHealth();
  for(const auto& line:takeNativeCommands()) {
    const auto command=parseSharingCommand(line,options_.bitrate);
    try {
      if(!broadcast_ || !command) throw std::runtime_error("Invalid sharing control command");
      if(command->kind==SharingCommand::Add) broadcast_->add(command->viewer);
      else broadcast_->remove(command->viewer.id);
      const auto peers=broadcast_->stats();
      const auto ready=std::count_if(peers.begin(),peers.end(),[](const auto& peer) { return peer.ready && peer.running; });
      std::cout<<"SNSHARE_STATUS viewers="<<peers.size()<<" readyViewers="<<ready<<" publishedFrames="<<broadcast_->publishedFrames()<<std::endl;
      std::cout<<"SNSHARE_ACK request="<<command->requestId<<" accepted=1"<<std::endl;
    } catch(...) {
      if(command) std::cout<<"SNSHARE_ACK request="<<command->requestId<<" accepted=0"<<std::endl;
      std::cerr<<"SNSHARE_COMMAND rejected=1"<<std::endl;
    }
  }
  if(broadcast_ && Clock::now()-lastStats_>=std::chrono::seconds(1)) {
    lastStats_=Clock::now();
    for(const auto& peer:broadcast_->stats()) {
      std::cout<<"SNSHARE_PEER viewer="<<peer.id<<" ready="<<peer.ready<<" running="<<peer.running
        <<" submitted="<<peer.submitted<<" dispatched="<<peer.dispatched<<" replaced="<<peer.replaced
        <<" referenceDrops="<<peer.referenceDrops<<" sendQueueDrops="<<peer.sendQueueDrops
        <<" keyframeSignals="<<peer.keyframeSignals<<" keyframeRequests="<<peer.keyframeRequests
        <<" repaired="<<peer.repaired<<" repairCacheMisses="<<peer.repairCacheMisses
        <<" bitrate="<<peer.recommendedBitrate<<" videoState="<<peer.videoState
        <<" pauses="<<peer.pauses<<" backoffMs="<<peer.backoffMs<<'\n';
    }
    std::cout.flush();
  }
}
}
