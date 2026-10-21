#include "audio_capture.h"
#include <cstring>

namespace sanser::desktop {
void captureSystemAudio(CMSampleBufferRef sample,AudioPacketizer& output,voice::PlaybackTap* reference) {
  if(!sample || !CMSampleBufferIsValid(sample) || !CMSampleBufferDataIsReady(sample)) return;
  const auto format=CMSampleBufferGetFormatDescription(sample);
  const auto* asbd=format ? CMAudioFormatDescriptionGetStreamBasicDescription(format) : nullptr;
  if(!asbd || asbd->mFormatID!=kAudioFormatLinearPCM || asbd->mSampleRate!=audioRate ||
     asbd->mChannelsPerFrame!=audioChannels || asbd->mBitsPerChannel!=32 ||
     !(asbd->mFormatFlags&kAudioFormatFlagIsFloat) || (asbd->mFormatFlags&kAudioFormatFlagIsBigEndian)) return;
  const auto frames=CMSampleBufferGetNumSamples(sample);
  if(frames<=0 || frames>audioRate) return;
  std::size_t size=0;
  if(CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(sample,&size,nullptr,0,nullptr,nullptr,0,nullptr)!=noErr ||
     size<sizeof(AudioBufferList) || size>sizeof(AudioBufferList)+sizeof(AudioBuffer)*audioChannels) return;
  std::vector<std::uint64_t> storage((size+7)/8);
  auto* buffers=reinterpret_cast<AudioBufferList*>(storage.data()); CMBlockBufferRef retained=nullptr;
  const auto status=CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(sample,nullptr,buffers,size,nullptr,nullptr,0,&retained);
  struct Release { CMBlockBufferRef value; ~Release() { if(value) CFRelease(value); } } release{retained};
  if(status!=noErr) return;
  const bool planar=(asbd->mFormatFlags&kAudioFormatFlagIsNonInterleaved)!=0;
  if(buffers->mNumberBuffers!=(planar ? 2U : 1U) || asbd->mBytesPerFrame!=(planar ? 4U : 8U)) return;
  for(unsigned i=0;i<buffers->mNumberBuffers;++i) {
    const auto& buffer=buffers->mBuffers[i];
    if(!buffer.mData || buffer.mNumberChannels!=(planar ? 1U : 2U) || buffer.mDataByteSize<static_cast<std::size_t>(frames)*asbd->mBytesPerFrame) return;
  }
  std::vector<std::int16_t> rendered;
  if(reference) rendered.reserve(static_cast<std::size_t>(frames)*audioChannels);
  for(CMItemCount i=0;i<frames;++i) {
    float left=0,right=0;
    const auto* first=static_cast<const std::uint8_t*>(buffers->mBuffers[0].mData);
    const auto* second=planar ? static_cast<const std::uint8_t*>(buffers->mBuffers[1].mData) : first;
    std::memcpy(&left,first+i*asbd->mBytesPerFrame,sizeof(float));
    std::memcpy(&right,second+i*asbd->mBytesPerFrame+(planar ? 0 : sizeof(float)),sizeof(float));
    output.sample(left,right);
    if(reference) for(float value:{left,right}) rendered.push_back(static_cast<std::int16_t>(
      std::clamp(std::isfinite(value) ? value : 0.0F,-1.0F,1.0F)*32767));
  }
  if(reference) {
    const auto now=voice::EchoClock::now();
    auto first=now-std::chrono::microseconds(static_cast<std::uint64_t>(frames)*1000000/audioRate);
    const auto pts=CMSampleBufferGetPresentationTimeStamp(sample);
    const auto age=CMTimeGetSeconds(CMTimeSubtract(CMClockGetTime(CMClockGetHostTimeClock()),pts));
    if(std::isfinite(age) && age>=-0.2 && age<1.0)
      first=now-std::chrono::nanoseconds(static_cast<std::int64_t>(age*1000000000));
    reference->publish(rendered,first);
  }
}
}
