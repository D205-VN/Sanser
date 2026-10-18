#include "audio_player.h"
#include "audio_playout.h"
#import <AudioToolbox/AudioToolbox.h>
#include <cstring>
#include <iostream>
namespace sanser::desktop {
struct AudioPlayer::State {
  AudioQueueRef output=nullptr;
  AudioQueue queue{3};
  sanser::AudioContinuity continuity;
  bool wasSilent=true;
  static void fill(void* context,AudioQueueRef output,AudioQueueBufferRef buffer) {
    auto& self=*static_cast<State*>(context);
    auto* target=static_cast<std::int16_t*>(buffer->mAudioData);
    for(unsigned block=0;block<2;++block) {
      auto samples=self.queue.pop();
      auto pcm=samples.value_or(AudioSamples{});
      self.continuity.process(pcm.data(),audioFrames,audioChannels,audioRate,!samples || self.wasSilent);
      self.wasSilent=!samples;
      std::memcpy(target+block*pcm.size(),pcm.data(),pcm.size()*sizeof(std::int16_t));
    }
    buffer->mAudioDataByteSize=audioFrames*audioChannels*2*2;
    AudioQueueEnqueueBuffer(output,buffer,0,nullptr);
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
}
