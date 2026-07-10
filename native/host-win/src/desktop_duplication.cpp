#include "desktop_duplication.h"

#include <d3d11.h>
#include <dxgi1_2.h>
#include <wrl/client.h>

#include <cstdio>
#include <chrono>
#include <cstring>
#include <iterator>
#include <stdexcept>
#include <utility>

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
  bool recoveryPending = false;
  std::chrono::steady_clock::time_point nextRecoveryAttempt{};
  std::uint32_t recoveryFailures = 0;
};

DesktopDuplicator::DesktopDuplicator() : impl_(std::make_unique<Impl>()) {}

DesktopDuplicator::~DesktopDuplicator() = default;

void DesktopDuplicator::initialize(std::uint32_t adapterIndex, std::uint32_t outputIndex) {
  auto next = std::make_unique<Impl>();
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
            next->device.GetAddressOf(),
            &chosenLevel,
            next->context.GetAddressOf()),
          "D3D11CreateDevice");

  ComPtr<IDXGIOutput> output;
  checkHr(adapter->EnumOutputs(outputIndex, output.GetAddressOf()), "EnumOutputs");

  DXGI_OUTPUT_DESC outputDesc{};
  checkHr(output->GetDesc(&outputDesc), "IDXGIOutput::GetDesc");
  const auto nextLeft = outputDesc.DesktopCoordinates.left;
  const auto nextTop = outputDesc.DesktopCoordinates.top;
  const auto nextWidth = static_cast<std::uint32_t>(
    outputDesc.DesktopCoordinates.right - outputDesc.DesktopCoordinates.left);
  const auto nextHeight = static_cast<std::uint32_t>(
    outputDesc.DesktopCoordinates.bottom - outputDesc.DesktopCoordinates.top);

  auto output1 = queryInterface<IDXGIOutput1>(output.Get(), "IDXGIOutput1");
  checkHr(output1->DuplicateOutput(next->device.Get(), next->duplication.GetAddressOf()), "DuplicateOutput");

  DXGI_OUTDUPL_DESC duplicationDesc{};
  next->duplication->GetDesc(&duplicationDesc);
  const auto nextSurfaceWidth = duplicationDesc.ModeDesc.Width;
  const auto nextSurfaceHeight = duplicationDesc.ModeDesc.Height;
  if (nextWidth == 0 || nextHeight == 0 || nextSurfaceWidth == 0 || nextSurfaceHeight == 0) {
    throw std::runtime_error("Desktop output has invalid dimensions.");
  }

  D3D11_TEXTURE2D_DESC stagingDesc{};
  stagingDesc.Width = nextSurfaceWidth;
  stagingDesc.Height = nextSurfaceHeight;
  stagingDesc.MipLevels = 1;
  stagingDesc.ArraySize = 1;
  stagingDesc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
  stagingDesc.SampleDesc.Count = 1;
  stagingDesc.Usage = D3D11_USAGE_STAGING;
  stagingDesc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;

  checkHr(next->device->CreateTexture2D(&stagingDesc, nullptr, next->staging.GetAddressOf()),
          "CreateTexture2D staging");

  impl_ = std::move(next);
  adapterIndex_ = adapterIndex;
  outputIndex_ = outputIndex;
  left_ = nextLeft;
  top_ = nextTop;
  width_ = nextWidth;
  height_ = nextHeight;
  surfaceWidth_ = nextSurfaceWidth;
  surfaceHeight_ = nextSurfaceHeight;
  rotation_ = static_cast<unsigned int>(outputDesc.Rotation);
}

bool DesktopDuplicator::captureFrame(FrameBgra& frame, std::uint32_t timeoutMs) {
  if (impl_->recoveryPending) {
    const auto now = std::chrono::steady_clock::now();
    if (now < impl_->nextRecoveryAttempt) return false;
    try {
      initialize(adapterIndex_, outputIndex_);
    } catch (const std::exception& error) {
      impl_->nextRecoveryAttempt = now + std::chrono::milliseconds(250);
      impl_->recoveryFailures += 1;
      if (impl_->recoveryFailures == 1 || impl_->recoveryFailures % 20 == 0) {
        std::fprintf(stderr,
                     "Desktop duplication recovery pending (attempt %u): %s\n",
                     impl_->recoveryFailures,
                     error.what());
      }
      return false;
    }
  }
  if (!impl_->duplication) {
    throw std::runtime_error("DesktopDuplicator is not initialized.");
  }

  DXGI_OUTDUPL_FRAME_INFO frameInfo{};
  ComPtr<IDXGIResource> desktopResource;
  const HRESULT acquireHr = impl_->duplication->AcquireNextFrame(timeoutMs, &frameInfo, desktopResource.GetAddressOf());
  if (acquireHr == DXGI_ERROR_WAIT_TIMEOUT) return false;
  if (acquireHr == DXGI_ERROR_ACCESS_LOST) {
    impl_->recoveryPending = true;
    impl_->nextRecoveryAttempt = std::chrono::steady_clock::now();
    return false;
  }
  checkHr(acquireHr, "AcquireNextFrame");

  bool released = false;
  auto releaseFrame = [&]() {
    if (!released) {
      impl_->duplication->ReleaseFrame();
      released = true;
    }
  };

  bool mappedStaging = false;
  try {
    auto desktopTexture = queryInterface<ID3D11Texture2D>(desktopResource.Get(), "desktop texture");
    D3D11_TEXTURE2D_DESC textureDesc{};
    desktopTexture->GetDesc(&textureDesc);
    if (textureDesc.Width != surfaceWidth_ || textureDesc.Height != surfaceHeight_) {
      releaseFrame();
      impl_->recoveryPending = true;
      impl_->nextRecoveryAttempt = std::chrono::steady_clock::now();
      return false;
    }
    const auto rotation = static_cast<DXGI_MODE_ROTATION>(rotation_);
    const bool swapsAxes = rotation == DXGI_MODE_ROTATION_ROTATE90
      || rotation == DXGI_MODE_ROTATION_ROTATE270;
    const auto expectedWidth = swapsAxes ? surfaceHeight_ : surfaceWidth_;
    const auto expectedHeight = swapsAxes ? surfaceWidth_ : surfaceHeight_;
    if (width_ != expectedWidth || height_ != expectedHeight) {
      releaseFrame();
      impl_->recoveryPending = true;
      impl_->nextRecoveryAttempt = std::chrono::steady_clock::now();
      return false;
    }
    impl_->context->CopyResource(impl_->staging.Get(), desktopTexture.Get());

    D3D11_MAPPED_SUBRESOURCE mapped{};
    checkHr(impl_->context->Map(impl_->staging.Get(), 0, D3D11_MAP_READ, 0, &mapped), "Map staging");

    mappedStaging = true;
    auto unmapStaging = [&]() {
      if (mappedStaging) {
        impl_->context->Unmap(impl_->staging.Get(), 0);
        mappedStaging = false;
      }
    };

    frame.width = width_;
    frame.height = height_;
    frame.stride = width_ * 4;
    frame.pixels.resize(static_cast<std::size_t>(frame.stride) * frame.height);

    const auto* source = static_cast<const std::uint8_t*>(mapped.pData);
    const auto copyPixel = [&](std::uint32_t dstX,
                               std::uint32_t dstY,
                               std::uint32_t srcX,
                               std::uint32_t srcY) {
      if (srcX >= surfaceWidth_ || srcY >= surfaceHeight_) {
        throw std::runtime_error("Desktop rotation produced an out-of-range pixel coordinate.");
      }
      const auto* srcPixel = source
        + static_cast<std::size_t>(mapped.RowPitch) * srcY
        + static_cast<std::size_t>(srcX) * 4;
      auto* dstPixel = frame.pixels.data()
        + static_cast<std::size_t>(frame.stride) * dstY
        + static_cast<std::size_t>(dstX) * 4;
      std::memcpy(dstPixel, srcPixel, 4);
    };

    switch (rotation) {
      case DXGI_MODE_ROTATION_ROTATE90:
        for (std::uint32_t y = 0; y < frame.height; ++y) {
          for (std::uint32_t x = 0; x < frame.width; ++x) {
            copyPixel(x, y, y, surfaceHeight_ - 1 - x);
          }
        }
        break;
      case DXGI_MODE_ROTATION_ROTATE180:
        for (std::uint32_t y = 0; y < frame.height; ++y) {
          for (std::uint32_t x = 0; x < frame.width; ++x) {
            copyPixel(x, y, surfaceWidth_ - 1 - x, surfaceHeight_ - 1 - y);
          }
        }
        break;
      case DXGI_MODE_ROTATION_ROTATE270:
        for (std::uint32_t y = 0; y < frame.height; ++y) {
          for (std::uint32_t x = 0; x < frame.width; ++x) {
            copyPixel(x, y, surfaceWidth_ - 1 - y, x);
          }
        }
        break;
      case DXGI_MODE_ROTATION_UNSPECIFIED:
      case DXGI_MODE_ROTATION_IDENTITY:
      default:
        for (std::uint32_t y = 0; y < frame.height; ++y) {
          const auto* srcRow = source + static_cast<std::size_t>(mapped.RowPitch) * y;
          auto* dstRow = frame.pixels.data() + static_cast<std::size_t>(frame.stride) * y;
          std::memcpy(dstRow, srcRow, frame.stride);
        }
        break;
    }

    unmapStaging();
    releaseFrame();
    return true;
  } catch (...) {
    if (mappedStaging && impl_->context && impl_->staging) {
      impl_->context->Unmap(impl_->staging.Get(), 0);
    }
    releaseFrame();
    throw;
  }
}

std::string hresultMessage(long hr) {
  char buffer[64]{};
  std::snprintf(buffer, sizeof(buffer), "HRESULT 0x%08lX", static_cast<unsigned long>(hr));
  return buffer;
}
