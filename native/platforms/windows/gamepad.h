#pragma once
#include "../../desktop/gamepad.h"
#include <functional>
#include <memory>

namespace sanser::desktop {
// Optional compatibility with an already installed virtual-controller backend.
// Does not download DLLs/drivers, elevate, or fall back to keyboard input.
class WindowsGamepad {
public:
  struct Driver {
    std::function<bool()> attach;
    std::function<bool(GamepadState)> update;
    std::function<void()> detach;
  };
  explicit WindowsGamepad(bool enabled);
  explicit WindowsGamepad(Driver driver); // Fault-injection seam; no OS input.
  ~WindowsGamepad();
  WindowsGamepad(const WindowsGamepad&)=delete;
  WindowsGamepad& operator=(const WindowsGamepad&)=delete;
  bool available() const;
  bool apply(GamepadState state);
private:
  struct State;std::unique_ptr<State> state_;
};
}
