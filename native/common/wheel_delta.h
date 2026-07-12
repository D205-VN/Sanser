#pragma once

#include <algorithm>
#include <cmath>

namespace sanser::input {

// Converts the vertical delta emitted by NSEvent into Windows wheel detents.
// Both platforms use a positive value for scrolling up. Discrete wheels need
// at least one detent, while precise trackpads keep their sub-detent movement.
inline double windowsWheelStepsFromCocoa(double deltaY, bool precise) noexcept {
  if (!std::isfinite(deltaY) || std::abs(deltaY) < 0.01) return 0.0;
  if (precise) return std::clamp(deltaY / 8.0, -3.0, 3.0);
  return std::copysign(std::clamp(std::abs(deltaY), 1.0, 3.0), deltaY);
}

} // namespace sanser::input
