#include "mp4_recorder.h"
#ifdef SANSER_RECORDER_TEST_OPUS
#include "opus_audio.h"
#endif
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <map>

using namespace sanser::recording;
namespace fs = std::filesystem;

static int failures = 0;
#define CHECK(condition) do { if (!(condition)) { std::cerr << __FILE__ << ":" << __LINE__ << " CHECK failed: " #condition "\n"; ++failures; } } while (0)

struct Fixture { std::uint32_t width = 0, height = 0; std::vector<Bytes> units; };

static Fixture loadFixture(const fs::path& path) {
  std::ifstream input(path, std::ios::binary);
  Bytes data((std::istreambuf_iterator<char>(input)), {});
  Fixture fixture;
  auto be32 = [&](std::size_t at) { return (std::uint32_t(data[at]) << 24) | (std::uint32_t(data[at + 1]) << 16) | (std::uint32_t(data[at + 2]) << 8) | data[at + 3]; };
  if (data.size() < 16 || std::memcmp(data.data(), "SVC1", 4) != 0) return fixture;
  fixture.width = be32(4); fixture.height = be32(8);
  std::size_t offset = 16;
  while (offset + 5 <= data.size()) {
    const auto size = be32(offset);
    if (offset + 5 + size > data.size()) break;
    fixture.units.emplace_back(data.begin() + std::ptrdiff_t(offset + 5), data.begin() + std::ptrdiff_t(offset + 5 + size));
    offset += 5 + size;
  }
  return fixture;
}

// Minimal box walker: returns every box path (e.g. "moof/traf/trun") with payloads.
struct Box { std::string type; std::size_t offset = 0, size = 0; };
static std::vector<Box> topLevel(const Bytes& data) {
  std::vector<Box> boxes;
  std::size_t offset = 0;
  while (offset + 8 <= data.size()) {
    const std::uint32_t size = (std::uint32_t(data[offset]) << 24) | (std::uint32_t(data[offset + 1]) << 16) | (std::uint32_t(data[offset + 2]) << 8) | data[offset + 3];
    if (size < 8 || offset + size > data.size()) break;
    boxes.push_back({std::string(reinterpret_cast<const char*>(&data[offset + 4]), 4), offset, size});
    offset += size;
  }
  return boxes;
}

static Bytes readFile(const fs::path& path) {
  std::ifstream input(path, std::ios::binary);
  return Bytes((std::istreambuf_iterator<char>(input)), {});
}

struct TrunCounts { std::size_t video = 0, audio = 0, fragments = 0, videoSync = 0; bool firstVideoSync = false; };
static TrunCounts countSamples(const Bytes& data) {
  TrunCounts counts;
  auto be32 = [&](std::size_t at) { return (std::uint32_t(data[at]) << 24) | (std::uint32_t(data[at + 1]) << 16) | (std::uint32_t(data[at + 2]) << 8) | data[at + 3]; };
  bool sawFirst = false;
  for (const auto& box : topLevel(data)) {
    if (box.type != "moof") continue;
    ++counts.fragments;
    std::size_t offset = box.offset + 8;
    while (offset + 8 <= box.offset + box.size) {
      const auto size = be32(offset);
      if (std::memcmp(&data[offset + 4], "traf", 4) == 0) {
        std::uint32_t track = 0;
        std::size_t inner = offset + 8;
        while (inner + 8 <= offset + size) {
          const auto innerSize = be32(inner);
          if (std::memcmp(&data[inner + 4], "tfhd", 4) == 0) track = be32(inner + 12);
          if (std::memcmp(&data[inner + 4], "trun", 4) == 0) {
            const auto count = be32(inner + 12);
            if (track == 1) {
              counts.video += count;
              for (std::uint32_t index = 0; index < count; ++index) {
                const auto flags = be32(inner + 20 + index * 12 + 8);
                const bool sync = flags == 0x02000000;
                counts.videoSync += sync;
                if (!sawFirst) { counts.firstVideoSync = sync; sawFirst = true; }
              }
            } else {
              counts.audio += count;
            }
          }
          inner += innerSize;
        }
      }
      offset += size;
    }
  }
  return counts;
}

static bool ffmpegAvailable() {
#if defined(_WIN32)
  return std::system("ffmpeg -version >NUL 2>&1") == 0;
#else
  return std::system("ffmpeg -version >/dev/null 2>&1") == 0;
#endif
}

static bool ffmpegDecodesCleanly(const fs::path& path, const fs::path& log) {
  const auto command = "ffmpeg -nostdin -v error -i \"" + path.string() + "\" -f null - >\"" + log.string() + "\" 2>&1";
  if (std::system(command.c_str()) != 0) return false;
  return fs::file_size(log) == 0;
}

static std::function<std::optional<Bytes>(std::span<const std::int16_t>)> audioEncoder() {
#ifdef SANSER_RECORDER_TEST_OPUS
  auto encoder = std::make_shared<sanser::audio::Encoder>();
  return [encoder](std::span<const std::int16_t> pcm) -> std::optional<Bytes> {
    sanser::audio::Samples samples{};
    std::copy(pcm.begin(), pcm.end(), samples.begin());
    return encoder->encode(samples);
  };
#else
  // Valid Opus TOC for a 2.5 ms CELT frame of silence is enough for box tests.
  return [](std::span<const std::int16_t>) -> std::optional<Bytes> { return Bytes{0xFC, 0xFF, 0xFE}; };
#endif
}

// Feeds the fixture `repeat` times as one continuous 60 FPS stream.
struct Feeder {
  const Fixture& fixture;
  std::uint64_t sequence = 1, timestamp = 1'000'000, received = 5'000'000;
  VideoInput next(std::size_t index) {
    VideoInput input;
    input.accessUnit = fixture.units[index % fixture.units.size()];
    input.sequence = sequence++;
    input.timestampMicros = timestamp; timestamp += 16'667;
    input.receivedMicros = received; received += 16'667;
    input.width = fixture.width; input.height = fixture.height;
    return input;
  }
};

static void feedAudio(RecordingSession& session, std::uint64_t receivedMicros, double& phase) {
  std::vector<std::int16_t> block(800 * 2);  // 16.7 ms at 48 kHz
  for (std::size_t frame = 0; frame < 800; ++frame) {
    const auto value = std::int16_t(std::sin(phase) * 8000);
    phase += 2 * 3.14159265358979 * 440 / 48000;
    block[frame * 2] = block[frame * 2 + 1] = value;
  }
  session.audio(block, receivedMicros);
}

int main(int argc, char** argv) {
  fs::path fixturePath = argc > 1 ? fs::path(argv[1]) : fs::path(SANSER_FIXTURE_DIR) / "macos-h264-low-delay.svc";
  const auto fixture = loadFixture(fixturePath);
  if (fixture.units.size() != 45) { std::cerr << "fixture missing: " << fixturePath << "\n"; return 1; }
  const auto directory = fs::temp_directory_path() / ("sanser-recorder-test-" + std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()));
  std::error_code cleanup;
  fs::remove_all(directory, cleanup);
  fs::create_directories(directory);
  const bool ffmpeg = ffmpegAvailable();
  if (!ffmpeg) std::cerr << "ffmpeg not found; skipping decoder validation\n";

  CHECK(partPath("/a/b/rec.mp4", 1) == "/a/b/rec.mp4");
  CHECK(partPath("/a/b/rec.mp4", 2) == "/a/b/rec-part2.mp4");
  CHECK(partPath("C:\\x.y\\rec", 3) == "C:\\x.y\\rec-part3.mp4");
  CHECK(splitAnnexB(Bytes{0, 0, 0, 1, 0x67, 1, 0, 0, 1, 0x68, 2}).size() == 2);
  CHECK(splitAvcc(Bytes{0, 0, 0, 2, 0x65, 9, 0, 0, 0, 1, 0x41}).size() == 2);
  CHECK(splitAvcc(Bytes{0, 0, 0, 9, 0x65}).empty());

  // 1. Two GOPs with audio: valid structure, starts on IDR, decodes cleanly.
  {
    const auto path = directory / "basic.mp4";
    std::string started; StopReason reason = StopReason::None;
    RecorderOptions options; options.path = path.string(); options.audio = true; options.encodeAudio = audioEncoder();
    RecordingSession session(options, {[&](const std::string& value) { started = value; },
                                       [&](const std::string&, StopReason value, std::uint64_t, std::uint64_t) { reason = value; }});
    CHECK(session.needsKeyframe());
    Feeder feeder{fixture};
    double phase = 0;
    for (std::size_t index = 0; index < 90; ++index) {
      const auto input = feeder.next(index);
      session.video(input);
      feedAudio(session, input.receivedMicros, phase);
    }
    CHECK(!session.needsKeyframe());
    session.finish(StopReason::User);
    CHECK(reason == StopReason::User);
    CHECK(started == path.string());
    const auto data = readFile(path);
    const auto boxes = topLevel(data);
    CHECK(boxes.size() >= 4 && boxes[0].type == "ftyp" && boxes[1].type == "moov");
    const auto counts = countSamples(data);
    CHECK(counts.video == 90);
    CHECK(counts.videoSync == 2);
    CHECK(counts.firstVideoSync);
    CHECK(counts.audio > 250);  // ~1.5 s of 5 ms packets
    CHECK(counts.fragments >= 2);
    if (ffmpeg) CHECK(ffmpegDecodesCleanly(path, directory / "basic.log"));
  }

  // 2. Started mid-GOP and a lost picture: waits for IDR each time, no broken references.
  {
    const auto path = directory / "gap.mp4";
    RecorderOptions options; options.path = path.string();
    RecordingSession session(options, {});
    Feeder feeder{fixture};
    for (std::size_t index = 10; index < 45; ++index) session.video(feeder.next(index));  // mid-GOP start
    CHECK(session.needsKeyframe());
    CHECK(!fs::exists(path));
    for (std::size_t index = 0; index < 20; ++index) session.video(feeder.next(index));
    feeder.sequence += 1;  // one picture lost in transit
    for (std::size_t index = 21; index < 45; ++index) session.video(feeder.next(index));
    CHECK(session.needsKeyframe());
    for (std::size_t index = 0; index < 45; ++index) session.video(feeder.next(index));
    session.finish(StopReason::User);
    const auto counts = countSamples(readFile(path));
    CHECK(counts.video == 20 + 45);
    CHECK(counts.videoSync == 2);
    CHECK(session.gaps() == 1);
    if (ffmpeg) CHECK(ffmpegDecodesCleanly(path, directory / "gap.log"));
  }

  // 3. Stream reconfiguration (new resolution at an IDR) continues in part 2.
  {
    const auto path = directory / "parts.mp4";
    RecorderOptions options; options.path = path.string();
    RecordingSession session(options, {});
    Feeder feeder{fixture};
    for (std::size_t index = 0; index < 45; ++index) session.video(feeder.next(index));
    for (std::size_t index = 0; index < 45; ++index) { auto input = feeder.next(index); input.width = 1280; input.height = 720; session.video(input); }
    session.finish(StopReason::User);
    CHECK(session.part() == 2);
    CHECK(countSamples(readFile(path)).video == 45);
    CHECK(countSamples(readFile(partPath(path.string(), 2))).video == 45);
  }

  // 4. Size and duration limits stop cleanly with a playable file.
  {
    const auto path = directory / "limit.mp4";
    StopReason reason = StopReason::None; std::uint64_t bytes = 0;
    RecorderOptions options; options.path = path.string(); options.maxBytes = 40'000;
    RecordingSession session(options, {{}, [&](const std::string&, StopReason value, std::uint64_t written, std::uint64_t) { reason = value; bytes = written; }});
    Feeder feeder{fixture};
    for (std::size_t index = 0; index < 450 && session.active(); ++index) session.video(feeder.next(index));
    CHECK(reason == StopReason::SizeLimit);
    CHECK(bytes > 0 && bytes <= 40'000 + (1u << 16));
    CHECK(fs::file_size(path) == bytes);
    if (ffmpeg) CHECK(ffmpegDecodesCleanly(path, directory / "limit.log"));

    const auto timed = directory / "duration.mp4";
    RecorderOptions durationOptions; durationOptions.path = timed.string(); durationOptions.maxDurationMicros = 500'000;
    StopReason durationReason = StopReason::None;
    RecordingSession limited(durationOptions, {{}, [&](const std::string&, StopReason value, std::uint64_t, std::uint64_t) { durationReason = value; }});
    Feeder durationFeeder{fixture};
    for (std::size_t index = 0; index < 90 && limited.active(); ++index) limited.video(durationFeeder.next(index));
    CHECK(durationReason == StopReason::DurationLimit);
    CHECK(countSamples(readFile(timed)).video <= 32);
  }

  // 5. Unwritable path reports a write error instead of crashing.
  {
    StopReason reason = StopReason::None;
    RecorderOptions options; options.path = (directory / "missing-dir" / "x.mp4").string();
    RecordingSession session(options, {{}, [&](const std::string&, StopReason value, std::uint64_t, std::uint64_t) { reason = value; }});
    Feeder feeder{fixture};
    session.video(feeder.next(0));
    CHECK(reason == StopReason::WriteError);
    CHECK(!session.active());
  }

  // 6. Threaded front end: stop from another thread finalizes the file.
  {
    const auto path = directory / "threaded.mp4";
    Recorder recorder;
    std::atomic<int> stopped{0};
    RecorderOptions options; options.path = path.string(); options.audio = true; options.encodeAudio = audioEncoder();
    recorder.start(options, {{}, [&](const std::string&, StopReason reason, std::uint64_t, std::uint64_t) { if (reason == StopReason::User) ++stopped; }});
    CHECK(recorder.active());
    Feeder feeder{fixture};
    std::vector<std::int16_t> silence(800 * 2);
    for (std::size_t index = 0; index < 45; ++index) {
      auto input = feeder.next(index);
      const auto received = input.receivedMicros;
      recorder.video(std::move(input));
      recorder.audio(silence, received);
    }
    recorder.stop(StopReason::User);
    CHECK(!recorder.active());
    CHECK(stopped == 1);
    CHECK(countSamples(readFile(path)).video >= 1);
    if (ffmpeg) CHECK(ffmpegDecodesCleanly(path, directory / "threaded.log"));
  }

  std::error_code ignored;
  if (failures == 0) fs::remove_all(directory, ignored);
  else std::cerr << "outputs kept in " << directory << "\n";
  std::cout << (failures ? "FAILED" : "mp4 recorder tests passed") << "\n";
  return failures ? 1 : 0;
}
