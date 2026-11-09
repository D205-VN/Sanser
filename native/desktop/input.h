#pragma once
#include <cstdint>
#include <string>
#include "gamepad.h"

namespace sanser::desktop {
struct Input {
  // Absolute position uses [0,65535]; key codes use USB HID keyboard usages.
  // Scroll code: 0 vertical (positive up), 1 horizontal (positive right).
  enum Kind : std::uint8_t { Move, Button, Key, Scroll, Reset, Text, Gamepad };
  Kind kind = Reset;
  std::uint16_t x = 0, y = 0, code = 0;
  std::int16_t delta = 0;
  bool down = false;
  std::string text{};
  GamepadState gamepad{};
};
}
