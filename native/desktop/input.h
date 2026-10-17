#pragma once
#include <cstdint>

namespace sanser::desktop {
struct Input {
  // Absolute position uses [0,65535]; key codes use USB HID keyboard usages.
  enum Kind : std::uint8_t { Move, Button, Key, Scroll, Reset };
  Kind kind = Reset;
  std::uint16_t x = 0, y = 0, code = 0;
  std::int16_t delta = 0;
  bool down = false;
};
}
