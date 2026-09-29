#include "mf_video_packet_encoder.h"
#include "mf_encoder_events.h"
#include <wrl.h>
#include <iostream>
#include <stdexcept>
#include <iterator>

using namespace Microsoft::WRL;
void require(bool ok, const char* message) { if (!ok) throw std::runtime_error(message); }

// A real MF event queue allows asynchronous scheduling tests on GPU-less CI.
class TestEvents : public RuntimeClass<RuntimeClassFlags<ClassicCom>, IMFMediaEventGenerator> {
public:
  TestEvents() { require(SUCCEEDED(MFCreateEventQueue(&queue_)), "create MF event queue"); }
  ~TestEvents() { queue_->Shutdown(); }
  STDMETHODIMP GetEvent(DWORD flags, IMFMediaEvent** event) override { return queue_->GetEvent(flags, event); }
  STDMETHODIMP BeginGetEvent(IMFAsyncCallback* callback, IUnknown* state) override { return queue_->BeginGetEvent(callback, state); }
  STDMETHODIMP EndGetEvent(IMFAsyncResult* result, IMFMediaEvent** event) override { return queue_->EndGetEvent(result, event); }
  STDMETHODIMP QueueEvent(MediaEventType type, REFGUID extended, HRESULT status, const PROPVARIANT* value) override {
    return queue_->QueueEventParamVar(type, extended, status, value);
  }
  void emit(MediaEventType type, HRESULT status = S_OK) {
    require(SUCCEEDED(QueueEvent(type, GUID_NULL, status, nullptr)), "queue MF event");
  }
private:
  ComPtr<IMFMediaEventQueue> queue_;
};

int main() {
  try {
    require(SUCCEEDED(CoInitializeEx(nullptr, COINIT_MULTITHREADED)), "initialize COM");
    require(SUCCEEDED(MFStartup(MF_VERSION)), "initialize MF");
    {
      auto queue = Make<TestEvents>();
      MfEncoderEvents events;
      events.initialize(queue.Get());
      events.collect();
      require(!events.inputReady() && !events.takeOutput(), "no processing before events");
      queue->emit(METransformNeedInput);
      queue->emit(METransformNeedInput);
      events.collect();
      require(events.inputReady(), "first input credit"); events.consumeInput();
      require(events.inputReady(), "second input credit"); events.consumeInput();
      require(!events.inputReady(), "input credits consumed exactly once");
      require(!events.takeOutput(), "ProcessInput does not imply ProcessOutput readiness");
      queue->emit(METransformHaveOutput);
      events.collect();
      require(events.takeOutput() && !events.takeOutput(), "one ProcessOutput call per event");
      events.beginDrain();
      queue->emit(METransformHaveOutput);
      queue->emit(METransformDrainComplete);
      events.collect();
      require(!events.drained(), "drain must retain queued output");
      require(events.takeOutput() && events.drained(), "drain output before completion");
      queue->emit(MEError, E_UNEXPECTED);
      bool failed = false;
      try { events.collect(); } catch (const std::exception&) { failed = true; }
      require(failed, "hardware event failures must propagate for fallback");
    }
    {
      VideoPacketEncodeOptions options;
      options.hardware = false;
      options.encoderPreference = "software";
      options.fps = 30;
      options.bitrate = 2000000;
      MfVideoPacketEncoder encoder(320, 240, options);
      require(!encoder.usingHardware(), "software fallback must select software");
      FrameBgra frame{320, 240, 320 * 4, std::vector<std::uint8_t>(320 * 240 * 4, 128)};
      std::vector<EncodedVideoPacket> packets;
      for (unsigned i = 0; i < 10; ++i) {
        frame.pixels[0] = static_cast<std::uint8_t>(i);
        auto encoded = encoder.encodeFrame(frame);
        packets.insert(packets.end(), std::make_move_iterator(encoded.begin()), std::make_move_iterator(encoded.end()));
      }
      auto tail = encoder.finish();
      packets.insert(packets.end(), std::make_move_iterator(tail.begin()), std::make_move_iterator(tail.end()));
      require(!packets.empty(), "software H264 must produce packets without a GPU");
      require(packets.front().keyframe, "fallback starts with a keyframe");
      for (std::size_t i = 1; i < packets.size(); ++i)
        require(packets[i].timestampMicros > packets[i-1].timestampMicros, "monotonic timestamps");
    }
    MFShutdown();
    CoUninitialize();
    std::cout << "Async MF event gating, error propagation and software H264 encoding passed\n";
    return 0;
  } catch (const std::exception& error) { std::cerr << error.what() << '\n'; return 1; }
}
