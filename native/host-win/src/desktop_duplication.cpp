#include "desktop_duplication.h"

#include <d3d11.h>
#include <dxgi1_2.h>
#include <wrl/client.h>

#include <cstdio>
#include <cstring>
#include <iterator>
#include <stdexcept>

using Microsoft::WRL::ComPtr;

namespace {

void checkHr(HRESULT hr, const char* label) {
  if (FAILED(hr)) {
    throw std::runtime_error(std::string(label) + ": " + hresultMessage(hr));
  }
}

template <typename T>
ComPtr<T> queryInterface(IUnknown* source, const char* label) {
  ComPtr<T> result;
  checkHr(source->QueryInterface(__uuidof(T), reinterpret_cast<void**>(result.GetAddressOf())), label);
  return result;
}

} // namespace

struct DesktopDuplicator::Impl {
  ComPtr<ID3D11Device> device;
  ComPtr<ID3D11DeviceContext> context;
  ComPtr<IDXGIOutputDuplication> duplication;
  ComPtr<ID3D11Texture2D> staging;
};

DesktopDuplicator::DesktopDuplicator() : impl_(std::make_unique<Impl>()) {}

DesktopDuplicator::~DesktopDuplicator() = default;

void DesktopDuplicator::initialize(std::uint32_t adapterIndex, std::uint32_t outputIndex) {
  ComPtr<IDXGIFactory1> factory;
  checkHr(CreateDXGIFactory1(__uuidof(IDXGIFactory1), reinterpret_cast<void**>(factory.GetAddressOf())),
          "CreateDXGIFactory1");

  ComPtr<IDXGIAdapter1> adapter;
  checkHr(factory->EnumAdapters1(adapterIndex, adapter.GetAddressOf()), "EnumAdapters1");

  D3D_FEATURE_LEVEL featureLevels[] = {
    D3D_FEATURE_LEVEL_11_1,
    D3D_FEATURE_LEVEL_11_0,
    D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_10_0,
  };
  D3D_FEATURE_LEVEL chosenLevel{};

  checkHr(D3D11CreateDevice(
            adapter.Get(),
            D3D_DRIVER_TYPE_UNKNOWN,
            nullptr,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            featureLevels,
            static_cast<UINT>(std::size(featureLevels)),
            D3D11_SDK_VERSION,
            impl_->device.GetAddressOf(),
            &chosenLevel,
            impl_->context.GetAddressOf()),
          "D3D11CreateDevice");

  ComPtr<IDXGIOutput> output;
  checkHr(adapter->EnumOutputs(outputIndex, output.GetAddressOf()), "EnumOutputs");

  DXGI_OUTPUT_DESC outputDesc{};
  checkHr(output->GetDesc(&outputDesc), "IDXGIOutput::GetDesc");
  width_ = static_cast<std::uint32_t>(outputDesc.DesktopCoordinates.right - outputDesc.DesktopCoordinates.left);
  height_ = static_cast<std::uint32_t>(outputDesc.DesktopCoordinates.bottom - outputDesc.DesktopCoordinates.top);

  auto output1 = queryInterface<IDXGIOutput1>(output.Get(), "IDXGIOutput1");
  checkHr(output1->DuplicateOutput(impl_->device.Get(), impl_->duplication.GetAddressOf()), "DuplicateOutput");

  D3D11_TEXTURE2D_DESC stagingDesc{};
  stagingDesc.Width = width_;
  stagingDesc.Height = height_;
  stagingDesc.MipLevels = 1;
  stagingDesc.ArraySize = 1;
  stagingDesc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
  stagingDesc.SampleDesc.Count = 1;
  stagingDesc.Usage = D3D11_USAGE_STAGING;
  stagingDesc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;

  checkHr(impl_->device->CreateTexture2D(&stagingDesc, nullptr, impl_->staging.GetAddressOf()),
          "CreateTexture2D staging");
}

bool DesktopDuplicator::captureFrame(FrameBgra& frame, std::uint32_t timeoutMs) {
  if (!impl_->duplication) {
    throw std::runtime_error("DesktopDuplicator is not initialized.");
  }

  DXGI_OUTDUPL_FRAME_INFO frameInfo{};
  ComPtr<IDXGIResource> desktopResource;
  const HRESULT acquireHr = impl_->duplication->AcquireNextFrame(timeoutMs, &frameInfo, desktopResource.GetAddressOf());
  if (acquireHr == DXGI_ERROR_WAIT_TIMEOUT) return false;
  checkHr(acquireHr, "AcquireNextFrame");

  bool released = false;
  auto releaseFrame = [&]() {
    if (!released) {
      impl_->duplication->ReleaseFrame();
      released = true;
    }
  };

  try {
    auto desktopTexture = queryInterface<ID3D11Texture2D>(desktopResource.Get(), "desktop texture");
    impl_->context->CopyResource(impl_->staging.Get(), desktopTexture.Get());

    D3D11_MAPPED_SUBRESOURCE mapped{};
    checkHr(impl_->context->Map(impl_->staging.Get(), 0, D3D11_MAP_READ, 0, &mapped), "Map staging");

    frame.width = width_;
    frame.height = height_;
    frame.stride = width_ * 4;
    frame.pixels.resize(static_cast<std::size_t>(frame.stride) * frame.height);

    const auto* source = static_cast<const std::uint8_t*>(mapped.pData);
    for (std::uint32_t y = 0; y < frame.height; ++y) {
      const auto* srcRow = source + static_cast<std::size_t>(mapped.RowPitch) * y;
      auto* dstRow = frame.pixels.data() + static_cast<std::size_t>(frame.stride) * y;
      std::memcpy(dstRow, srcRow, frame.stride);
    }

    impl_->context->Unmap(impl_->staging.Get(), 0);
    releaseFrame();
    return true;
  } catch (...) {
    releaseFrame();
    throw;
  }
}

std::string hresultMessage(long hr) {
  char buffer[64]{};
  std::snprintf(buffer, sizeof(buffer), "HRESULT 0x%08lX", static_cast<unsigned long>(hr));
  return buffer;
}
