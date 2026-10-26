#pragma once
#include <chrono>
#include <optional>
#include <string>
#include <utility>

namespace sanser::audio {
// Device queries run on the capture worker, never an OS notification callback.
// A still-valid old endpoint can go silent when the default changes.
class LoopbackRouteWatch {
public:
  using Clock=std::chrono::steady_clock;
  explicit LoopbackRouteWatch(std::wstring endpoint,Clock::time_point now=Clock::now())
    : endpoint_(std::move(endpoint)),next_(now+std::chrono::milliseconds(250)) {}
  template<class Query>
  bool changed(Clock::time_point now,Query query) {
    if(now<next_) return false;
    next_=now+std::chrono::milliseconds(250);
    const std::optional<std::wstring> endpoint=query();
    return !endpoint || *endpoint!=endpoint_;
  }
private:
  std::wstring endpoint_;
  Clock::time_point next_;
};
}
