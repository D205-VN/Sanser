#pragma once
// Synthetic Core Audio adapter: exercise the production player without opening
// an output endpoint. Time, device stops and callback completion are controlled.
struct FakeAudioOutput {
  struct Queue {
    AudioQueueOutputCallback callback; void* context;
    bool running=false,disposed=false;
    UInt32 converterError=0;
    std::vector<std::unique_ptr<std::uint8_t[]>> storage;
    std::vector<std::unique_ptr<AudioQueueBuffer>> buffers;
    std::deque<AudioQueueBufferRef> pending;
  };
  inline static FakeAudioOutput* active=nullptr;
  std::vector<std::unique_ptr<Queue>> queues;
  std::chrono::steady_clock::time_point time=std::chrono::steady_clock::time_point{}+std::chrono::seconds(1);
  bool immediate=false,emptyStart=false,clearReturnedSize=false;
  bool converterSupported=true;
  bool clockSupported=true,frozenClock=false,clockHasSampleTime=true;
  OSStatus enqueueError=noErr,startError=noErr;
  unsigned starts=0;
  AudioOutputRoute route{1,1,48000};
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
      if(*bytes!=sizeof(UInt32)) return paramErr;
      if(property==kAudioQueueProperty_IsRunning) *static_cast<UInt32*>(data)=cast(q).running;
      else if(property==kAudioQueueProperty_ConverterError && active->converterSupported) *static_cast<UInt32*>(data)=cast(q).converterError;
      else return paramErr;
      return noErr;
    };
    api.setProperty=+[](AudioQueueRef,AudioQueuePropertyID,const void*,UInt32)->OSStatus { return noErr; };
    api.setParameter=+[](AudioQueueRef,AudioQueueParameterID,AudioQueueParameterValue)->OSStatus { return noErr; };
    api.now=+[] { return active->time; };
    api.deviceTime=+[](AudioQueueRef,AudioTimeStamp* stamp)->OSStatus {
      if(!active->clockSupported) return paramErr;
      stamp->mFlags=active->clockHasSampleTime ? kAudioTimeStampSampleTimeValid : kAudioTimeStampHostTimeValid;
      stamp->mSampleTime=active->frozenClock ? 100.0 :
        std::chrono::duration<double>(active->time.time_since_epoch()).count()*48000;
      return noErr;
    };
    api.route=+[](const std::string&) { return active->route; };
    return api;
  }
};

void testAudioOutputRecovery() {
  FakeAudioOutput device;
  AudioQueuePcmPlayer player({},device.functions());
  std::array<float,480> pcm{}; pcm.fill(0.25F);
  requireLatency(!player.submit(48000,2,reinterpret_cast<const std::uint8_t*>(pcm.data()),sizeof(pcm),536870912U)
    && !player.submit(48000,2,nullptr,sizeof(pcm),240) && device.queues.empty(),
    "Malformed PCM size must be rejected before device allocation or signal scanning");
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
  // Hot-unplug can leave a queue reporting running=true on the old route.
  // A source change on the same device (built-in headphones -> speakers)
  // must replace it without waiting for a callback failure or a session reset.
  for(unsigned i=0;i<8000;++i) {
    if(i==4000) ++device.route.source;
    if(i==6000) { device.route.device=2; device.route.rate=44100; }
    const auto previousChanges=player.stats().routeChanges;
    device.time+=std::chrono::milliseconds(5);
    requireLatency(submit(),"Audio stopped during a forty-second route-switch run");
    if(player.stats().routeChanges>previousChanges) {
      const auto* pcm=static_cast<const float*>(device.current().pending.front()->mAudioData);
      requireLatency(std::abs(pcm[0])<0.01F && std::abs(pcm[478]-0.25F)<0.0001F,
        "A new output route must ramp in without clipping continuous speech");
    }
    device.complete();
  }
  requireLatency(player.stats().recoveries==5 && player.stats().routeChanges==2 && player.stats().queuedFrames==0,
    "Both same-device jack changes and default device/rate changes must recover");
  const auto before=player.stats().routeChanges;
  for(unsigned i=0;i<8;++i) {
    device.time+=std::chrono::milliseconds(250);
    device.route.sourceKnown=device.route.rateKnown=i%2==0;
    device.route.rate=device.route.rateKnown ? 44100.0001 : 0;
    device.route.source=device.route.sourceKnown ? 2 : 0;
    requireLatency(submit(),"Temporary USB property lookup loss stopped playback");
    device.complete();
  }
  requireLatency(player.stats().routeChanges==before,
    "Unknown USB source/rate reads and fractional nominal-rate noise must not restart output");

  // A USB converter can report an error while IsRunning and enqueue still
  // succeed. Detect it and retry from the fault time, not the last callback.
  device.current().converterError=static_cast<UInt32>(paramErr);
  device.time+=std::chrono::milliseconds(100);
  requireLatency(submit() && device.current().running && player.stats().converterErrors==1 && player.stats().lastError==paramErr,
    "USB converter failure must be visible even while the queue reports running");
  const auto failedAt=device.time;
  device.time=failedAt+std::chrono::milliseconds(100); device.complete();
  requireLatency(!submit(),"Converter failure must use a bounded restart cooldown");
  device.time=failedAt+std::chrono::milliseconds(250);
  const auto oldQueues=device.queues.size();
  requireLatency(submit() && device.queues.size()==oldQueues+1 && device.current().converterError==0,
    "A late USB callback must not postpone converter recovery indefinitely");
  device.complete();
  for(unsigned i=0;i<1000;++i) {
    device.time+=std::chrono::milliseconds(5);
    requireLatency(submit(),"Recovered USB converter did not keep playing beyond forty seconds");
    device.complete();
  }
  device.converterSupported=false;
  const auto errors=player.stats().errors;
  device.time+=std::chrono::milliseconds(200);
  requireLatency(submit() && player.stats().errors==errors,
    "An unsupported converter property must not be treated as USB output failure");
  device.complete();
  // The physical output can stop advancing while enqueue and completion
  // callbacks still succeed. The audio worker must recover without unplugging.
  const auto oldClockQueues=device.queues.size();
  device.frozenClock=true;
  for(unsigned i=0;i<100;++i) {
    device.time+=std::chrono::milliseconds(5);
    if(submit()) device.complete();
  }
  requireLatency(player.stats().clockStalls==1 && player.stats().errors==errors,
    "Stalled device clock must be detected separately from API/converter errors");
  device.frozenClock=false;
  for(unsigned i=0;i<60;++i) {
    device.time+=std::chrono::milliseconds(5);
    if(submit()) device.complete();
  }
  requireLatency(device.queues.size()==oldClockQueues+1 && player.stats().clockStalls==1,
    "A frozen output clock did not recover on the same headphone route");
  device.clockSupported=false;
  for(unsigned i=0;i<100;++i) {
    device.time+=std::chrono::milliseconds(5);
    requireLatency(submit(),"Unsupported device clock must not stop valid playback");
    device.complete();
  }
  requireLatency(player.stats().clockStalls==1,"An unsupported clock query caused a false recovery");
  device.clockSupported=true;
  device.clockHasSampleTime=false;
  device.frozenClock=true;
  const auto healthyQueues=device.queues.size();
  for(unsigned i=0;i<100;++i) {
    device.time+=std::chrono::milliseconds(5);
    requireLatency(submit(),"A timestamp without valid sample time must not stop playback");
    device.complete();
  }
  requireLatency(device.queues.size()==healthyQueues && player.stats().clockStalls==1,
    "An invalid sample timestamp caused a false output recovery");
}
