#pragma once
#include "desktop_duplication.h"
#include <mfidl.h>
#include <wrl/client.h>
#include <memory>

// Capture BGRA -> cursor composition -> scaled NV12 stays on the capture device.
// Each MF sample owns its NV12 surface until the encoder releases it.
class GpuVideoProcessor {
public:
  GpuVideoProcessor(ID3D11Device* device, std::uint32_t width, std::uint32_t height);
  ~GpuVideoProcessor();
  IMFDXGIDeviceManager* manager() const;
  // Null means the bounded surface pool is busy: skip this unencoded frame.
  Microsoft::WRL::ComPtr<IMFSample> convert(const FrameBgra& frame);
private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

FrameBgra readbackGpuFrame(const FrameBgra& frame);
