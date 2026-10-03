#define NOMINMAX
#define WIN32_LEAN_AND_MEAN
#include <algorithm>
#include "windows_precise_wait.h"
#include <iostream>
#include <stdexcept>

int main() {
  using namespace std::chrono;
  sanser::WindowsPreciseWait timer;
  double overshoot = 0;
  for (int i = 0; i < 16; ++i) {
    const auto deadline = steady_clock::now() + microseconds(500);
    timer.until(deadline);
    const auto ended = steady_clock::now();
    if (ended < deadline) throw std::runtime_error("pacer woke before deadline");
    overshoot += duration<double, std::milli>(ended - deadline).count();
    // Re-arming after an expired deadline must not reuse a signaled timer.
    timer.until(deadline - milliseconds(1));
  }
  std::cout << "highResolution=" << timer.highResolution()
            << " meanOvershootMs=" << overshoot / 16 << '\n';
}
