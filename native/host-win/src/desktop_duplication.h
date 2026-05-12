#pragma once

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

struct FrameBgra {
  std::uint32_t width = 0;
  std::uint32_t height = 0;
  std::uint32_t stride = 0;
  std::vector<std::uint8_t> pixels;
};

class DesktopDuplicator {
public:
  DesktopDuplicator();
  ~DesktopDuplicator();

  DesktopDuplicator(const DesktopDuplicator&) = delete;
  DesktopDuplicator& operator=(const DesktopDuplicator&) = delete;

  void initialize(std::uint32_t adapterIndex = 0, std::uint32_t outputIndex = 0);
  bool captureFrame(FrameBgra& frame, std::uint32_t timeoutMs = 1000);

  std::uint32_t width() const { return width_; }
  std::uint32_t height() const { return height_; }

private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
  std::uint32_t width_ = 0;
  std::uint32_t height_ = 0;
};

std::string hresultMessage(long hr);
