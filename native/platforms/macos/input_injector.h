#pragma once
#include <mutex>
#import <ApplicationServices/ApplicationServices.h>
#include "transport.h"
#include <array>
#include <set>

namespace sanser::desktop {
class MacInputInjector {
public:
  explicit MacInputInjector(CGRect bounds,std::function<void(CGEventRef)> post={});
  ~MacInputInjector();
  void apply(const Input& input);
  // Display switch: later absolute pointer events map onto the new display.
  void setBounds(CGRect bounds) { std::lock_guard lock(boundsMutex_); bounds_=bounds; }
private:
  CGEventFlags flags() const;
  void emit(CGEventRef event,bool literal=false);
  void reset();
  std::mutex boundsMutex_;
  CGRect bounds_; CGPoint point_{};
  CGEventSourceRef source_=nullptr;
  std::function<void(CGEventRef)> post_;
  std::set<std::uint16_t> keys_,buttons_;
  bool capsLock_=false;
  struct Click { Clock::time_point at{}; CGPoint point{}; unsigned count=0; std::int64_t number=0; };
  std::array<Click,3> clicks_{};
  std::int64_t number_=0;
};
}
