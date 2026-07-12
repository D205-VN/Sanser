#include "wheel_delta.h"

#include <cmath>
#include <iostream>
#include <limits>

namespace {

bool near(double actual, double expected) {
  return std::abs(actual - expected) < 0.0001;
}

bool requireStep(double actual, double expected, const char* label) {
  if (near(actual, expected)) return true;
  std::cerr << label << ": expected " << expected << ", got " << actual << '\n';
  return false;
}

} // namespace

int main() {
  using sanser::input::windowsWheelStepsFromCocoa;
  bool valid = true;
  valid &= requireStep(windowsWheelStepsFromCocoa(0.1, false), 1.0, "discrete up");
  valid &= requireStep(windowsWheelStepsFromCocoa(-0.1, false), -1.0, "discrete down");
  valid &= requireStep(windowsWheelStepsFromCocoa(8.0, true), 1.0, "precise up");
  valid &= requireStep(windowsWheelStepsFromCocoa(-16.0, true), -2.0, "precise down");
  valid &= requireStep(windowsWheelStepsFromCocoa(40.0, true), 3.0, "precise clamp");
  valid &= requireStep(windowsWheelStepsFromCocoa(0.0, false), 0.0, "zero");
  valid &= requireStep(
    windowsWheelStepsFromCocoa(std::numeric_limits<double>::quiet_NaN(), false),
    0.0,
    "invalid"
  );
  return valid ? 0 : 1;
}
