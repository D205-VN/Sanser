#include "bmp_writer.h"
#include "desktop_duplication.h"
#include "mf_video_encoder.h"
#include "mf_video_packet_encoder.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <limits>
#include <memory>
#include <mutex>
#include <string>
#include <string_view>
#include <thread>
#include <vector>

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#endif

struct InputBounds {
  int left = 0;
  int top = 0;
  int width = 1;
  int height = 1;
};

InputBounds gInputBounds;
std::mutex gInputBoundsMutex;

void makeProcessDpiAware() {
#ifdef _WIN32
  SetProcessDPIAware();
#endif
}

struct Options {
  std::filesystem::path outputDir = "captures";
  std::filesystem::path outputFile = "captures/capture_h264.mp4";
  std::filesystem::path packetFile;
  std::string tcpConnect;
  std::uint32_t frames = 1;
  std::uint32_t intervalMs = 250;
  std::uint32_t adapterIndex = 0;
  std::uint32_t outputIndex = 0;
  std::uint32_t fps = 30;
  std::uint32_t bitrate = 28000000;
  VideoCodec codec = VideoCodec::H264;
  bool pipe = false;
  bool encode = false;
  bool encodePipe = false;
  bool framesProvided = false;
  bool hardwareEncoder = true;
  bool listEncoders = false;
};

struct PipeFrameHeader {
  char magic[4] = {'S', 'N', 'F', '1'};
  std::uint32_t headerSize = sizeof(PipeFrameHeader);
  std::uint32_t width = 0;
  std::uint32_t height = 0;
  std::uint32_t stride = 0;
  std::uint32_t pixelFormat = 1; // 1 = BGRA8
  std::uint64_t timestampMicros = 0;
  std::uint32_t payloadSize = 0;
};

#pragma pack(push, 1)
struct EncodedPacketHeader {
  char magic[4] = {'S', 'N', 'V', '1'};
  std::uint32_t headerSize = sizeof(EncodedPacketHeader);
  std::uint32_t codec = 1; // 1 = H.264
  std::uint32_t packetFormat = 1; // 1 = compressed sample bytes as emitted by Media Foundation
  std::uint32_t width = 0;
  std::uint32_t height = 0;
  std::uint64_t sequence = 0;
  std::uint64_t timestampMicros = 0;
  std::uint32_t durationMicros = 0;
  std::uint32_t flags = 0; // bit 0 = keyframe, bit 1 = hardware encoder
  std::uint32_t payloadSize = 0;
};
#pragma pack(pop)

#ifdef _WIN32
#pragma pack(push, 1)
struct ControlMessageHeader {
  char magic[4] = {'S', 'N', 'I', '1'};
  std::uint32_t headerSize = sizeof(ControlMessageHeader);
  std::uint32_t payloadSize = 0;
};
#pragma pack(pop)
#endif

std::uint32_t readUintArg(int argc, char** argv, int& i, const char* name) {
  if (i + 1 >= argc) {
    throw std::runtime_error(std::string("Missing value for ") + name);
  }
  return static_cast<std::uint32_t>(std::stoul(argv[++i]));
}

Options parseOptions(int argc, char** argv) {
  Options options;
  for (int i = 1; i < argc; ++i) {
    const std::string arg = argv[i];
    if (arg == "--output-dir") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --output-dir");
      options.outputDir = argv[++i];
    } else if (arg == "--output-file") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --output-file");
      options.outputFile = argv[++i];
    } else if (arg == "--packet-file") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --packet-file");
      options.packetFile = argv[++i];
    } else if (arg == "--tcp-connect") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --tcp-connect");
      options.tcpConnect = argv[++i];
    } else if (arg == "--frames") {
      options.frames = readUintArg(argc, argv, i, "--frames");
      options.framesProvided = true;
    } else if (arg == "--interval-ms") {
      options.intervalMs = readUintArg(argc, argv, i, "--interval-ms");
    } else if (arg == "--adapter") {
      options.adapterIndex = readUintArg(argc, argv, i, "--adapter");
    } else if (arg == "--output") {
      options.outputIndex = readUintArg(argc, argv, i, "--output");
    } else if (arg == "--fps") {
      options.fps = readUintArg(argc, argv, i, "--fps");
    } else if (arg == "--bitrate") {
      options.bitrate = readUintArg(argc, argv, i, "--bitrate");
    } else if (arg == "--pipe") {
      options.pipe = true;
    } else if (arg == "--encode") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --encode");
      options.codec = parseVideoCodec(argv[++i]);
      options.encode = true;
    } else if (arg == "--encode-pipe") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --encode-pipe");
      options.codec = parseVideoCodec(argv[++i]);
      options.encodePipe = true;
    } else if (arg == "--software-encoder") {
      options.hardwareEncoder = false;
    } else if (arg == "--list-encoders") {
      options.listEncoders = true;
    } else if (arg == "--help" || arg == "-h") {
      std::cout
        << "sanser-native-host --frames 10 --interval-ms 100 --output-dir captures\n"
        << "sanser-native-host --pipe --fps 60\n"
        << "sanser-native-host --list-encoders\n"
        << "sanser-native-host --encode h264 --frames 300 --fps 60 --output-file captures/capture_h264.mp4\n"
        << "sanser-native-host --encode-pipe h264 --fps 60 --packet-file native-captures/capture.snv\n"
        << "sanser-native-host --encode-pipe h264 --fps 60 --tcp-connect 100.x.y.z:7777\n"
        << "  --adapter N      DXGI adapter index, default 0\n"
        << "  --output N       DXGI output/monitor index, default 0\n"
        << "  --pipe           Write BGRA frames to stdout with SNF1 headers\n"
        << "  --fps N          Target FPS in pipe/encode mode, default 30\n"
        << "  --encode CODEC   Encode captured frames with Media Foundation: h264, hevc, av1\n"
        << "  --encode-pipe CODEC Encode H.264 packets to stdout or --packet-file with SNV1 headers\n"
        << "  --bitrate N      Target encode bitrate, default 28000000\n"
        << "  --output-file P  Encoded output file path\n"
        << "  --packet-file P  SNV1 packet output file; stdout is used when omitted\n"
        << "  --tcp-connect H:P Connect to a TCP SNV1 receiver, stream video, receive native input\n"
        << "  --list-encoders  List Media Foundation hardware encoders\n"
        << "  --software-encoder Disable hardware transform request\n";
      std::exit(0);
    } else {
      throw std::runtime_error("Unknown argument: " + arg);
    }
  }
  return options;
}

std::uint64_t nowMicros() {
  const auto now = std::chrono::steady_clock::now().time_since_epoch();
  return static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(now).count());
}

#ifdef _WIN32
class WinsockRuntime {
public:
  WinsockRuntime() {
    WSADATA data{};
    const int result = WSAStartup(MAKEWORD(2, 2), &data);
    if (result != 0) {
      throw std::runtime_error("WSAStartup failed: " + std::to_string(result));
    }
  }

  ~WinsockRuntime() {
    WSACleanup();
  }

  WinsockRuntime(const WinsockRuntime&) = delete;
  WinsockRuntime& operator=(const WinsockRuntime&) = delete;
};

std::size_t skipJsonWhitespace(const std::string& json, std::size_t offset) {
  while (offset < json.size()) {
    const char ch = json[offset];
    if (ch != ' ' && ch != '\n' && ch != '\r' && ch != '\t') break;
    ++offset;
  }
  return offset;
}

std::size_t findJsonValue(const std::string& json, const char* key) {
  const std::string needle = std::string("\"") + key + "\"";
  const std::size_t keyOffset = json.find(needle);
  if (keyOffset == std::string::npos) return std::string::npos;
  const std::size_t colon = json.find(':', keyOffset + needle.size());
  if (colon == std::string::npos) return std::string::npos;
  return skipJsonWhitespace(json, colon + 1);
}

std::string jsonUnescape(std::string_view value) {
  std::string out;
  out.reserve(value.size());
  for (std::size_t i = 0; i < value.size(); ++i) {
    const char ch = value[i];
    if (ch != '\\' || i + 1 >= value.size()) {
      out.push_back(ch);
      continue;
    }
    const char escaped = value[++i];
    switch (escaped) {
      case 'n': out.push_back('\n'); break;
      case 'r': out.push_back('\r'); break;
      case 't': out.push_back('\t'); break;
      case '\\': out.push_back('\\'); break;
      case '"': out.push_back('"'); break;
      default: out.push_back(escaped); break;
    }
  }
  return out;
}

std::string jsonStringValue(const std::string& json, const char* key, const std::string& fallback = {}) {
  std::size_t offset = findJsonValue(json, key);
  if (offset == std::string::npos || offset >= json.size() || json[offset] != '"') return fallback;
  ++offset;

  std::string raw;
  while (offset < json.size()) {
    const char ch = json[offset++];
    if (ch == '"') return jsonUnescape(raw);
    if (ch == '\\' && offset < json.size()) {
      raw.push_back(ch);
      raw.push_back(json[offset++]);
    } else {
      raw.push_back(ch);
    }
  }
  return fallback;
}

double jsonDoubleValue(const std::string& json, const char* key, double fallback = 0.0) {
  const std::size_t offset = findJsonValue(json, key);
  if (offset == std::string::npos) return fallback;
  try {
    return std::stod(json.substr(offset));
  } catch (...) {
    return fallback;
  }
}

int jsonIntValue(const std::string& json, const char* key, int fallback = 0) {
  const std::size_t offset = findJsonValue(json, key);
  if (offset == std::string::npos) return fallback;
  try {
    return std::stoi(json.substr(offset));
  } catch (...) {
    return fallback;
  }
}

std::wstring utf8ToWide(const std::string& value) {
  if (value.empty()) return {};
  const int required = MultiByteToWideChar(CP_UTF8, 0, value.data(), static_cast<int>(value.size()), nullptr, 0);
  if (required <= 0) return {};
  std::wstring wide(static_cast<std::size_t>(required), L'\0');
  MultiByteToWideChar(CP_UTF8, 0, value.data(), static_cast<int>(value.size()), wide.data(), required);
  return wide;
}

bool movePointerNormalized(double normalizedX, double normalizedY) {
  InputBounds bounds;
  {
    std::lock_guard<std::mutex> lock(gInputBoundsMutex);
    bounds = gInputBounds;
  }
  const int left = bounds.left;
  const int top = bounds.top;
  const int width = std::max(bounds.width, 1);
  const int height = std::max(bounds.height, 1);

  const double x = std::clamp(normalizedX, 0.0, 1.0);
  const double y = std::clamp(normalizedY, 0.0, 1.0);
  const int screenX = left + static_cast<int>(std::lround(x * static_cast<double>(width - 1)));
  const int screenY = top + static_cast<int>(std::lround(y * static_cast<double>(height - 1)));
  if (SetCursorPos(screenX, screenY) != 0) return true;

  const int virtualLeft = GetSystemMetrics(SM_XVIRTUALSCREEN);
  const int virtualTop = GetSystemMetrics(SM_YVIRTUALSCREEN);
  const int virtualWidth = std::max(GetSystemMetrics(SM_CXVIRTUALSCREEN), 1);
  const int virtualHeight = std::max(GetSystemMetrics(SM_CYVIRTUALSCREEN), 1);
  INPUT input{};
  input.type = INPUT_MOUSE;
  input.mi.dx = static_cast<LONG>(std::lround((screenX - virtualLeft) * 65535.0 / std::max(virtualWidth - 1, 1)));
  input.mi.dy = static_cast<LONG>(std::lround((screenY - virtualTop) * 65535.0 / std::max(virtualHeight - 1, 1)));
  input.mi.dwFlags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
  SendInput(1, &input, sizeof(input));

  POINT actual{};
  if (!GetCursorPos(&actual)) return false;
  return std::abs(actual.x - screenX) <= 2 && std::abs(actual.y - screenY) <= 2;
}

DWORD mouseButtonFlag(int button, bool down) {
  if (button == 1) return down ? MOUSEEVENTF_MIDDLEDOWN : MOUSEEVENTF_MIDDLEUP;
  if (button == 2) return down ? MOUSEEVENTF_RIGHTDOWN : MOUSEEVENTF_RIGHTUP;
  return down ? MOUSEEVENTF_LEFTDOWN : MOUSEEVENTF_LEFTUP;
}

DWORD normalizeMacMouseButton(int button, bool down) {
  if (button == 1) return down ? MOUSEEVENTF_RIGHTDOWN : MOUSEEVENTF_RIGHTUP;
  if (button == 2) return down ? MOUSEEVENTF_MIDDLEDOWN : MOUSEEVENTF_MIDDLEUP;
  return down ? MOUSEEVENTF_LEFTDOWN : MOUSEEVENTF_LEFTUP;
}

bool sendMouseButton(int button, bool down) {
  INPUT input{};
  input.type = INPUT_MOUSE;
  input.mi.dwFlags = normalizeMacMouseButton(button, down);
  if (SendInput(1, &input, sizeof(input)) == 1) return true;
  mouse_event(input.mi.dwFlags, 0, 0, 0, 0);
  return GetLastError() == ERROR_SUCCESS;
}

bool sendWheel(double deltaY) {
  if (std::abs(deltaY) < 0.01) return true;
  const double wheelSteps = std::clamp(-deltaY / 8.0, -3.0, 3.0);
  const double clamped = std::clamp(wheelSteps * static_cast<double>(WHEEL_DELTA), -360.0, 360.0);
  INPUT input{};
  input.type = INPUT_MOUSE;
  input.mi.dwFlags = MOUSEEVENTF_WHEEL;
  input.mi.mouseData = static_cast<DWORD>(static_cast<LONG>(std::lround(clamped)));
  if (SendInput(1, &input, sizeof(input)) == 1) return true;
  mouse_event(MOUSEEVENTF_WHEEL, 0, 0, static_cast<DWORD>(static_cast<LONG>(std::lround(clamped))), 0);
  return GetLastError() == ERROR_SUCCESS;
}

WORD macKeyCodeToVirtualKey(int keyCode, const std::string& keyText) {
  switch (keyCode) {
    case 0: return 'A';
    case 1: return 'S';
    case 2: return 'D';
    case 3: return 'F';
    case 4: return 'H';
    case 5: return 'G';
    case 6: return 'Z';
    case 7: return 'X';
    case 8: return 'C';
    case 9: return 'V';
    case 11: return 'B';
    case 12: return 'Q';
    case 13: return 'W';
    case 14: return 'E';
    case 15: return 'R';
    case 16: return 'Y';
    case 17: return 'T';
    case 18: return '1';
    case 19: return '2';
    case 20: return '3';
    case 21: return '4';
    case 22: return '6';
    case 23: return '5';
    case 24: return VK_OEM_PLUS;
    case 25: return '9';
    case 26: return '7';
    case 27: return VK_OEM_MINUS;
    case 28: return '8';
    case 29: return '0';
    case 30: return VK_OEM_6;
    case 31: return 'O';
    case 32: return 'U';
    case 33: return VK_OEM_4;
    case 34: return 'I';
    case 35: return 'P';
    case 36: return VK_RETURN;
    case 37: return 'L';
    case 38: return 'J';
    case 39: return VK_OEM_7;
    case 40: return 'K';
    case 41: return VK_OEM_1;
    case 42: return VK_OEM_5;
    case 43: return VK_OEM_COMMA;
    case 44: return VK_OEM_2;
    case 45: return 'N';
    case 46: return 'M';
    case 47: return VK_OEM_PERIOD;
    case 48: return VK_TAB;
    case 49: return VK_SPACE;
    case 50: return VK_OEM_3;
    case 51: return VK_BACK;
    case 53: return VK_ESCAPE;
    case 54:
    case 55: return VK_CONTROL;
    case 56:
    case 60: return VK_SHIFT;
    case 58:
    case 61: return VK_MENU;
    case 59:
    case 62: return VK_CONTROL;
    case 63: return 0;
    case 115: return VK_HOME;
    case 116: return VK_PRIOR;
    case 117: return VK_DELETE;
    case 119: return VK_END;
    case 121: return VK_NEXT;
    case 123: return VK_LEFT;
    case 124: return VK_RIGHT;
    case 125: return VK_DOWN;
    case 126: return VK_UP;
    default:
      break;
  }

  if (!keyText.empty()) {
    const std::wstring wide = utf8ToWide(keyText);
    if (!wide.empty()) {
      const SHORT mapped = VkKeyScanW(wide[0]);
      if (mapped != -1) return static_cast<WORD>(mapped & 0xff);
    }
  }
  return 0;
}

bool sendVirtualKey(WORD virtualKey, bool down) {
  if (virtualKey == 0) return false;
  INPUT input{};
  input.type = INPUT_KEYBOARD;
  input.ki.wVk = virtualKey;
  input.ki.dwFlags = down ? 0 : KEYEVENTF_KEYUP;
  if (SendInput(1, &input, sizeof(input)) == 1) return true;
  keybd_event(static_cast<BYTE>(virtualKey), 0, down ? 0 : KEYEVENTF_KEYUP, 0);
  return GetLastError() == ERROR_SUCCESS;
}

void sendShortcut(WORD virtualKey) {
  sendVirtualKey(VK_CONTROL, true);
  sendVirtualKey(virtualKey, true);
  sendVirtualKey(virtualKey, false);
  sendVirtualKey(VK_CONTROL, false);
}

void setClipboardText(const std::string& text) {
  const std::wstring wide = utf8ToWide(text);
  const SIZE_T bytes = (wide.size() + 1) * sizeof(wchar_t);
  HGLOBAL memory = GlobalAlloc(GMEM_MOVEABLE, bytes);
  if (!memory) return;

  void* target = GlobalLock(memory);
  if (!target) {
    GlobalFree(memory);
    return;
  }
  std::memcpy(target, wide.c_str(), bytes);
  GlobalUnlock(memory);

  if (!OpenClipboard(nullptr)) {
    GlobalFree(memory);
    return;
  }
  EmptyClipboard();
  if (!SetClipboardData(CF_UNICODETEXT, memory)) {
    GlobalFree(memory);
  }
  CloseClipboard();
}

void handleModifierState(int keyCode, int modifiers) {
  constexpr int shiftMask = 1 << 17;
  constexpr int controlMask = 1 << 18;
  constexpr int optionMask = 1 << 19;
  constexpr int commandMask = 1 << 20;

  WORD virtualKey = macKeyCodeToVirtualKey(keyCode, {});
  bool down = false;
  if (keyCode == 56 || keyCode == 60) down = (modifiers & shiftMask) != 0;
  if (keyCode == 58 || keyCode == 61) down = (modifiers & optionMask) != 0;
  if (keyCode == 59 || keyCode == 62) down = (modifiers & controlMask) != 0;
  if (keyCode == 54 || keyCode == 55) down = (modifiers & commandMask) != 0;
  sendVirtualKey(virtualKey, down);
}

void handleControlPayload(const std::string& json) {
  const std::string type = jsonStringValue(json, "type");
  if (type == "pointer-move") {
    const double x = jsonDoubleValue(json, "x");
    const double y = jsonDoubleValue(json, "y");
    const bool moved = movePointerNormalized(x, y);
    static auto lastMoveLog = std::chrono::steady_clock::time_point{};
    const auto now = std::chrono::steady_clock::now();
    if (now - lastMoveLog > std::chrono::milliseconds(250)) {
      lastMoveLog = now;
      POINT cursor{};
      GetCursorPos(&cursor);
      std::cerr << "SNINPUT_APPLIED pointer-move"
                << " x=" << x
                << " y=" << y
                << " moved=" << (moved ? "yes" : "no")
                << " cursor=" << cursor.x << "," << cursor.y
                << "\n";
    }
  } else if (type == "pointer-down" || type == "pointer-up") {
    const double x = jsonDoubleValue(json, "x");
    const double y = jsonDoubleValue(json, "y");
    const bool moved = movePointerNormalized(x, y);
    const bool clicked = sendMouseButton(jsonIntValue(json, "button"), type == "pointer-down");
    POINT cursor{};
    GetCursorPos(&cursor);
    std::cerr << "SNINPUT_APPLIED " << type
              << " button=" << jsonIntValue(json, "button")
              << " x=" << x
              << " y=" << y
              << " moved=" << (moved ? "yes" : "no")
              << " sent=" << (clicked ? "yes" : "no")
              << " cursor=" << cursor.x << "," << cursor.y
              << "\n";
  } else if (type == "wheel") {
    const bool moved = movePointerNormalized(jsonDoubleValue(json, "x"), jsonDoubleValue(json, "y"));
    const bool wheeled = sendWheel(jsonDoubleValue(json, "dy"));
    std::cerr << "SNINPUT_APPLIED wheel dy=" << jsonDoubleValue(json, "dy")
              << " moved=" << (moved ? "yes" : "no")
              << " sent=" << (wheeled ? "yes" : "no")
              << "\n";
  } else if (type == "key-down" || type == "key-up") {
    const WORD virtualKey = macKeyCodeToVirtualKey(jsonIntValue(json, "keyCode", -1),
                                                   jsonStringValue(json, "key"));
    const bool sent = sendVirtualKey(virtualKey, type == "key-down");
    std::cerr << "SNINPUT_APPLIED " << type
              << " vk=" << virtualKey
              << " sent=" << (sent ? "yes" : "no")
              << "\n";
  } else if (type == "modifiers") {
    handleModifierState(jsonIntValue(json, "keyCode", -1), jsonIntValue(json, "modifiers", 0));
  } else if (type == "clipboard") {
    setClipboardText(jsonStringValue(json, "text"));
  } else if (type == "copy") {
    sendShortcut('C');
  } else if (type == "cut") {
    sendShortcut('X');
  } else if (type == "paste") {
    sendShortcut('V');
  } else if (type == "select-all") {
    sendShortcut('A');
  }
}

class TcpClient {
public:
  explicit TcpClient(const std::string& endpoint) {
    const auto separator = endpoint.rfind(':');
    if (separator == std::string::npos || separator == 0 || separator == endpoint.size() - 1) {
      throw std::runtime_error("--tcp-connect must be HOST:PORT");
    }
    const std::string host = endpoint.substr(0, separator);
    const std::string port = endpoint.substr(separator + 1);

    addrinfo hints{};
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;

    addrinfo* results = nullptr;
    const int lookup = getaddrinfo(host.c_str(), port.c_str(), &hints, &results);
    if (lookup != 0) {
      throw std::runtime_error("getaddrinfo failed for " + endpoint + ": " + std::to_string(lookup));
    }

    for (addrinfo* item = results; item; item = item->ai_next) {
      SOCKET candidate = socket(item->ai_family, item->ai_socktype, item->ai_protocol);
      if (candidate == INVALID_SOCKET) continue;

      if (connect(candidate, item->ai_addr, static_cast<int>(item->ai_addrlen)) == 0) {
        socket_ = candidate;
        break;
      }

      closesocket(candidate);
    }
    freeaddrinfo(results);

    if (socket_ == INVALID_SOCKET) {
      throw std::runtime_error("Could not connect to " + endpoint + ", WSA error " + std::to_string(WSAGetLastError()));
    }

    int noDelay = 1;
    setsockopt(socket_, IPPROTO_TCP, TCP_NODELAY, reinterpret_cast<const char*>(&noDelay), sizeof(noDelay));
  }

  ~TcpClient() {
    if (socket_ != INVALID_SOCKET) {
      running_ = false;
      shutdown(socket_, SD_BOTH);
    }
    if (controlThread_.joinable()) {
      controlThread_.join();
    }
    if (socket_ != INVALID_SOCKET) {
      closesocket(socket_);
    }
  }

  TcpClient(const TcpClient&) = delete;
  TcpClient& operator=(const TcpClient&) = delete;

  void sendAll(const void* data, std::size_t size) {
    const char* cursor = static_cast<const char*>(data);
    std::size_t remaining = size;
    while (remaining > 0) {
      const int chunk = remaining > static_cast<std::size_t>(std::numeric_limits<int>::max())
        ? std::numeric_limits<int>::max()
        : static_cast<int>(remaining);
      const int sent = send(socket_, cursor, chunk, 0);
      if (sent == SOCKET_ERROR || sent == 0) {
        throw std::runtime_error("TCP send failed, WSA error " + std::to_string(WSAGetLastError()));
      }
      cursor += sent;
      remaining -= static_cast<std::size_t>(sent);
    }
  }

  void startControlReceiver() {
    if (controlThread_.joinable()) return;
    running_ = true;
    controlThread_ = std::thread([this]() { controlLoop(); });
  }

private:
  bool recvAll(void* data, std::size_t size) {
    char* cursor = static_cast<char*>(data);
    std::size_t remaining = size;
    while (remaining > 0 && running_) {
      const int chunk = remaining > static_cast<std::size_t>(std::numeric_limits<int>::max())
        ? std::numeric_limits<int>::max()
        : static_cast<int>(remaining);
      const int received = recv(socket_, cursor, chunk, 0);
      if (received == 0) return false;
      if (received == SOCKET_ERROR) {
        if (!running_) return false;
        std::cerr << "SNINPUT receive failed, WSA error " << WSAGetLastError() << "\n";
        return false;
      }
      cursor += received;
      remaining -= static_cast<std::size_t>(received);
    }
    return remaining == 0;
  }

  void controlLoop() {
    std::cerr << "SNINPUT control backchannel enabled.\n";
    while (running_) {
      ControlMessageHeader header{};
      if (!recvAll(&header, sizeof(header))) break;
      if (std::memcmp(header.magic, "SNI1", 4) != 0 || header.headerSize < sizeof(ControlMessageHeader)) {
        std::cerr << "SNINPUT invalid control header.\n";
        break;
      }
      if (header.payloadSize > 4 * 1024 * 1024) {
        std::cerr << "SNINPUT payload too large: " << header.payloadSize << "\n";
        break;
      }
      if (header.headerSize > sizeof(ControlMessageHeader)) {
        std::vector<char> extra(header.headerSize - sizeof(ControlMessageHeader));
        if (!recvAll(extra.data(), extra.size())) break;
      }

      std::string payload(header.payloadSize, '\0');
      if (!payload.empty() && !recvAll(payload.data(), payload.size())) break;
      try {
        handleControlPayload(payload);
      } catch (const std::exception& error) {
        std::cerr << "SNINPUT control error: " << error.what() << "\n";
      }
    }
    std::cerr << "SNINPUT control backchannel closed.\n";
  }

  SOCKET socket_ = INVALID_SOCKET;
  std::atomic<bool> running_{false};
  std::thread controlThread_;
};
#endif

void writePipeFrame(const FrameBgra& frame) {
  PipeFrameHeader header;
  header.width = frame.width;
  header.height = frame.height;
  header.stride = frame.stride;
  header.timestampMicros = nowMicros();
  header.payloadSize = static_cast<std::uint32_t>(frame.pixels.size());

  std::cout.write(reinterpret_cast<const char*>(&header), sizeof(header));
  std::cout.write(reinterpret_cast<const char*>(frame.pixels.data()), frame.pixels.size());
  std::cout.flush();
}

std::vector<std::uint8_t> makeEncodedPacketBytes(const EncodedVideoPacket& packet,
                                                 std::uint32_t width,
                                                 std::uint32_t height,
                                                 std::uint64_t sequence,
                                                 bool hardware) {
  EncodedPacketHeader header;
  header.width = width;
  header.height = height;
  header.sequence = sequence;
  header.timestampMicros = packet.timestampMicros;
  header.durationMicros = packet.durationMicros;
  header.flags = (packet.keyframe ? 1u : 0u) | (hardware ? 2u : 0u);
  header.payloadSize = static_cast<std::uint32_t>(packet.payload.size());

  std::vector<std::uint8_t> bytes(sizeof(header) + packet.payload.size());
  std::memcpy(bytes.data(), &header, sizeof(header));
  if (!packet.payload.empty()) {
    std::memcpy(bytes.data() + sizeof(header), packet.payload.data(), packet.payload.size());
  }
  return bytes;
}

void writeEncodedPacket(std::ostream& output,
                        const EncodedVideoPacket& packet,
                        std::uint32_t width,
                        std::uint32_t height,
                        std::uint64_t sequence,
                        bool hardware) {
  const auto bytes = makeEncodedPacketBytes(packet, width, height, sequence, hardware);
  output.write(reinterpret_cast<const char*>(bytes.data()), bytes.size());
}

int runPipeMode(DesktopDuplicator& duplicator, const Options& options) {
#ifdef _WIN32
  _setmode(_fileno(stdout), _O_BINARY);
#endif
  const auto frameDelay = options.fps > 0
    ? std::chrono::microseconds(1000000 / options.fps)
    : std::chrono::microseconds(0);
  while (true) {
    const auto frameStartedAt = std::chrono::steady_clock::now();
    FrameBgra frame;
    if (duplicator.captureFrame(frame, 1000)) {
      writePipeFrame(frame);
    }

    if (frameDelay.count() > 0) {
      std::this_thread::sleep_until(frameStartedAt + frameDelay);
    }
  }
}

int runEncodeMode(DesktopDuplicator& duplicator, const Options& options) {
  if (options.outputFile.has_parent_path()) {
    std::filesystem::create_directories(options.outputFile.parent_path());
  }

  VideoEncodeOptions encodeOptions;
  encodeOptions.codec = options.codec;
  encodeOptions.outputFile = options.outputFile;
  encodeOptions.fps = options.fps;
  encodeOptions.bitrate = options.bitrate;
  encodeOptions.hardware = options.hardwareEncoder;

  MfVideoFileEncoder encoder(duplicator.width(), duplicator.height(), encodeOptions);

  std::cout << "Encoding " << options.frames << " frame(s) to "
            << options.outputFile.string() << " as " << videoCodecName(options.codec)
            << " at " << options.fps << " FPS, " << options.bitrate << " bps"
            << (options.hardwareEncoder ? " with hardware transforms requested" : " with software encoder")
            << "\n";

  std::uint32_t encoded = 0;
  while (encoded < options.frames) {
    const auto frameStartedAt = std::chrono::steady_clock::now();
    FrameBgra frame;
    if (!duplicator.captureFrame(frame, 1000)) {
      std::cout << "Timed out waiting for a changed frame.\n";
      continue;
    }

    encoder.writeFrame(frame);
    ++encoded;

    if (options.intervalMs > 0) {
      std::this_thread::sleep_for(std::chrono::milliseconds(options.intervalMs));
    } else if (options.fps > 0) {
      std::this_thread::sleep_until(frameStartedAt + std::chrono::microseconds(1000000 / options.fps));
    }
  }

  encoder.finalize();
  std::cout << "Wrote " << options.outputFile.string() << "\n";
  return 0;
}

#ifdef _WIN32
void putPixelBgra(FrameBgra& frame, int x, int y, std::uint8_t r, std::uint8_t g, std::uint8_t b) {
  if (x < 0 || y < 0 || x >= static_cast<int>(frame.width) || y >= static_cast<int>(frame.height)) return;
  auto* pixel = frame.pixels.data()
    + static_cast<std::size_t>(y) * frame.stride
    + static_cast<std::size_t>(x) * 4;
  pixel[0] = b;
  pixel[1] = g;
  pixel[2] = r;
  pixel[3] = 255;
}

void drawSoftwareCursor(FrameBgra& frame, const DesktopDuplicator& duplicator) {
  CURSORINFO cursorInfo{};
  cursorInfo.cbSize = sizeof(cursorInfo);
  if (!GetCursorInfo(&cursorInfo) || (cursorInfo.flags & CURSOR_SHOWING) == 0) return;

  const int originX = cursorInfo.ptScreenPos.x - static_cast<int>(duplicator.left());
  const int originY = cursorInfo.ptScreenPos.y - static_cast<int>(duplicator.top());
  if (originX < -24 || originY < -24 ||
      originX >= static_cast<int>(frame.width) ||
      originY >= static_cast<int>(frame.height)) {
    return;
  }

  std::vector<std::pair<int, int>> white;
  for (int y = 0; y <= 16; ++y) {
    const int maxX = std::min(8, y / 2 + 1);
    for (int x = 0; x <= maxX; ++x) white.emplace_back(x, y);
  }
  for (int y = 11; y <= 22; ++y) {
    for (int x = 3; x <= 5; ++x) white.emplace_back(x, y);
  }

  for (const auto& [x, y] : white) {
    for (int dy = -1; dy <= 1; ++dy) {
      for (int dx = -1; dx <= 1; ++dx) {
        putPixelBgra(frame, originX + x + dx, originY + y + dy, 0, 0, 0);
      }
    }
  }
  for (const auto& [x, y] : white) {
    putPixelBgra(frame, originX + x, originY + y, 255, 255, 255);
  }
}
#endif

int runEncodedPipeMode(DesktopDuplicator& duplicator, const Options& options) {
  if (options.codec != VideoCodec::H264) {
    throw std::runtime_error("--encode-pipe currently supports h264 only.");
  }
  if (!options.tcpConnect.empty() && !options.packetFile.empty()) {
    throw std::runtime_error("Use either --tcp-connect or --packet-file, not both.");
  }

  std::unique_ptr<std::ofstream> packetFile;
  std::ostream* output = &std::cout;
#ifdef _WIN32
  std::unique_ptr<WinsockRuntime> winsock;
  std::unique_ptr<TcpClient> tcpClient;
#endif

  if (!options.tcpConnect.empty()) {
#ifdef _WIN32
    winsock = std::make_unique<WinsockRuntime>();
    tcpClient = std::make_unique<TcpClient>(options.tcpConnect);
    tcpClient->startControlReceiver();
#else
    throw std::runtime_error("--tcp-connect is currently implemented for Windows host builds.");
#endif
  } else if (!options.packetFile.empty()) {
    if (options.packetFile.has_parent_path()) {
      std::filesystem::create_directories(options.packetFile.parent_path());
    }
    packetFile = std::make_unique<std::ofstream>(options.packetFile, std::ios::binary);
    if (!packetFile->is_open()) {
      throw std::runtime_error("Could not open packet output file: " + options.packetFile.string());
    }
    output = packetFile.get();
  } else {
#ifdef _WIN32
    _setmode(_fileno(stdout), _O_BINARY);
#endif
  }

  VideoPacketEncodeOptions encodeOptions;
  encodeOptions.codec = VideoCodec::H264;
  encodeOptions.fps = options.fps;
  encodeOptions.bitrate = options.bitrate;
  encodeOptions.hardware = options.hardwareEncoder;

  MfVideoPacketEncoder encoder(duplicator.width(), duplicator.height(), encodeOptions);
  std::cerr << "SNV1 H.264 packet stream: "
            << duplicator.width() << "x" << duplicator.height()
            << " @ " << options.fps << " FPS, " << options.bitrate << " bps, "
            << (encoder.usingHardware() ? "hardware encoder" : "software encoder")
            << (!options.tcpConnect.empty()
                  ? " -> tcp " + options.tcpConnect
                  : (options.packetFile.empty() ? " -> stdout" : " -> " + options.packetFile.string()))
            << "\n";

  const auto frameDelay = options.fps > 0
    ? std::chrono::microseconds(1000000 / options.fps)
    : std::chrono::microseconds(0);
  const std::uint64_t targetFrames = (!options.framesProvided || options.frames == 0)
    ? std::numeric_limits<std::uint64_t>::max()
    : options.frames;

  std::uint64_t capturedFrames = 0;
  std::uint64_t sequence = 0;
  while (capturedFrames < targetFrames) {
    const auto frameStartedAt = std::chrono::steady_clock::now();
    FrameBgra frame;
    if (!duplicator.captureFrame(frame, 1000)) {
      std::cerr << "Timed out waiting for a changed frame.\n";
      continue;
    }
#ifdef _WIN32
    drawSoftwareCursor(frame, duplicator);
#endif

    auto packets = encoder.encodeFrame(frame);
    for (const auto& packet : packets) {
      const auto bytes = makeEncodedPacketBytes(packet,
                                                duplicator.width(),
                                                duplicator.height(),
                                                sequence++,
                                                encoder.usingHardware());
#ifdef _WIN32
      if (tcpClient) {
        tcpClient->sendAll(bytes.data(), bytes.size());
      } else
#endif
      {
        output->write(reinterpret_cast<const char*>(bytes.data()), bytes.size());
      }
    }
#ifdef _WIN32
    if (!tcpClient)
#endif
    {
      output->flush();
    }
    ++capturedFrames;

    if (options.intervalMs > 0) {
      std::this_thread::sleep_for(std::chrono::milliseconds(options.intervalMs));
    } else if (frameDelay.count() > 0) {
      std::this_thread::sleep_until(frameStartedAt + frameDelay);
    }
  }

  auto packets = encoder.finish();
  for (const auto& packet : packets) {
    const auto bytes = makeEncodedPacketBytes(packet,
                                              duplicator.width(),
                                              duplicator.height(),
                                              sequence++,
                                              encoder.usingHardware());
#ifdef _WIN32
    if (tcpClient) {
      tcpClient->sendAll(bytes.data(), bytes.size());
    } else
#endif
    {
      output->write(reinterpret_cast<const char*>(bytes.data()), bytes.size());
    }
  }
#ifdef _WIN32
  if (!tcpClient)
#endif
  {
    output->flush();
  }

  std::cerr << "SNV1 packet stream wrote " << sequence << " packet(s).\n";
  return 0;
}

int main(int argc, char** argv) {
  try {
    makeProcessDpiAware();
    const Options options = parseOptions(argc, argv);

    if (options.listEncoders) {
      listHardwareVideoEncoders(std::cout);
      return 0;
    }

    DesktopDuplicator duplicator;
    duplicator.initialize(options.adapterIndex, options.outputIndex);
    {
      std::lock_guard<std::mutex> lock(gInputBoundsMutex);
      gInputBounds = {
        static_cast<int>(duplicator.left()),
        static_cast<int>(duplicator.top()),
        static_cast<int>(duplicator.width()),
        static_cast<int>(duplicator.height())
      };
    }
    std::cerr << "SNINPUT target bounds left=" << gInputBounds.left
              << " top=" << gInputBounds.top
              << " width=" << gInputBounds.width
              << " height=" << gInputBounds.height
              << "\n";

    if (options.pipe) {
      return runPipeMode(duplicator, options);
    }

    if (options.encodePipe) {
      return runEncodedPipeMode(duplicator, options);
    }

    if (options.encode) {
      return runEncodeMode(duplicator, options);
    }

    std::filesystem::create_directories(options.outputDir);

    std::cout << "Desktop Duplication initialized: "
              << duplicator.width() << "x" << duplicator.height() << "\n";

    std::uint32_t saved = 0;
    while (saved < options.frames) {
      FrameBgra frame;
      if (!duplicator.captureFrame(frame, 1000)) {
        std::cout << "Timed out waiting for a changed frame.\n";
        continue;
      }

      auto file = options.outputDir / ("frame_" + std::to_string(saved) + ".bmp");
      writeBmp(file, frame);
      std::cout << "Wrote " << file.string() << "\n";
      ++saved;

      if (options.intervalMs > 0) {
        std::this_thread::sleep_for(std::chrono::milliseconds(options.intervalMs));
      }
    }

    return 0;
  } catch (const std::exception& error) {
    std::cerr << "sanser-native-host error: " << error.what() << "\n";
    return 1;
  }
}
