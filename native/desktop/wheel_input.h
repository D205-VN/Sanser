#pragma once
#include <array>
#include <cstdint>

namespace sanser::desktop {
// Accumulate Windows high-resolution wheel ticks rather than discarding every
// sub-notch message. The wire carries signed line steps, axis 0=vertical/1=horizontal.
class WheelInput {
public:
  std::int16_t steps(unsigned axis,std::int16_t ticks) {
    if(axis>1)return 0;
    auto& remainder=remainder_[axis];remainder+=ticks;
    const auto lines=remainder/120;remainder%=120;
    return static_cast<std::int16_t>(lines);
  }
  void reset() {remainder_={};}
private:
  std::array<int,2> remainder_{};
};
}
