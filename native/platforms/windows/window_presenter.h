#pragma once
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <cstdint>

namespace sanser::desktop {
// One reusable back buffer; only a completed image is copied to the window.
class WindowPresenter {
public:
  ~WindowPresenter();
  WindowPresenter() = default;
  WindowPresenter(const WindowPresenter&) = delete;
  WindowPresenter& operator=(const WindowPresenter&) = delete;
  bool paint(HDC target, int width, int height, const std::uint8_t* pixels,
             std::uint32_t sourceWidth, std::uint32_t sourceHeight, RECT& video);
private:
  bool resize(HDC target, int width, int height);
  HDC buffer_=nullptr;
  HBITMAP bitmap_=nullptr;
  HGDIOBJ original_=nullptr;
  int width_=0, height_=0;
};
}
