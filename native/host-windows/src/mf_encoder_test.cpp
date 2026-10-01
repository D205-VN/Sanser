#include "mf_video_packet_encoder.h"
#include "mf_encoder_events.h"
#include "gpu_video_processor.h"
#include <d3d11.h>
#include <algorithm>
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
      std::vector<std::uint64_t> captureTimes;
      for (unsigned i = 0; i < 10; ++i) {
        frame.pixels[0] = static_cast<std::uint8_t>(i);
        // An idle desktop and an encoder running slower than requested FPS
        // must retain wall-clock capture spacing, not i * nominal duration.
        const std::uint64_t capturedAt = 1000 + i * 50000 + (i >= 5 ? 1000000 : 0);
        captureTimes.push_back(capturedAt);
        auto encoded = encoder.encodeFrame(frame, capturedAt);
        packets.insert(packets.end(), std::make_move_iterator(encoded.begin()), std::make_move_iterator(encoded.end()));
      }
      require(!packets.empty(), "live H264 output must be available before end-of-stream drain");
      auto tail = encoder.finish();
      packets.insert(packets.end(), std::make_move_iterator(tail.begin()), std::make_move_iterator(tail.end()));
      require(!packets.empty(), "software H264 must produce packets without a GPU");
      require(packets.front().keyframe, "fallback starts with a keyframe");
      // Some MFTs emit configuration samples without a timestamp. The host
      // normalizes the wire timeline separately; verify encoded payload here.
      for (const auto& packet : packets) {
        require(!packet.payload.empty(), "software encoder payload must not be empty");
      }
      require(std::any_of(packets.begin(), packets.end(), [](const auto& packet) {
        return packet.timestampMicros >= 1251000;
      }), "capture idle gap must survive hardware API output timestamps");
      for (const auto& packet : packets) {
        if (packet.timestampMicros == 0) continue; // codec configuration sample
        require(std::find(captureTimes.begin(), captureTimes.end(), packet.timestampMicros) != captureTimes.end(),
          "encoded frame must keep its submitted capture timestamp");
      }
    }
    {
      // WARP exercises GPU ownership/readback on CI without requiring a GPU.
      ComPtr<ID3D11Device> device;
      require(SUCCEEDED(D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_WARP, nullptr, 0,
        nullptr, 0, D3D11_SDK_VERSION, &device, nullptr, nullptr)), "create test D3D11 device");
      std::vector<std::uint8_t> pixels(32 * 32 * 4, 128);
      D3D11_TEXTURE2D_DESC desc{};
      desc.Width = 32; desc.Height = 32; desc.MipLevels = 1; desc.ArraySize = 1;
      desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM; desc.SampleDesc.Count = 1; desc.Usage = D3D11_USAGE_DEFAULT;
      D3D11_SUBRESOURCE_DATA initial{pixels.data(), 32 * 4, 0};
      ComPtr<ID3D11Texture2D> texture;
      require(SUCCEEDED(device->CreateTexture2D(&desc, &initial, &texture)), "create owned capture texture");
      FrameBgra gpu;
      gpu.width = 32; gpu.height = 32; gpu.stride = 128;
      gpu.texture = std::shared_ptr<ID3D11Texture2D>(texture.Detach(), [](auto* value) { value->Release(); });
      auto restored = readbackGpuFrame(gpu);
      require(restored.pixels == pixels && !restored.texture, "GPU fallback must preserve capture pixels");
      gpu.cursorVisible = true; gpu.cursorX = 4; gpu.cursorY = 4;
      auto cursor = readbackGpuFrame(gpu);
      require(cursor.pixels[(4 * 32 + 4) * 4] == 255, "GPU fallback must preserve visible cursor");
      require(readbackGpuFrame(FrameBgra{32, 32, 128, pixels}).pixels == pixels, "CPU fallback remains compatible");
    }
    MFShutdown();
    CoUninitialize();
    std::cout << "Async MF event gating, error propagation and software H264 encoding passed\n";
    return 0;
  } catch (const std::exception& error) { std::cerr << error.what() << '\n'; return 1; }
}
