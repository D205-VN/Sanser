#pragma once
#ifdef _WIN32
#include <windows.h>
#include <algorithm>
#include <chrono>
#include <thread>

namespace sanser {
// One timer per sending thread/object; no system-wide timer-resolution change.
// Never share a timer between waiters: arming it would replace the other's wait.
class WindowsPreciseWait {
public:
  WindowsPreciseWait()
    : timer_(CreateWaitableTimerExW(nullptr, nullptr, 0x00000002,
                                   TIMER_MODIFY_STATE | SYNCHRONIZE)) {}
  ~WindowsPreciseWait() { if (timer_) CloseHandle(timer_); }
  WindowsPreciseWait(const WindowsPreciseWait&) = delete;
  WindowsPreciseWait& operator=(const WindowsPreciseWait&) = delete;
  bool highResolution() const { return timer_ != nullptr; }

  void until(std::chrono::steady_clock::time_point deadline) {
    using namespace std::chrono;
    constexpr auto tail = microseconds(75);
    for (;;) {
      const auto now = steady_clock::now();
      if (now >= deadline) return;
      const auto remaining = deadline - now;
      if (timer_ && remaining > microseconds(300)) {
        LARGE_INTEGER due{};
        due.QuadPart = -std::max<LONGLONG>(1,
          duration_cast<nanoseconds>(remaining - tail).count() / 100);
        if (SetWaitableTimerEx(timer_, &due, 0, nullptr, nullptr, nullptr, 0)) {
          // No unbounded wait if the timer fails. The outer monotonic deadline
          // remains authoritative after a timeout or scheduler overshoot.
          const auto timeout = duration_cast<milliseconds>(remaining).count() + 1;
          WaitForSingleObject(timer_, static_cast<DWORD>(timeout));
          continue;
        }
      }
      // Short tail, or older Windows without high-resolution timers. Avoid a
      // coarse Sleep rounding a sub-millisecond packet gap to a scheduler tick.
      if (remaining > microseconds(20)) std::this_thread::yield();
      else YieldProcessor();
    }
  }
private:
  HANDLE timer_ = nullptr;
};
}
#endif
