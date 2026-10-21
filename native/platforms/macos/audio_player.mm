#include "audio_player.h"
#include "audio_playout.h"
#include "voice_echo.h"
#import <AudioToolbox/AudioToolbox.h>
#include <mach/mach_time.h>
#include <cstring>
#include <iostream>
namespace sanser::desktop {
struct AudioPlayer::State {
  AudioQueueRef output=nullptr;
  AudioQueue queue{3};
  sanser::AudioContinuity continuity;
  bool wasSilent=true;
  voice::PlaybackTap reference;
  static void fill(void* context,AudioQueueRef output,AudioQueueBufferRef buffer) {
    auto& self=*static_cast<State*>(context);
    auto* target=static_cast<std::int16_t*>(buffer->mAudioData);
    for(unsigned block=0;block<2;++block) {
      auto samples=self.queue.popBlock();
      auto pcm=samples ? std::move(samples->samples) : AudioSamples{};
      self.continuity.process(pcm.data(),audioFrames,audioChannels,audioRate,!samples || self.wasSilent || samples->discontinuity);
      self.wasSilent=!samples;
      std::memcpy(target+block*pcm.size(),pcm.data(),pcm.size()*sizeof(std::int16_t));
    }
    buffer->mAudioDataByteSize=audioFrames*audioChannels*2*2;
    AudioTimeStamp actual{};
    if(AudioQueueEnqueueBufferWithParameters(output,buffer,0,nullptr,0,0,0,nullptr,nullptr,&actual)==noErr) {
      auto at=voice::EchoClock::now();
      if(actual.mFlags&kAudioTimeStampHostTimeValid) {
        const auto host=mach_absolute_time(); mach_timebase_info_data_t timebase{}; mach_timebase_info(&timebase);
        if(timebase.denom) {
          const auto delta=static_cast<std::int64_t>((static_cast<long double>(actual.mHostTime)-host)*timebase.numer/timebase.denom);
          if(delta>-100000000 && delta<200000000) at+=std::chrono::nanoseconds(delta);
        }
      } else {
        AudioTimeStamp current{};
        if(AudioQueueGetCurrentTime(output,nullptr,&current,nullptr)==noErr &&
           (actual.mFlags&current.mFlags&kAudioTimeStampSampleTimeValid)) {
          const auto delta=static_cast<std::int64_t>((actual.mSampleTime-current.mSampleTime)*1000000000.0/audioRate);
          if(delta>-100000000 && delta<200000000) at+=std::chrono::nanoseconds(delta);
        }
      }
      self.reference.publish(std::span<const std::int16_t>(target,audioFrames*audioChannels*2),at);
    }
  }
  bool start() {
    AudioStreamBasicDescription format{};
    format.mSampleRate=audioRate; format.mFormatID=kAudioFormatLinearPCM;
    format.mFormatFlags=kLinearPCMFormatFlagIsSignedInteger|kLinearPCMFormatFlagIsPacked;
    format.mChannelsPerFrame=audioChannels; format.mBitsPerChannel=16;
    format.mFramesPerPacket=1; format.mBytesPerFrame=audioChannels*2; format.mBytesPerPacket=format.mBytesPerFrame;
    if(AudioQueueNewOutput(&format,fill,this,nullptr,nullptr,0,&output)!=noErr) return false;
    for(unsigned i=0;i<2;++i) {
      AudioQueueBufferRef buffer=nullptr;
      if(AudioQueueAllocateBuffer(output,audioFrames*audioChannels*2*2,&buffer)!=noErr) return false;
      fill(this,output,buffer);
    }
    return AudioQueueStart(output,nullptr)==noErr;
  }
  ~State() { if(output) { AudioQueueStop(output,true); AudioQueueDispose(output,true); } }
};
AudioPlayer::AudioPlayer(bool enabled) {
  if(!enabled) return;
  auto state=std::make_unique<State>();
  if(!state->start()) { std::cerr<<"SNV2_AUDIO_OUTPUT unavailable; video remains active"<<std::endl; return; }
  state_=std::move(state);
  std::cout<<"SNV2_AUDIO_OUTPUT started=1 rate=48000 channels=2 format=pcm16"<<std::endl;
}
AudioPlayer::~AudioPlayer()=default;
void AudioPlayer::submit(AudioSamples samples) { if(state_) state_->queue.push(std::move(samples)); }
void AudioPlayer::reset() { if(state_) state_->queue.clear(); }
AudioQueue::Stats AudioPlayer::stats() const { return state_ ? state_->queue.stats() : AudioQueue::Stats{}; }
}
