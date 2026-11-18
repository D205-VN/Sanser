#pragma once
// Viewer recording bridge shared by the macOS and Windows SNV2 viewers.
//
// Commands (stdin, one line, path last so it may contain spaces):
//   record\tstart\t<maxBytes>\t<maxSeconds>\t<utf8 path>
//   record\tstop
// Reports (stdout, SNUSER_ prefix keeps them out of logs and diagnostics):
//   SNUSER_RECORD\tcapable
//   SNUSER_RECORD\twaiting             waiting for an IDR (one was requested)
//   SNUSER_RECORD\trecording\t<audio 0|1>
//   SNUSER_RECORD\tstopped\t<reason>\t<bytes>\t<durationMs>\t<parts>
//   SNUSER_RECORD\terror\t<code>
// The file path is never echoed back, so stdout cannot leak the user's home.
#include "audio.h"
#include "mp4_recorder.h"
#include "opus_audio.h"
#include "transport.h"
#include <chrono>
#include <cstdio>
#include <functional>
#include <memory>
#include <mutex>
#include <string>

namespace sanser::desktop {

struct RecordCommand {
  enum Kind { Start, Stop } kind = Stop;
  std::uint64_t maxBytes = recording::kDefaultMaxBytes;
  std::uint64_t maxSeconds = recording::kDefaultMaxDurationMicros / 1'000'000;
  std::string path;
};

// Returns nullopt for lines that are not recording commands; a malformed
// recording command yields a Stop with an empty path and sets `malformed`.
inline std::optional<RecordCommand> parseRecordCommand(const std::string& line, bool& malformed) {
  malformed = false;
  if (line.rfind("record\t", 0) != 0) return std::nullopt;
  const auto rest = line.substr(7);
  if (rest == "stop") return RecordCommand{};
  if (rest.rfind("start\t", 0) != 0) { malformed = true; return RecordCommand{}; }
  std::size_t first = 6, second = rest.find('\t', first);
  const auto third = second == std::string::npos ? std::string::npos : rest.find('\t', second + 1);
  if (second == std::string::npos || third == std::string::npos) { malformed = true; return RecordCommand{}; }
  RecordCommand command;
  command.kind = RecordCommand::Start;
  try {
    command.maxBytes = std::stoull(rest.substr(first, second - first));
    command.maxSeconds = std::stoull(rest.substr(second + 1, third - second - 1));
  } catch (...) { malformed = true; return RecordCommand{}; }
  command.path = rest.substr(third + 1);
  const bool badPath = command.path.empty() || command.path.size() > 4096 ||
                       command.path.find_first_of(std::string("\t\r\n\0", 4)) != std::string::npos;
  if (badPath) { malformed = true; return RecordCommand{}; }
  command.maxBytes = std::clamp<std::uint64_t>(command.maxBytes, 1'000'000, recording::kDefaultMaxBytes);
  command.maxSeconds = std::clamp<std::uint64_t>(command.maxSeconds, 5, 4 * 3600);
  return command;
}

class RecordingTap {
public:
  using Emit = std::function<void(const std::string&)>;
  explicit RecordingTap(std::function<void()> requestKeyframe, Emit emit = {})
    : requestKeyframe_(std::move(requestKeyframe)), emit_(std::move(emit)) {
    if (!emit_) emit_ = [](const std::string& line) { std::fprintf(stdout, "%s\n", line.c_str()); std::fflush(stdout); };
  }
  ~RecordingTap() { recorder_.stop(recording::StopReason::SessionEnded); }
  RecordingTap(const RecordingTap&) = delete;
  RecordingTap& operator=(const RecordingTap&) = delete;

  void announce() { emit_("SNUSER_RECORD\tcapable"); }

  // True when the line was a recording command (handled or rejected).
  bool command(const std::string& line) {
    bool malformed = false;
    const auto parsed = parseRecordCommand(line, malformed);
    if (!parsed) return false;
    if (malformed) { emit_("SNUSER_RECORD\terror\tbad-command"); return true; }
    if (parsed->kind == RecordCommand::Stop) { recorder_.stop(recording::StopReason::User); return true; }
    start(*parsed);
    return true;
  }

  // Called on the receive thread before the frame is moved to the decoder.
  void frame(const Frame& frame) {
    if (!recorder_.active()) return;
    if (looksLikeHevc(frame.data)) {
      emit_("SNUSER_RECORD\terror\tunsupported-codec");
      recorder_.stop(recording::StopReason::Unsupported);
      return;
    }
    recording::VideoInput input;
    input.accessUnit = frame.data;
    input.sequence = frame.id;
    input.timestampMicros = input.receivedMicros = nowMicros();
    input.width = frame.width;
    input.height = frame.height;
    recorder_.video(std::move(input));
    maybeRequestKeyframe();
  }

  void audio(const AudioSamples& samples) {
    lastAudioMicros_.store(nowMicros());
    if (recorder_.active()) recorder_.audio(samples, nowMicros());
  }

  bool active() const { return recorder_.active(); }

private:
  recording::Recorder recorder_;
  std::function<void()> requestKeyframe_;
  Emit emit_;
  std::atomic<std::uint64_t> lastAudioMicros_{0};
  std::chrono::steady_clock::time_point lastKeyframeRequest_{};

  static std::uint64_t nowMicros() {
    return std::uint64_t(std::chrono::duration_cast<std::chrono::microseconds>(
      std::chrono::steady_clock::now().time_since_epoch()).count());
  }

  // HEVC parameter sets begin 0x40 0x01 (VPS), 0x42 0x01 (SPS), 0x44 0x01 (PPS).
  static bool looksLikeHevc(const Bytes& data) {
    for (const auto nal : recording::splitAccessUnit(data)) {
      if (nal.size() >= 2 && nal[1] == 0x01 && (nal[0] == 0x40 || nal[0] == 0x42 || nal[0] == 0x44)) return true;
    }
    return false;
  }

  void maybeRequestKeyframe() {
    if (!recorder_.needsKeyframe() || !requestKeyframe_) return;
    const auto now = std::chrono::steady_clock::now();
    if (now - lastKeyframeRequest_ < std::chrono::seconds(1)) return;
    lastKeyframeRequest_ = now;
    requestKeyframe_();
  }

  void start(const RecordCommand& command) {
    recording::RecorderOptions options;
    options.path = command.path;
    options.maxBytes = command.maxBytes;
    options.maxDurationMicros = command.maxSeconds * 1'000'000;
    const auto lastAudio = lastAudioMicros_.load();
    options.audio = lastAudio && nowMicros() - lastAudio < 3'000'000;
    if (options.audio) {
      try {
        auto encoder = std::make_shared<sanser::audio::Encoder>();
        options.encodeAudio = [encoder](std::span<const std::int16_t> pcm) -> std::optional<Bytes> {
          sanser::audio::Samples samples{};
          if (pcm.size() != samples.size()) return std::nullopt;
          std::copy(pcm.begin(), pcm.end(), samples.begin());
          try { return encoder->encode(samples); } catch (...) { return std::nullopt; }
        };
      } catch (...) { options.audio = false; }
    }
    const bool withAudio = options.audio;
    auto emit = emit_;
    recording::RecorderEvents events;
    events.started = [emit, withAudio](const std::string&) { emit(std::string("SNUSER_RECORD\trecording\t") + (withAudio ? "1" : "0")); };
    events.stopped = [emit](const std::string& path, recording::StopReason reason, std::uint64_t bytes, std::uint64_t micros) {
      // Count parts from the final path's "-partN" suffix without echoing it.
      unsigned parts = 1;
      if (const auto at = path.rfind("-part"); at != std::string::npos) {
        try { parts = unsigned(std::stoul(path.substr(at + 5))); } catch (...) {}
      }
      emit("SNUSER_RECORD\tstopped\t" + std::string(recording::stopReasonText(reason)) + "\t" + std::to_string(bytes) + "\t" +
           std::to_string(micros / 1000) + "\t" + std::to_string(parts));
    };
    recorder_.start(std::move(options), std::move(events));
    emit_("SNUSER_RECORD\twaiting");
    lastKeyframeRequest_ = {};
    maybeRequestKeyframe();
    if (requestKeyframe_ && lastKeyframeRequest_ == std::chrono::steady_clock::time_point{}) {
      lastKeyframeRequest_ = std::chrono::steady_clock::now();
      requestKeyframe_();
    }
  }
};

}  // namespace sanser::desktop
