#include "recording_tap.h"
#include <filesystem>
#include <fstream>
#include <iostream>

using namespace sanser::desktop;
namespace fs = std::filesystem;
static int failures = 0;
#define CHECK(condition) do { if (!(condition)) { std::cerr << __FILE__ << ":" << __LINE__ << " CHECK failed: " #condition "\n"; ++failures; } } while (0)

static std::vector<Bytes> fixtureUnits() {
  std::ifstream input(fs::path(SANSER_FIXTURE_DIR) / "macos-h264-low-delay.svc", std::ios::binary);
  Bytes data((std::istreambuf_iterator<char>(input)), {});
  std::vector<Bytes> units;
  std::size_t offset = 16;
  while (offset + 5 <= data.size()) {
    const std::uint32_t size = (std::uint32_t(data[offset]) << 24) | (std::uint32_t(data[offset + 1]) << 16) | (std::uint32_t(data[offset + 2]) << 8) | data[offset + 3];
    units.emplace_back(data.begin() + std::ptrdiff_t(offset + 5), data.begin() + std::ptrdiff_t(offset + 5 + size));
    offset += 5 + size;
  }
  return units;
}

int main() {
  bool malformed = false;
  CHECK(!parseRecordCommand("audio-muted\t1", malformed));
  auto stop = parseRecordCommand("record\tstop", malformed);
  CHECK(stop && stop->kind == RecordCommand::Stop && !malformed);
  auto start = parseRecordCommand("record\tstart\t50000000\t600\t/tmp/My Recordings/a.mp4", malformed);
  CHECK(start && start->kind == RecordCommand::Start && !malformed);
  CHECK(start->path == "/tmp/My Recordings/a.mp4" && start->maxBytes == 50'000'000 && start->maxSeconds == 600);
  auto clamped = parseRecordCommand("record\tstart\t1\t999999999\tC:\\x.mp4", malformed);
  CHECK(clamped && clamped->maxBytes == 1'000'000 && clamped->maxSeconds == 4 * 3600);
  for (const char* bad : {"record\tstart\tabc\t1\t/x", "record\tstart\t1\t1\t", "record\tstart\t1", "record\tpause"}) {
    parseRecordCommand(bad, malformed);
    CHECK(malformed);
  }

  const auto units = fixtureUnits();
  CHECK(units.size() == 45);
  const auto directory = fs::temp_directory_path() / ("sanser-tap-test-" + std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()));
  fs::create_directories(directory);

  std::vector<std::string> lines; std::mutex mutex;
  int keyframes = 0;
  {
    RecordingTap tap([&] { ++keyframes; }, [&](const std::string& line) { std::lock_guard lock(mutex); lines.push_back(line); });
    tap.announce();
    CHECK(!tap.command("voice-mute\t1"));
    CHECK(tap.command("record\tbogus"));
    AudioSamples silence{};
    tap.audio(silence);  // system audio is being received before Record
    const auto path = (directory / "tap.mp4").string();
    CHECK(tap.command("record\tstart\t100000000\t60\t" + path));
    CHECK(tap.active());
    CHECK(keyframes == 1);  // an IDR is requested immediately
    for (std::uint64_t index = 0; index < 45; ++index) {
      Frame frame; frame.data = units[index]; frame.id = index + 1; frame.width = 1846; frame.height = 1200; frame.keyframe = index == 0;
      tap.frame(frame);
      tap.audio(silence);
    }
    CHECK(tap.command("record\tstop"));
    CHECK(!tap.active());
    CHECK(fs::exists(path) && fs::file_size(path) > 10'000);

    // HEVC parameter sets are rejected with a clear error.
    CHECK(tap.command("record\tstart\t100000000\t60\t" + (directory / "hevc.mp4").string()));
    Frame hevc; hevc.data = {0, 0, 0, 1, 0x40, 0x01, 0x0C, 0, 0, 0, 1, 0x42, 0x01, 0x01}; hevc.id = 1; hevc.width = 64; hevc.height = 64;
    tap.frame(hevc);
    CHECK(!tap.active());
  }
  std::lock_guard lock(mutex);
  auto has = [&](const std::string& prefix) { for (const auto& line : lines) if (line.rfind(prefix, 0) == 0) return true; return false; };
  CHECK(has("SNUSER_RECORD\tcapable"));
  CHECK(has("SNUSER_RECORD\terror\tbad-command"));
  CHECK(has("SNUSER_RECORD\twaiting"));
  CHECK(has("SNUSER_RECORD\trecording\t1"));  // audio had been received before start
  CHECK(has("SNUSER_RECORD\tstopped\tuser\t"));
  CHECK(has("SNUSER_RECORD\terror\tunsupported-codec"));
  for (const auto& line : lines) CHECK(line.find(directory.string()) == std::string::npos);  // never echo paths
  std::error_code ignored;
  if (!failures) fs::remove_all(directory, ignored);
  std::cout << (failures ? "FAILED" : "recording tap tests passed") << "\n";
  return failures ? 1 : 0;
}
