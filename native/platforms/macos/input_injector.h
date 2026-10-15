#pragma once
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
private:
  CGEventFlags flags() const;
  void emit(CGEventRef event);
  void reset();
  CGRect bounds_; CGPoint point_{};
  CGEventSourceRef source_=nullptr;
  std::function<void(CGEventRef)> post_;
  std::set<std::uint16_t> keys_,buttons_;
  struct Click { Clock::time_point at{}; CGPoint point{}; unsigned count=0; std::int64_t number=0; };
  std::array<Click,3> clicks_{};
  std::int64_t number_=0;
};
}
