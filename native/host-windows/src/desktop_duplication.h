#pragma once

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

struct ID3D11Device;
struct ID3D11Texture2D;

struct FrameBgra {
  std::uint32_t width = 0;
  std::uint32_t height = 0;
  std::uint32_t stride = 0;
  std::vector<std::uint8_t> pixels;
  // Owned GPU texture; never retain the DXGI acquired resource after ReleaseFrame.
  std::shared_ptr<ID3D11Texture2D> texture;
  bool cursorVisible = false;
  int cursorX = 0;
  int cursorY = 0;
};

class DesktopDuplicator {
public:
  DesktopDuplicator();
  ~DesktopDuplicator();

  DesktopDuplicator(const DesktopDuplicator&) = delete;
  DesktopDuplicator& operator=(const DesktopDuplicator&) = delete;

  void initialize(std::uint32_t adapterIndex = 0, std::uint32_t outputIndex = 0);
  bool captureFrame(FrameBgra& frame, std::uint32_t timeoutMs = 1000);
  void setGpuCapture(bool enabled) { gpuCapture_ = enabled; }
  ID3D11Device* gpuDevice() const;
  // AcquireNextFrame may wait for a desktop change; this is not capture work.
  std::uint64_t lastAcquireWaitMicros() const { return lastAcquireWaitMicros_; }
  std::uint64_t gpuPoolBusyDrops() const { return gpuPoolBusyDrops_; }
  bool recovering() const;
  std::uint64_t generation() const { return generation_; }

  std::uint32_t width() const { return width_; }
  std::uint32_t height() const { return height_; }
  long left() const { return left_; }
  long top() const { return top_; }

private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
  std::uint32_t width_ = 0;
  std::uint32_t height_ = 0;
  std::uint32_t surfaceWidth_ = 0;
  std::uint32_t surfaceHeight_ = 0;
  std::uint32_t adapterIndex_ = 0;
  std::uint32_t outputIndex_ = 0;
  unsigned int rotation_ = 0;
  long left_ = 0;
  long top_ = 0;
  bool gpuCapture_ = false;
  std::uint64_t lastAcquireWaitMicros_ = 0;
  std::uint64_t gpuPoolBusyDrops_ = 0;
  std::uint64_t generation_ = 0;
};

std::string hresultMessage(long hr);
