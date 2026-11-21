#include "blocked_apps.h"
#include "control_cutoff_policy.h"
#include <dwmapi.h>
#include <algorithm>
#include <chrono>
#include <iterator>
#include <mutex>
#include <string>
#include <unordered_map>

namespace sanser::blocked {
namespace {
using Clock = std::chrono::steady_clock;

struct ProcessEntry { bool blocked = false; Clock::time_point checked{}; };

struct State {
  std::mutex mutex;
  List list;
  InputGate gate;
  std::unordered_map<DWORD, ProcessEntry> processes;
  std::vector<Rect> mask;
  Clock::time_point maskAt{};
};

State& state() { static State value; return value; }

std::string utf8(const wchar_t* text, DWORD length) {
  if (length == 0) return {};
  const int size = WideCharToMultiByte(CP_UTF8, 0, text, static_cast<int>(length), nullptr, 0, nullptr, nullptr);
  if (size <= 0) return {};
  std::string out(static_cast<std::size_t>(size), '\0');
  WideCharToMultiByte(CP_UTF8, 0, text, static_cast<int>(length), out.data(), size, nullptr, nullptr);
  return out;
}

// Caller holds the mutex. Image names are cached for two seconds per PID, so
// a reused PID is re-read quickly.
bool processBlocked(State& s, DWORD pid) {
  if (pid == 0 || pid == GetCurrentProcessId()) return false;
  const auto now = Clock::now();
  auto& entry = s.processes[pid];
  if (entry.checked.time_since_epoch().count() != 0 && now - entry.checked < std::chrono::seconds(2)) return entry.blocked;
  entry.checked = now;
  entry.blocked = false;
  if (HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid)) {
    wchar_t path[MAX_PATH * 2]{};
    DWORD length = static_cast<DWORD>(std::size(path));
    if (QueryFullProcessImageNameW(process, 0, path, &length)) entry.blocked = s.list.matchesPath(utf8(path, length));
    CloseHandle(process);
  }
  const bool blocked = entry.blocked;
  if (s.processes.size() > 4096) s.processes.clear();
  return blocked;
}

bool windowBlocked(State& s, HWND window) {
  if (!window) return false;
  HWND root = GetAncestor(window, GA_ROOT);
  DWORD pid = 0;
  GetWindowThreadProcessId(root ? root : window, &pid);
  return processBlocked(s, pid);
}

bool visibleInCapture(HWND window) {
  if (!IsWindowVisible(window) || IsIconic(window)) return false;
  BOOL cloaked = FALSE;
  if (SUCCEEDED(DwmGetWindowAttribute(window, DWMWA_CLOAKED, &cloaked, sizeof(cloaked))) && cloaked) return false;
  DWORD affinity = WDA_NONE;
  // Windows excluded from capture (the privacy overlay) neither show nor hide anything.
  if (GetWindowDisplayAffinity(window, &affinity) && affinity != WDA_NONE) return false;
  DWORD pid = 0;
  GetWindowThreadProcessId(window, &pid);
  if (pid == GetCurrentProcessId()) return false;
  if (GetWindowLongPtrW(window, GWL_EXSTYLE) & WS_EX_TRANSPARENT) return false;
  return true;
}

RECT frameOf(HWND window) {
  RECT rect{};
  if (FAILED(DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &rect, sizeof(rect)))) GetWindowRect(window, &rect);
  return rect;
}

struct Enumeration { State* state; HRGN covered; HRGN mask; };

BOOL CALLBACK collect(HWND window, LPARAM parameter) {
  auto& e = *reinterpret_cast<Enumeration*>(parameter);
  if (!visibleInCapture(window)) return TRUE;
  const RECT rect = frameOf(window);
  if (rect.right <= rect.left || rect.bottom <= rect.top) return TRUE;
  HRGN area = CreateRectRgn(rect.left, rect.top, rect.right, rect.bottom);
  if (!area) return TRUE;
  // EnumWindows walks top to bottom: only the part not covered by a window
  // above is visible, so only that part is painted black.
  if (windowBlocked(*e.state, window)) {
    HRGN visible = CreateRectRgn(0, 0, 0, 0);
    if (visible) {
      if (CombineRgn(visible, area, e.covered, RGN_DIFF) != ERROR) CombineRgn(e.mask, e.mask, visible, RGN_OR);
      DeleteObject(visible);
    }
  }
  CombineRgn(e.covered, e.covered, area, RGN_OR);
  DeleteObject(area);
  return TRUE;
}

std::vector<Rect> computeMask(State& s) {
  std::vector<Rect> rects;
  HRGN covered = CreateRectRgn(0, 0, 0, 0), mask = CreateRectRgn(0, 0, 0, 0);
  if (covered && mask) {
    Enumeration e{&s, covered, mask};
    EnumWindows(collect, reinterpret_cast<LPARAM>(&e));
    const DWORD size = GetRegionData(mask, 0, nullptr);
    if (size > sizeof(RGNDATAHEADER)) {
      std::vector<std::uint8_t> buffer(size);
      auto* data = reinterpret_cast<RGNDATA*>(buffer.data());
      if (GetRegionData(mask, size, data)) {
        const auto* list = reinterpret_cast<const RECT*>(data->Buffer);
        for (DWORD i = 0; i < data->rdh.nCount; ++i) rects.push_back({list[i].left, list[i].top, list[i].right, list[i].bottom});
      }
    }
  }
  if (covered) DeleteObject(covered);
  if (mask) DeleteObject(mask);
  return rects;
}

// A key-up, or a button-up without any press or wheel step.
bool isRelease(const INPUT& input) {
  if(input.type==INPUT_KEYBOARD) return (input.ki.dwFlags & KEYEVENTF_KEYUP)!=0;
  if(input.type!=INPUT_MOUSE) return false;
  constexpr DWORD releases=MOUSEEVENTF_LEFTUP|MOUSEEVENTF_RIGHTUP|MOUSEEVENTF_MIDDLEUP|MOUSEEVENTF_XUP;
  constexpr DWORD presses=MOUSEEVENTF_LEFTDOWN|MOUSEEVENTF_RIGHTDOWN|MOUSEEVENTF_MIDDLEDOWN|
                          MOUSEEVENTF_XDOWN|MOUSEEVENTF_WHEEL|MOUSEEVENTF_HWHEEL;
  return (input.mi.dwFlags & releases)!=0 && (input.mi.dwFlags & presses)==0;
}

bool isPress(const MOUSEINPUT& mouse) {
  constexpr DWORD presses = MOUSEEVENTF_LEFTDOWN | MOUSEEVENTF_RIGHTDOWN | MOUSEEVENTF_MIDDLEDOWN |
                            MOUSEEVENTF_XDOWN | MOUSEEVENTF_WHEEL | MOUSEEVENTF_HWHEEL;
  return (mouse.dwFlags & presses) != 0;
}

POINT pointOf(const MOUSEINPUT& mouse) {
  if (!(mouse.dwFlags & MOUSEEVENTF_ABSOLUTE)) { POINT cursor{}; GetCursorPos(&cursor); return cursor; }
  const bool virtualDesk = (mouse.dwFlags & MOUSEEVENTF_VIRTUALDESK) != 0;
  const long left = virtualDesk ? GetSystemMetrics(SM_XVIRTUALSCREEN) : 0;
  const long top = virtualDesk ? GetSystemMetrics(SM_YVIRTUALSCREEN) : 0;
  const long width = std::max(1, GetSystemMetrics(virtualDesk ? SM_CXVIRTUALSCREEN : SM_CXSCREEN) - 1);
  const long height = std::max(1, GetSystemMetrics(virtualDesk ? SM_CYVIRTUALSCREEN : SM_CYSCREEN) - 1);
  return {left + static_cast<long>(static_cast<long long>(mouse.dx) * width / 65535),
          top + static_cast<long>(static_cast<long long>(mouse.dy) * height / 65535)};
}

Key keyOf(const KEYBDINPUT& key) {
  Key out;
  out.down = (key.dwFlags & KEYEVENTF_KEYUP) == 0;
  if (key.dwFlags & KEYEVENTF_UNICODE) return out;
  if (key.dwFlags & KEYEVENTF_SCANCODE) {
    const bool extended = (key.dwFlags & KEYEVENTF_EXTENDEDKEY) != 0;
    switch (key.wScan) {
      case 0x2A: case 0x36: case 0x1D: out.modifier = true; break;           // Shift, Ctrl
      case 0x38: out.modifier = true; out.switcher = true; break;            // Alt
      case 0x5B: case 0x5C: out.modifier = extended; out.switcher = extended; break; // Windows
      case 0x0F: out.tab = true; break;
      default: break;
    }
    return out;
  }
  switch (key.wVk) {
    case VK_SHIFT: case VK_LSHIFT: case VK_RSHIFT: case VK_CONTROL: case VK_LCONTROL: case VK_RCONTROL:
      out.modifier = true; break;
    case VK_MENU: case VK_LMENU: case VK_RMENU: case VK_LWIN: case VK_RWIN:
      out.modifier = true; out.switcher = true; break;
    case VK_TAB: out.tab = true; break;
    default: break;
  }
  return out;
}
}

void configureWindows(const List& list) {
  auto& s = state();
  std::lock_guard lock(s.mutex);
  s.list = list;
  s.processes.clear();
  s.mask.clear();
  s.gate.reset();
}

bool windowsEnabled() {
  auto& s = state();
  std::lock_guard lock(s.mutex);
  return !s.list.empty();
}

std::vector<Rect> windowsMask() {
  auto& s = state();
  std::lock_guard lock(s.mutex);
  if (s.list.empty()) return {};
  // At 60 fps this walks the window list at most every 15 ms.
  const auto now = Clock::now();
  if (now - s.maskAt >= std::chrono::milliseconds(15)) { s.mask = computeMask(s); s.maskAt = now; }
  return s.mask;
}

bool windowsInputAllowed(const INPUT* inputs, UINT count) {
  // Ctrl+Alt+K at the host: only releases pass for the rest of the session.
  if(cutoff::revoked() && inputs) {
    for(UINT i=0;i<count;++i) if(!cutoff::allows(isRelease(inputs[i]))) return false;
  }
  auto& s = state();
  std::lock_guard lock(s.mutex);
  if (s.list.empty() || !inputs || count == 0) return true;
  bool allowed = true;
  for (UINT i = 0; i < count; ++i) {
    const auto& input = inputs[i];
    if (input.type == INPUT_KEYBOARD) {
      const auto key = keyOf(input.ki);
      const bool front = key.down && !key.modifier && windowBlocked(s, GetForegroundWindow());
      if (!s.gate.allowKey(key, front)) allowed = false;
    } else if (input.type == INPUT_MOUSE && isPress(input.mi)) {
      if (!s.gate.allowPointer(true, windowBlocked(s, WindowFromPoint(pointOf(input.mi))))) allowed = false;
    }
  }
  return allowed;
}

std::uint64_t windowsDropped() {
  auto& s = state();
  std::lock_guard lock(s.mutex);
  return s.gate.dropped();
}
}
