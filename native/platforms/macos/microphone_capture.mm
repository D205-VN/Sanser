#include "microphone_capture.h"
#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <CoreAudio/CoreAudio.h>
#include <atomic>
#include <condition_variable>
#include <cstring>
#include <mutex>
#include <stdexcept>
#include <thread>

namespace sanser::desktop {
namespace {
using Clock=std::chrono::steady_clock;
struct Permission {
  std::mutex mutex;
  std::condition_variable wake;
  bool done=false,granted=false;
};
AudioDeviceID defaultMicrophone() {
  AudioDeviceID device=kAudioObjectUnknown;
  AudioObjectPropertyAddress address{kAudioHardwarePropertyDefaultInputDevice,kAudioObjectPropertyScopeGlobal,kAudioObjectPropertyElementMain};
  UInt32 size=sizeof(device);
  if(AudioObjectGetPropertyData(kAudioObjectSystemObject,&address,0,nullptr,&size,&device)!=noErr) return kAudioObjectUnknown;
  return device;
}
struct Queue {
  AudioQueueRef value=nullptr;
  ~Queue() { if(value) { AudioQueueStop(value,true); AudioQueueDispose(value,true); } }
};
}
struct MicrophoneCapture::State {
  std::function<void(const AudioSamples&)> sink;
  std::function<void(std::string)> status;
  std::atomic<bool> running{true};
  std::atomic<OSStatus> callbackError{noErr};
  std::mutex mutex;
  std::condition_variable wake;
  // Fixed capacity: at most 40 ms of fresh microphone PCM. The Core Audio
  // callback never waits for encoding, network delivery, or permission checks.
  struct Block { AudioSamples samples; Clock::time_point captured; };
  std::array<Block,8> blocks{};
  std::size_t head=0,count=0,used=0;
  AudioSamples pending{};
  std::thread worker;

  State(std::function<void(const AudioSamples&)> output,std::function<void(std::string)> report)
    :sink(std::move(output)),status(std::move(report)),worker([this] { run(); }) {}
  ~State() {
    running=false;
    wake.notify_all();
    if(worker.joinable()) worker.join();
  }
  void report(const char* value) noexcept {
    if(status) try { status(value); } catch(...) { }
  }
  bool authorize() {
    const auto authorization=[AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
    if(authorization==AVAuthorizationStatusAuthorized) return true;
    if(authorization!=AVAuthorizationStatusNotDetermined) { report("permission-denied"); return false; }
    // The completion may arrive after mute/teardown. It owns only this small
    // request state, never the capture instance or a network/session pointer.
    auto result=std::make_shared<Permission>();
    report("permission-requested");
    [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL granted) {
      { std::lock_guard lock(result->mutex); result->done=true; result->granted=granted; }
      result->wake.notify_all();
    }];
    std::unique_lock lock(result->mutex);
    while(running && !result->done) result->wake.wait_for(lock,std::chrono::milliseconds(20));
    if(!running) return false;
    if(!result->granted) { report("permission-denied"); return false; }
    return true;
  }
  static void captured(void* context,AudioQueueRef queue,AudioQueueBufferRef buffer,
                       const AudioTimeStamp*,UInt32,const AudioStreamPacketDescription*) noexcept {
    auto& self=*static_cast<State*>(context);
    if(!self.running) return;
    // AudioQueue supplies requested interleaved float32 mono at 48 kHz. A
    // mono source is duplicated into Sanser's existing stereo wire contract.
    std::unique_lock lock(self.mutex,std::try_to_lock);
    if(lock.owns_lock() && buffer->mAudioDataByteSize%sizeof(float)==0) {
      const auto now=Clock::now();
      const auto* bytes=static_cast<const std::uint8_t*>(buffer->mAudioData);
      const auto frames=buffer->mAudioDataByteSize/sizeof(float);
      for(std::size_t i=0;i<frames;++i) {
        float sample=0; std::memcpy(&sample,bytes+i*sizeof(float),sizeof(float));
        if(!std::isfinite(sample)) sample=0;
        const auto value=static_cast<std::int16_t>(std::clamp(sample,-1.0F,1.0F)*32767);
        self.pending[self.used++]=value;
        self.pending[self.used++]=value;
        if(self.used==self.pending.size()) {
          if(self.count==self.blocks.size()) { self.head=(self.head+1)%self.blocks.size(); --self.count; }
          self.blocks[(self.head+self.count)%self.blocks.size()]={self.pending,now};
          ++self.count; self.used=0;
        }
      }
      lock.unlock();
      self.wake.notify_one();
    } else {
      // Only the serialized input callback owns this partial sample block.
      self.used=0;
    }
    if(self.running) {
      const auto error=AudioQueueEnqueueBuffer(queue,buffer,0,nullptr);
      if(error!=noErr) { self.callbackError=error; self.wake.notify_one(); }
    }
  }
  void capture() {
    const auto selected=defaultMicrophone();
    if(selected==kAudioObjectUnknown) { report("no-device"); return; }
    Queue queue;
    AudioStreamBasicDescription format{};
    format.mSampleRate=audioRate; format.mFormatID=kAudioFormatLinearPCM;
    format.mFormatFlags=kAudioFormatFlagIsFloat|kAudioFormatFlagIsPacked;
    format.mChannelsPerFrame=1; format.mBitsPerChannel=32;
    format.mFramesPerPacket=1; format.mBytesPerFrame=sizeof(float); format.mBytesPerPacket=sizeof(float);
    if(AudioQueueNewInput(&format,captured,this,nullptr,nullptr,0,&queue.value)!=noErr)
      throw std::runtime_error("Microphone input queue");
    for(unsigned i=0;i<3;++i) {
      AudioQueueBufferRef buffer=nullptr;
      if(AudioQueueAllocateBuffer(queue.value,audioFrames*2*sizeof(float),&buffer)!=noErr ||
         AudioQueueEnqueueBuffer(queue.value,buffer,0,nullptr)!=noErr)
        throw std::runtime_error("Microphone input buffer");
    }
    { std::lock_guard lock(mutex); head=count=used=0; }
    callbackError=noErr;
    if(!running) return;
    if(AudioQueueStart(queue.value,nullptr)!=noErr) throw std::runtime_error("Microphone input start");
    report("running");
    auto deviceCheck=Clock::now();
    while(running) {
      std::optional<AudioSamples> samples;
      {
        std::unique_lock lock(mutex);
        wake.wait_for(lock,std::chrono::milliseconds(20),[this] { return !running || count || callbackError!=noErr; });
        const auto now=Clock::now();
        while(count && now-blocks[head].captured>std::chrono::milliseconds(40)) { head=(head+1)%blocks.size(); --count; }
        if(count) { samples=blocks[head].samples; head=(head+1)%blocks.size(); --count; }
      }
      if(!running) break;
      if(callbackError!=noErr) throw std::runtime_error("Microphone input stopped");
      if(samples && sink) sink(*samples);
      if(Clock::now()-deviceCheck>std::chrono::seconds(2)) {
        if(defaultMicrophone()!=selected) { report("device-changed"); break; }
        UInt32 active=0,size=sizeof(active);
        if(AudioQueueGetProperty(queue.value,kAudioQueueProperty_IsRunning,&active,&size)!=noErr || !active)
          throw std::runtime_error("Microphone input inactive");
        deviceCheck=Clock::now();
      }
    }
  }
  void run() noexcept {
    @autoreleasepool {
      report("starting");
      const bool authorized=authorize();
      if(authorized) {
        while(running) {
          try { capture(); } catch(...) { report("unavailable"); }
          // Reopen after unplug/default-device changes. Never spin or allow
          // microphone failure to terminate screen sharing.
          std::unique_lock lock(mutex);
          wake.wait_for(lock,std::chrono::seconds(1),[this] { return !running; });
        }
      }
      // Keep permission-denied as the visible terminal state. A later local
      // unmute creates a new capture and rechecks OS permission.
      if(authorized || !running) report("stopped");
    }
  }
};
MicrophoneCapture::MicrophoneCapture(bool enabled,std::function<void(const AudioSamples&)> sink,
                                   std::function<void(std::string)> status) {
  if(enabled) state_=std::make_unique<State>(std::move(sink),std::move(status));
}
MicrophoneCapture::~MicrophoneCapture()=default;
}
