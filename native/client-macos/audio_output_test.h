#pragma once
// Synthetic Core Audio adapter: exercise the production player without opening
// an output endpoint. Time, device stops and callback completion are controlled.
struct FakeAudioOutput {
  struct Queue {
    AudioQueueOutputCallback callback; void* context;
    bool running=false,disposed=false;
    std::vector<std::unique_ptr<std::uint8_t[]>> storage;
    std::vector<std::unique_ptr<AudioQueueBuffer>> buffers;
    std::deque<AudioQueueBufferRef> pending;
  };
  inline static FakeAudioOutput* active=nullptr;
  std::vector<std::unique_ptr<Queue>> queues;
  std::chrono::steady_clock::time_point time=std::chrono::steady_clock::time_point{}+std::chrono::seconds(1);
  bool immediate=false,emptyStart=false,clearReturnedSize=false;
  OSStatus enqueueError=noErr,startError=noErr;
  unsigned starts=0;
  FakeAudioOutput() { active=this; }
  ~FakeAudioOutput() { active=nullptr; }
  Queue& current() { return *queues.back(); }
  static Queue& cast(AudioQueueRef queue) { return *reinterpret_cast<Queue*>(queue); }
  void complete() {
    auto& queue=current();
    if(queue.pending.empty()) return;
    const auto buffer=queue.pending.front(); queue.pending.pop_front();
    if(clearReturnedSize) buffer->mAudioDataByteSize=0;
    queue.callback(queue.context,reinterpret_cast<AudioQueueRef>(&queue),buffer);
  }
  AudioQueueFunctions functions() {
    AudioQueueFunctions api;
    api.create=+[](const AudioStreamBasicDescription*,AudioQueueOutputCallback callback,void* context,CFRunLoopRef,CFStringRef,UInt32,AudioQueueRef* output)->OSStatus {
      auto queue=std::make_unique<Queue>(); queue->callback=callback; queue->context=context;
      *output=reinterpret_cast<AudioQueueRef>(queue.get()); active->queues.push_back(std::move(queue)); return noErr;
    };
    api.allocate=+[](AudioQueueRef q,UInt32 bytes,AudioQueueBufferRef* output)->OSStatus {
      auto& queue=cast(q); queue.storage.push_back(std::make_unique<std::uint8_t[]>(bytes));
      queue.buffers.push_back(std::make_unique<AudioQueueBuffer>(queue.storage.back().get(),bytes,nullptr,0));
      *output=queue.buffers.back().get(); return noErr;
    };
    api.enqueue=+[](AudioQueueRef q,AudioQueueBufferRef buffer,UInt32,const AudioStreamPacketDescription*)->OSStatus {
      if(active->enqueueError!=noErr) { const auto error=active->enqueueError; active->enqueueError=noErr; return error; }
      cast(q).pending.push_back(buffer);
      if(active->immediate) active->complete();
      return noErr;
    };
    api.start=+[](AudioQueueRef q,const AudioTimeStamp*)->OSStatus {
      active->emptyStart=active->emptyStart || cast(q).pending.empty();
      ++active->starts;
      if(active->startError!=noErr) { const auto error=active->startError; active->startError=noErr; return error; }
      cast(q).running=true; return noErr;
    };
    api.stop=+[](AudioQueueRef q,Boolean)->OSStatus { cast(q).running=false; return noErr; };
    api.dispose=+[](AudioQueueRef q,Boolean)->OSStatus {
      auto& queue=cast(q); queue.disposed=true;
      // Disposing a stopped queue may return buffers synchronously. The old
      // queue's callback must neither deadlock nor replenish the new queue.
      while(!queue.pending.empty()) {
        const auto buffer=queue.pending.front(); queue.pending.pop_front(); queue.callback(queue.context,q,buffer);
      }
      return noErr;
    };
    api.getProperty=+[](AudioQueueRef q,AudioQueuePropertyID property,void* data,UInt32* bytes)->OSStatus {
      if(property!=kAudioQueueProperty_IsRunning || *bytes!=sizeof(UInt32)) return paramErr;
      *static_cast<UInt32*>(data)=cast(q).running; return noErr;
    };
    api.setProperty=+[](AudioQueueRef,AudioQueuePropertyID,const void*,UInt32)->OSStatus { return noErr; };
    api.setParameter=+[](AudioQueueRef,AudioQueueParameterID,AudioQueueParameterValue)->OSStatus { return noErr; };
    api.now=+[] { return active->time; };
    return api;
  }
};

void testAudioOutputRecovery() {
  FakeAudioOutput device;
  AudioQueuePcmPlayer player({},device.functions());
  std::array<float,480> pcm{}; pcm.fill(0.25F);
  const auto submit=[&] { return player.submit(48000,2,reinterpret_cast<const std::uint8_t*>(pcm.data()),sizeof(pcm),240); };
  for(unsigned i=0;i<600;++i) {
    requireLatency(submit(),"Synthetic playback failed before the three-second stop");
    requireLatency(device.current().running,"Audio output was not started after enqueue");
    device.complete(); device.time+=std::chrono::milliseconds(5);
  }
  requireLatency(!device.emptyStart,"Audio output started before any PCM buffer was queued");
  requireLatency(player.stats().signalFrames==600*240,"Numeric output signal counters must distinguish synthetic PCM from silence");
  device.current().running=false;
  requireLatency(submit() && device.current().running && device.starts==2,"Audio output must resume when the device stops after three seconds");
  device.complete();
  // A stopped/invalid device can also retain buffers without callbacks.
  for(unsigned i=0;i<4;++i) requireLatency(submit(),"Fill the bounded hardware budget");
  requireLatency(player.stats().queuedFrames==960,"Hardware queue must remain within twenty milliseconds");
  device.time+=std::chrono::milliseconds(300);
  requireLatency(submit() && device.queues.size()==2 && device.queues.front()->disposed,"A stalled output must be recreated without replaying its stale buffers");
  requireLatency(player.stats().recoveries==1 && player.stats().queuedFrames==240,"Recovery must queue only fresh PCM");
  device.complete();
  device.immediate=true;
  device.clearReturnedSize=true;
  requireLatency(submit() && player.stats().queuedFrames==0,"A synchronous completion must not deadlock or leak the output budget");
  device.immediate=false;
  device.enqueueError=paramErr;
  requireLatency(!submit() && player.stats().errors==1 && player.stats().lastError==paramErr,"Output enqueue errors must be reported");
  requireLatency(!submit(),"Output failures must not spin recreating devices on every packet");
  device.time+=std::chrono::milliseconds(300);
  requireLatency(submit() && player.stats().recoveries==2,"An invalid output must recover after the bounded retry interval");
  device.complete();
  device.current().running=false; device.startError=paramErr;
  requireLatency(submit() && player.stats().errors==2,"Start failures must reach numeric diagnostics");
  device.time+=std::chrono::milliseconds(300);
  requireLatency(submit() && player.stats().recoveries==3 && device.current().running,"Start failure must not permanently silence output");
  device.complete();
}
