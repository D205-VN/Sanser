#include "audio_capture.h"
#include <iostream>
#include <stdexcept>
using namespace sanser::desktop;
void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
void checkLayout(bool planar) {
  std::vector<AudioSamples> packets;
  AudioPacketizer packetizer([&](const AudioSamples& samples) { packets.push_back(samples); });
  AudioStreamBasicDescription asbd{}; asbd.mSampleRate=audioRate; asbd.mFormatID=kAudioFormatLinearPCM;
  asbd.mFormatFlags=kAudioFormatFlagIsFloat|kAudioFormatFlagIsPacked|(planar ? kAudioFormatFlagIsNonInterleaved : 0);
  asbd.mChannelsPerFrame=2; asbd.mBitsPerChannel=32; asbd.mFramesPerPacket=1;
  asbd.mBytesPerFrame=planar ? 4 : 8; asbd.mBytesPerPacket=asbd.mBytesPerFrame;
  CMAudioFormatDescriptionRef format=nullptr;
  require(CMAudioFormatDescriptionCreate(nullptr,&asbd,0,nullptr,0,nullptr,nullptr,&format)==noErr,"Create float PCM description");
  struct FormatStop { CMAudioFormatDescriptionRef value; ~FormatStop() { CFRelease(value); } } formatStop{format};
  CMSampleBufferRef sample=nullptr;
  CMSampleTimingInfo timing{CMTimeMake(1,audioRate),kCMTimeZero,kCMTimeInvalid};
  require(CMSampleBufferCreate(nullptr,nullptr,true,nullptr,nullptr,format,audioFrames*2,1,&timing,0,nullptr,&sample)==noErr,"Create audio sample");
  struct SampleStop { CMSampleBufferRef value; ~SampleStop() { CFRelease(value); } } sampleStop{sample};
  std::vector<float> first(planar ? audioFrames*2 : audioFrames*4),second(audioFrames*2,-0.25F);
  for(std::size_t i=0;i<first.size();++i) first[i]=planar || i%2==0 ? 0.5F : -0.25F;
  struct StereoBuffers { UInt32 count; AudioBuffer buffers[2]; } storage{};
  storage.count=planar ? 2 : 1;
  storage.buffers[0]={planar ? 1U : 2U,static_cast<UInt32>(first.size()*sizeof(float)),first.data()};
  storage.buffers[1]={1,static_cast<UInt32>(second.size()*sizeof(float)),second.data()};
  require(CMSampleBufferSetDataBufferFromAudioBufferList(sample,nullptr,nullptr,0,reinterpret_cast<AudioBufferList*>(&storage))==noErr,"Set PCM data");
  captureSystemAudio(sample,packetizer);
  require(packets.size()==2,"CoreMedia callback did not produce complete audio packets");
  for(const auto& block:packets) for(std::size_t i=0;i<block.size();i+=2)
    require(block[i]==16383 && block[i+1]==-8191,"CoreMedia stereo layout conversion differs");
}
int main() {
  @autoreleasepool {
    try { checkLayout(false); checkLayout(true); std::cout<<"ScreenCaptureKit audio conversion: planar and interleaved samples passed\n"; return 0; }
    catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
  }
}
