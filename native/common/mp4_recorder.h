#pragma once
// Viewer-side session recording: writes the H.264 access units the viewer
// already received into a fragmented MP4 without re-encoding. Optional system
// audio arrives as 48 kHz stereo PCM (what both viewers already decode or
// receive) and is stored as an Opus track encoded by a caller-supplied
// function, so this header stays free of codec and platform dependencies.
//
// Robustness rules:
//  * A file never starts with a non-IDR picture: the recorder waits for an IDR
//    with SPS/PPS and reports needsKeyframe() so the caller can request one.
//  * A lost/skipped access unit (sequence gap, decoder recovery, queue
//    overflow) drops pictures until the next IDR, so no sample in the file
//    references a picture that is not in the file. The gap shows as a freeze.
//  * New SPS/PPS (resolution change, encoder reset) close the current file and
//    continue in "<name>-part2.mp4"; one sample description per file keeps the
//    output playable by every common player.
//  * Fragmented layout (moof/mdat every <=2 s): a crash or full disk leaves a
//    playable file up to the last complete fragment.
//  * Bounded by bytes and duration; all disk I/O runs on a worker thread with
//    a bounded queue so the decode/render thread never blocks on the disk.
#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <deque>
#include <filesystem>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <span>
#include <string>
#include <thread>
#include <utility>
#include <vector>

namespace sanser::recording {

using Bytes = std::vector<std::uint8_t>;

inline constexpr std::uint32_t kVideoTimescale = 90000;
inline constexpr std::uint32_t kAudioRate = 48000;
inline constexpr std::uint32_t kAudioFrames = 240;  // 5 ms Opus packets
inline constexpr std::uint64_t kDefaultMaxBytes = 3'800'000'000ULL;  // FAT32-safe
inline constexpr std::uint64_t kDefaultMaxDurationMicros = 2ULL * 3600 * 1'000'000;

enum class StopReason { None, User, SizeLimit, DurationLimit, WriteError, Unsupported, SessionEnded };

inline const char* stopReasonText(StopReason reason) {
  switch (reason) {
    case StopReason::None: return "none";
    case StopReason::User: return "user";
    case StopReason::SizeLimit: return "size-limit";
    case StopReason::DurationLimit: return "duration-limit";
    case StopReason::WriteError: return "write-error";
    case StopReason::Unsupported: return "unsupported";
    case StopReason::SessionEnded: return "session-ended";
  }
  return "none";
}

// ---------------------------------------------------------------------------
// Annex-B / AVCC helpers
// ---------------------------------------------------------------------------
using Nal = std::span<const std::uint8_t>;

inline std::vector<Nal> splitAnnexB(std::span<const std::uint8_t> data) {
  std::vector<Nal> nals;
  std::size_t index = 0, start = std::string::npos;
  const auto size = data.size();
  while (index + 2 < size) {
    if (data[index] == 0 && data[index + 1] == 0 && data[index + 2] == 1) {
      if (start != std::string::npos) {
        auto end = index;
        while (end > start && data[end - 1] == 0) --end;  // trailing zero of a 4-byte code
        if (end > start) nals.push_back(data.subspan(start, end - start));
      }
      index += 3;
      start = index;
    } else {
      ++index;
    }
  }
  if (start != std::string::npos && start < size) nals.push_back(data.subspan(start));
  return nals;
}

inline std::vector<Nal> splitAvcc(std::span<const std::uint8_t> data) {
  std::vector<Nal> nals;
  std::size_t offset = 0;
  while (offset + 4 <= data.size()) {
    const std::uint32_t length = (std::uint32_t(data[offset]) << 24) | (std::uint32_t(data[offset + 1]) << 16) |
                                 (std::uint32_t(data[offset + 2]) << 8) | data[offset + 3];
    offset += 4;
    if (length == 0 || length > data.size() - offset) return {};
    nals.push_back(data.subspan(offset, length));
    offset += length;
  }
  if (offset != data.size()) return {};
  return nals;
}

// Accepts either framing; Annex-B is tried first like both viewers do.
inline std::vector<Nal> splitAccessUnit(std::span<const std::uint8_t> data) {
  auto nals = splitAnnexB(data);
  if (!nals.empty()) return nals;
  return splitAvcc(data);
}

// ---------------------------------------------------------------------------
// Box writer
// ---------------------------------------------------------------------------
class BoxWriter {
public:
  Bytes bytes;
  void u8(std::uint8_t value) { bytes.push_back(value); }
  void u16(std::uint16_t value) { u8(std::uint8_t(value >> 8)); u8(std::uint8_t(value)); }
  void u24(std::uint32_t value) { u8(std::uint8_t(value >> 16)); u16(std::uint16_t(value)); }
  void u32(std::uint32_t value) { u16(std::uint16_t(value >> 16)); u16(std::uint16_t(value)); }
  void u64(std::uint64_t value) { u32(std::uint32_t(value >> 32)); u32(std::uint32_t(value)); }
  void fourcc(const char* code) { bytes.insert(bytes.end(), code, code + 4); }
  void raw(std::span<const std::uint8_t> data) { bytes.insert(bytes.end(), data.begin(), data.end()); }
  void zeros(std::size_t count) { bytes.insert(bytes.end(), count, 0); }
  std::size_t begin(const char* type) { const auto at = bytes.size(); u32(0); fourcc(type); return at; }
  std::size_t full(const char* type, std::uint8_t version, std::uint32_t flags) {
    const auto at = begin(type); u8(version); u24(flags); return at;
  }
  void end(std::size_t at) { patch32(at, std::uint32_t(bytes.size() - at)); }
  void patch32(std::size_t at, std::uint32_t value) {
    bytes[at] = std::uint8_t(value >> 24); bytes[at + 1] = std::uint8_t(value >> 16);
    bytes[at + 2] = std::uint8_t(value >> 8); bytes[at + 3] = std::uint8_t(value);
  }
  void matrix() {
    const std::uint32_t values[9] = {0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000};
    for (auto value : values) u32(value);
  }
};

struct VideoConfig {
  Bytes sps, pps;
  std::uint32_t width = 0, height = 0;
  bool operator==(const VideoConfig&) const = default;
};

struct Sample {
  Bytes data;
  std::uint32_t duration = 0;
  bool sync = true;
};

// ---------------------------------------------------------------------------
// Fragmented MP4 file. Single-threaded; used by the Recorder worker.
// ---------------------------------------------------------------------------
class FragmentedMp4 {
public:
  FragmentedMp4() = default;
  FragmentedMp4(const FragmentedMp4&) = delete;
  FragmentedMp4& operator=(const FragmentedMp4&) = delete;
  ~FragmentedMp4() { close(); }

  bool open(const std::string& path, const VideoConfig& video, bool audio, std::uint16_t audioPreSkip) {
    close();
#if defined(_WIN32)
    // Paths come from the desktop shell as UTF-8.
    const std::filesystem::path native(std::u8string(reinterpret_cast<const char8_t*>(path.c_str())));
    if (_wfopen_s(&file_, native.c_str(), L"wb") != 0) file_ = nullptr;
#else
    file_ = std::fopen(path.c_str(), "wb");
#endif
    if (!file_) return false;
    audio_ = audio;
    sequence_ = 0;
    written_ = 0;
    videoDecodeTime_ = audioDecodeTime_ = 0;
    BoxWriter out;
    auto ftyp = out.begin("ftyp");
    out.fourcc("isom"); out.u32(0x200);
    out.fourcc("isom"); out.fourcc("iso6"); out.fourcc("avc1"); out.fourcc("mp41");
    out.end(ftyp);
    writeMoov(out, video, audioPreSkip);
    return write(out.bytes);
  }

  // Video duration is in 90 kHz ticks, audio in 48 kHz samples. Samples are
  // contiguous per track; audioGap starts the audio run at a later time.
  bool fragment(std::vector<Sample>& video, std::vector<Sample>& audio, std::uint64_t audioStart) {
    if (!file_) return false;
    if (video.empty() && audio.empty()) return true;
    if (!audio_) audio.clear();
    if (!audio.empty() && audioStart > audioDecodeTime_) audioDecodeTime_ = audioStart;
    BoxWriter out;
    const auto moof = out.begin("moof");
    const auto mfhd = out.full("mfhd", 0, 0); out.u32(++sequence_); out.end(mfhd);
    std::size_t videoOffsetAt = 0, audioOffsetAt = 0;
    if (!video.empty()) videoOffsetAt = traf(out, 1, videoDecodeTime_, video);
    if (!audio.empty()) audioOffsetAt = traf(out, 2, audioDecodeTime_, audio);
    out.end(moof);
    std::uint64_t payload = 0;
    for (const auto& sample : video) payload += sample.data.size();
    for (const auto& sample : audio) payload += sample.data.size();
    const auto mdatHeader = out.bytes.size();
    out.u32(std::uint32_t(8 + payload)); out.fourcc("mdat");
    const auto dataStart = std::uint32_t(mdatHeader + 8);  // relative to moof start (0)
    std::uint64_t videoBytes = 0;
    for (const auto& sample : video) videoBytes += sample.data.size();
    if (videoOffsetAt) out.patch32(videoOffsetAt, dataStart);
    if (audioOffsetAt) out.patch32(audioOffsetAt, std::uint32_t(dataStart + videoBytes));
    if (!write(out.bytes)) return false;
    for (const auto& sample : video) { if (!write(sample.data)) return false; videoDecodeTime_ += sample.duration; }
    for (const auto& sample : audio) { if (!write(sample.data)) return false; audioDecodeTime_ += sample.duration; }
    std::fflush(file_);
    video.clear(); audio.clear();
    return true;
  }

  // Patches mvhd/mehd so players that read the header show the duration.
  void close() {
    if (!file_) return;
    const std::uint64_t duration = videoDecodeTime_ * 1000 / kVideoTimescale;
    if (mvhdDurationAt_ && std::fseek(file_, long(mvhdDurationAt_), SEEK_SET) == 0) writeU64(duration);
    if (mehdDurationAt_ && std::fseek(file_, long(mehdDurationAt_), SEEK_SET) == 0) writeU64(duration);
    std::fclose(file_);
    file_ = nullptr;
  }

  bool isOpen() const { return file_ != nullptr; }
  std::uint64_t bytesWritten() const { return written_; }
  std::uint64_t videoTicks() const { return videoDecodeTime_; }
  std::uint64_t audioSamples() const { return audioDecodeTime_; }

private:
  std::FILE* file_ = nullptr;
  bool audio_ = false;
  std::uint32_t sequence_ = 0;
  std::uint64_t written_ = 0, videoDecodeTime_ = 0, audioDecodeTime_ = 0;
  std::size_t mvhdDurationAt_ = 0, mehdDurationAt_ = 0;

  bool write(std::span<const std::uint8_t> data) {
    if (data.empty()) return true;
    if (std::fwrite(data.data(), 1, data.size(), file_) != data.size()) return false;
    written_ += data.size();
    return true;
  }
  void writeU64(std::uint64_t value) {
    std::uint8_t bytes[8];
    for (int index = 0; index < 8; ++index) bytes[index] = std::uint8_t(value >> (56 - index * 8));
    std::fwrite(bytes, 1, 8, file_);
  }

  static std::size_t traf(BoxWriter& out, std::uint32_t track, std::uint64_t decodeTime, const std::vector<Sample>& samples) {
    const auto traf = out.begin("traf");
    const auto tfhd = out.full("tfhd", 0, 0x020000); out.u32(track); out.end(tfhd);  // default-base-is-moof
    const auto tfdt = out.full("tfdt", 1, 0); out.u64(decodeTime); out.end(tfdt);
    const auto trun = out.full("trun", 0, 0x000001 | 0x000100 | 0x000200 | 0x000400);
    out.u32(std::uint32_t(samples.size()));
    const auto offsetAt = out.bytes.size();
    out.u32(0);
    for (const auto& sample : samples) {
      out.u32(sample.duration);
      out.u32(std::uint32_t(sample.data.size()));
      out.u32(sample.sync ? 0x02000000 : 0x01010000);
    }
    out.end(trun);
    out.end(traf);
    return offsetAt;
  }

  void writeMoov(BoxWriter& out, const VideoConfig& video, std::uint16_t preSkip) {
    const auto moov = out.begin("moov");
    const auto mvhd = out.full("mvhd", 1, 0);
    out.u64(0); out.u64(0); out.u32(1000);
    mvhdDurationAt_ = out.bytes.size(); out.u64(0);
    out.u32(0x00010000); out.u16(0x0100); out.zeros(10); out.matrix(); out.zeros(24);
    out.u32(audio_ ? 3 : 2);
    out.end(mvhd);
    videoTrack(out, video);
    if (audio_) audioTrack(out, preSkip);
    const auto mvex = out.begin("mvex");
    const auto mehd = out.full("mehd", 1, 0); mehdDurationAt_ = out.bytes.size(); out.u64(0); out.end(mehd);
    trex(out, 1);
    if (audio_) trex(out, 2);
    out.end(mvex);
    out.end(moov);
  }

  static void trex(BoxWriter& out, std::uint32_t track) {
    const auto trex = out.full("trex", 0, 0);
    out.u32(track); out.u32(1); out.u32(0); out.u32(0); out.u32(0);
    out.end(trex);
  }

  static void tkhd(BoxWriter& out, std::uint32_t track, bool audio, std::uint32_t width, std::uint32_t height) {
    const auto tkhd = out.full("tkhd", 1, 0x3);  // enabled | in movie
    out.u64(0); out.u64(0); out.u32(track); out.u32(0); out.u64(0);
    out.zeros(8); out.u16(0); out.u16(audio ? 1 : 0); out.u16(audio ? 0x0100 : 0); out.u16(0);
    out.matrix(); out.u32(width << 16); out.u32(height << 16);
    out.end(tkhd);
  }

  static void mdhd(BoxWriter& out, std::uint32_t timescale) {
    const auto mdhd = out.full("mdhd", 1, 0);
    out.u64(0); out.u64(0); out.u32(timescale); out.u64(0); out.u16(0x55C4); out.u16(0);  // "und"
    out.end(mdhd);
  }

  static void hdlr(BoxWriter& out, const char* type, const char* name) {
    const auto hdlr = out.full("hdlr", 0, 0);
    out.u32(0); out.fourcc(type); out.zeros(12);
    out.raw({reinterpret_cast<const std::uint8_t*>(name), std::strlen(name) + 1});
    out.end(hdlr);
  }

  static void dinfAndEmptyTables(BoxWriter& out) {
    const auto dinf = out.begin("dinf");
    const auto dref = out.full("dref", 0, 0); out.u32(1);
    const auto url = out.full("url ", 0, 1); out.end(url);
    out.end(dref); out.end(dinf);
  }

  static void emptySampleTables(BoxWriter& out) {
    for (const char* type : {"stts", "stsc", "stco"}) { const auto box = out.full(type, 0, 0); out.u32(0); out.end(box); }
    const auto stsz = out.full("stsz", 0, 0); out.u32(0); out.u32(0); out.end(stsz);
  }

  static void videoTrack(BoxWriter& out, const VideoConfig& video) {
    const auto trak = out.begin("trak");
    tkhd(out, 1, false, video.width, video.height);
    const auto mdia = out.begin("mdia");
    mdhd(out, kVideoTimescale);
    hdlr(out, "vide", "Sanser video");
    const auto minf = out.begin("minf");
    const auto vmhd = out.full("vmhd", 0, 1); out.zeros(8); out.end(vmhd);
    dinfAndEmptyTables(out);
    const auto stbl = out.begin("stbl");
    const auto stsd = out.full("stsd", 0, 0); out.u32(1);
    const auto avc1 = out.begin("avc1");
    out.zeros(6); out.u16(1); out.zeros(16);
    out.u16(std::uint16_t(video.width)); out.u16(std::uint16_t(video.height));
    out.u32(0x00480000); out.u32(0x00480000); out.u32(0); out.u16(1);
    out.zeros(32); out.u16(0x0018); out.u16(0xFFFF);
    const auto avcC = out.begin("avcC");
    out.u8(1);
    out.u8(video.sps.size() > 1 ? video.sps[1] : 66);
    out.u8(video.sps.size() > 2 ? video.sps[2] : 0);
    out.u8(video.sps.size() > 3 ? video.sps[3] : 31);
    out.u8(0xFF);  // 4-byte NAL lengths
    out.u8(0xE1); out.u16(std::uint16_t(video.sps.size())); out.raw(video.sps);
    out.u8(1); out.u16(std::uint16_t(video.pps.size())); out.raw(video.pps);
    out.end(avcC);
    out.end(avc1);
    out.end(stsd);
    emptySampleTables(out);
    out.end(stbl); out.end(minf); out.end(mdia); out.end(trak);
  }

  static void audioTrack(BoxWriter& out, std::uint16_t preSkip) {
    const auto trak = out.begin("trak");
    tkhd(out, 2, true, 0, 0);
    const auto mdia = out.begin("mdia");
    mdhd(out, kAudioRate);
    hdlr(out, "soun", "Sanser system audio");
    const auto minf = out.begin("minf");
    const auto smhd = out.full("smhd", 0, 0); out.u32(0); out.end(smhd);
    dinfAndEmptyTables(out);
    const auto stbl = out.begin("stbl");
    const auto stsd = out.full("stsd", 0, 0); out.u32(1);
    const auto opus = out.begin("Opus");
    out.zeros(6); out.u16(1); out.zeros(8);
    out.u16(2); out.u16(16); out.u32(0); out.u32(kAudioRate << 16);
    const auto dops = out.begin("dOps");  // fields are big-endian, unlike OggOpus
    out.u8(0); out.u8(2); out.u16(preSkip); out.u32(kAudioRate); out.u16(0); out.u8(0);
    out.end(dops);
    out.end(opus);
    out.end(stsd);
    emptySampleTables(out);
    out.end(stbl); out.end(minf); out.end(mdia); out.end(trak);
  }
};

// ---------------------------------------------------------------------------
// Recording policy (pure, synchronous; unit-tested without threads).
// ---------------------------------------------------------------------------
struct RecorderOptions {
  std::string path;                       // first file; parts get "-partN"
  std::uint64_t maxBytes = kDefaultMaxBytes;
  std::uint64_t maxDurationMicros = kDefaultMaxDurationMicros;
  bool audio = false;
  std::uint16_t audioPreSkip = 120;
  // Encodes exactly kAudioFrames interleaved stereo int16 samples.
  std::function<std::optional<Bytes>(std::span<const std::int16_t>)> encodeAudio;
};

struct VideoInput {
  Bytes accessUnit;                 // Annex-B or AVCC H.264
  std::uint64_t sequence = 0;       // consecutive per received picture
  std::uint64_t timestampMicros = 0;  // sender presentation clock
  std::uint64_t receivedMicros = 0;   // local steady clock
  std::uint32_t width = 0, height = 0;
  bool discontinuity = false;       // caller knows a picture was lost
};

struct RecorderEvents {
  std::function<void(const std::string& path)> started;  // first IDR written
  std::function<void(const std::string& path, StopReason, std::uint64_t bytes, std::uint64_t durationMicros)> stopped;
};

inline std::string partPath(const std::string& base, unsigned part) {
  if (part <= 1) return base;
  const auto dot = base.find_last_of('.');
  const auto slash = base.find_last_of("/\\");
  const bool hasExt = dot != std::string::npos && (slash == std::string::npos || dot > slash);
  const auto stem = hasExt ? base.substr(0, dot) : base;
  const auto ext = hasExt ? base.substr(dot) : std::string(".mp4");
  return stem + "-part" + std::to_string(part) + ext;
}

class RecordingSession {
public:
  RecordingSession(RecorderOptions options, RecorderEvents events)
    : options_(std::move(options)), events_(std::move(events)) {
    if (options_.audio && !options_.encodeAudio) options_.audio = false;
  }
  ~RecordingSession() { finish(StopReason::SessionEnded); }

  bool active() const { return !stopped_; }
  bool waitingForKeyframe() const { return waitingForIdr_; }
  bool needsKeyframe() const { return !stopped_ && waitingForIdr_; }
  std::uint64_t totalBytes() const { return completedBytes_ + file_.bytesWritten(); }
  unsigned part() const { return part_; }
  StopReason reason() const { return reason_; }

  void video(const VideoInput& input) {
    if (stopped_) return;
    const auto nals = splitAccessUnit(input.accessUnit);
    if (nals.empty()) return;
    const bool gap = input.discontinuity || (haveSequence_ && input.sequence != lastSequence_ + 1);
    lastSequence_ = input.sequence;
    haveSequence_ = true;
    bool idr = false;
    for (const auto nal : nals) {
      const auto type = nal[0] & 0x1F;
      if (type == 7) pendingConfig_.sps.assign(nal.begin(), nal.end());
      else if (type == 8) pendingConfig_.pps.assign(nal.begin(), nal.end());
      else if (type == 5) idr = true;
    }
    if (input.width && input.height) { pendingConfig_.width = input.width; pendingConfig_.height = input.height; }
    if (gap && !waitingForIdr_) {
      // Keep everything decodable: drop until a fresh IDR replaces references.
      waitingForIdr_ = true;
      ++gaps_;
    }
    if (waitingForIdr_ && !idr) return;
    if (idr && (pendingConfig_.sps.empty() || pendingConfig_.pps.empty())) return;
    if (idr && file_.isOpen() && !(pendingConfig_ == config_)) {
      if (pendingVideo_) {
        pendingVideo_->duration = ticksBetween(lastTimestamp_, input.timestampMicros);
        videoSamples_.push_back(std::move(*pendingVideo_));
        pendingVideo_.reset();
      }
      flush();
      completedBytes_ += file_.bytesWritten();
      completedMicros_ += mediaMicros();
      file_.close();
      ++part_;
    }
    if (!file_.isOpen()) {
      if (!openFile(input)) return;
    }
    waitingForIdr_ = false;
    Sample sample;
    sample.sync = idr;
    for (const auto nal : nals) {
      const auto type = nal[0] & 0x1F;
      if (type == 7 || type == 8 || type == 9) continue;  // parameter sets live in avcC
      const auto size = std::uint32_t(nal.size());
      sample.data.push_back(std::uint8_t(size >> 24)); sample.data.push_back(std::uint8_t(size >> 16));
      sample.data.push_back(std::uint8_t(size >> 8)); sample.data.push_back(std::uint8_t(size));
      sample.data.insert(sample.data.end(), nal.begin(), nal.end());
    }
    if (sample.data.empty()) return;
    // Duration of the previous picture is known now.
    if (pendingVideo_) {
      pendingVideo_->duration = ticksBetween(lastTimestamp_, input.timestampMicros);
      if (sample.sync && !videoSamples_.empty()) {
        videoSamples_.push_back(std::move(*pendingVideo_));
        pendingVideo_.reset();
        flush();
      } else {
        videoSamples_.push_back(std::move(*pendingVideo_));
      }
    }
    lastTimestamp_ = input.timestampMicros;
    pendingVideo_ = std::move(sample);
    if (bufferedTicks() >= 2 * kVideoTimescale || bufferedBytes() >= 8u << 20) flush();
    checkLimits();
  }

  // Interleaved stereo int16 at 48 kHz; any block size.
  void audio(std::span<const std::int16_t> stereo, std::uint64_t receivedMicros) {
    if (stopped_ || !options_.audio || !file_.isOpen() || stereo.size() % 2) return;
    // Position this audio on the video timeline using local arrival time.
    const auto frames = stereo.size() / 2;
    if (receivedMicros >= fileStartReceived_) {
      const auto arrivalEnd = (receivedMicros - fileStartReceived_) * kAudioRate / 1'000'000;
      const auto arrivalStart = arrivalEnd > frames ? arrivalEnd - frames : 0;
      const auto expected = audioPosition_ + pcm_.size() / 2;
      if (arrivalStart > expected + kAudioRate / 10) {  // >100 ms of missing audio
        pcm_.clear();
        if (!audioSamples_.empty()) flush();
        audioPosition_ = (arrivalStart / kAudioFrames) * kAudioFrames;
        audioRunStart_ = audioPosition_;
      }
    }
    pcm_.insert(pcm_.end(), stereo.begin(), stereo.end());
    while (pcm_.size() >= kAudioFrames * 2) {
      auto packet = options_.encodeAudio(std::span<const std::int16_t>(pcm_.data(), kAudioFrames * 2));
      pcm_.erase(pcm_.begin(), pcm_.begin() + kAudioFrames * 2);
      if (audioSamples_.empty()) audioRunStart_ = audioPosition_;
      audioPosition_ += kAudioFrames;
      if (!packet || packet->empty()) continue;
      audioSamples_.push_back({std::move(*packet), kAudioFrames, true});
    }
  }

  void finish(StopReason reason) {
    if (stopped_) return;
    if (file_.isOpen()) {
      if (pendingVideo_) {
        pendingVideo_->duration = kVideoTimescale / 30;
        videoSamples_.push_back(std::move(*pendingVideo_));
        pendingVideo_.reset();
      }
      flush();
    }
    stop(reason);
  }

  std::uint64_t gaps() const { return gaps_; }

private:
  RecorderOptions options_;
  RecorderEvents events_;
  FragmentedMp4 file_;
  VideoConfig config_, pendingConfig_;
  std::vector<Sample> videoSamples_, audioSamples_;
  std::optional<Sample> pendingVideo_;
  std::vector<std::int16_t> pcm_;
  std::uint64_t lastSequence_ = 0, lastTimestamp_ = 0, gaps_ = 0;
  std::uint64_t fileStartReceived_ = 0, audioPosition_ = 0, audioRunStart_ = 0;
  std::uint64_t completedBytes_ = 0, completedMicros_ = 0;
  unsigned part_ = 1;
  bool haveSequence_ = false, waitingForIdr_ = true, stopped_ = false, startedReported_ = false;
  StopReason reason_ = StopReason::None;

  static std::uint32_t ticksBetween(std::uint64_t previous, std::uint64_t next) {
    // Backwards or absurd jumps (sender clock reset) become one nominal frame.
    if (next <= previous || next - previous > 10'000'000) return kVideoTimescale / 60;
    return std::uint32_t(std::max<std::uint64_t>(1, (next - previous) * kVideoTimescale / 1'000'000));
  }
  std::uint64_t bufferedTicks() const {
    std::uint64_t total = 0;
    for (const auto& sample : videoSamples_) total += sample.duration;
    return total;
  }
  std::uint64_t bufferedBytes() const {
    std::uint64_t total = 0;
    for (const auto& sample : videoSamples_) total += sample.data.size();
    for (const auto& sample : audioSamples_) total += sample.data.size();
    return total;
  }
  std::uint64_t mediaMicros() const { return file_.videoTicks() * 1'000'000 / kVideoTimescale; }

  bool openFile(const VideoInput& input) {
    config_ = pendingConfig_;
    const auto path = partPath(options_.path, part_);
    if (!file_.open(path, config_, options_.audio, options_.audioPreSkip)) {
      stop(StopReason::WriteError);
      return false;
    }
    fileStartReceived_ = input.receivedMicros;
    audioPosition_ = audioRunStart_ = 0;
    pcm_.clear();
    audioSamples_.clear();
    if (!startedReported_ && events_.started) events_.started(path);
    startedReported_ = true;
    return true;
  }

  void flush() {
    if (!file_.isOpen()) return;
    if (!file_.fragment(videoSamples_, audioSamples_, audioRunStart_)) {
      stop(StopReason::WriteError);
      return;
    }
    audioRunStart_ = audioPosition_;
  }

  void checkLimits() {
    if (stopped_) return;
    if (totalBytes() + bufferedBytes() >= options_.maxBytes) finish(StopReason::SizeLimit);
    else if (completedMicros_ + mediaMicros() + bufferedTicks() * 1'000'000 / kVideoTimescale >= options_.maxDurationMicros)
      finish(StopReason::DurationLimit);
  }

  void stop(StopReason reason) {
    if (stopped_) return;
    const auto bytes = totalBytes();
    const auto micros = completedMicros_ + mediaMicros();
    file_.close();
    stopped_ = true;
    reason_ = reason;
    videoSamples_.clear(); audioSamples_.clear(); pendingVideo_.reset();
    if (events_.stopped) events_.stopped(partPath(options_.path, part_), reason, bytes, micros);
  }
};

// ---------------------------------------------------------------------------
// Thread-safe front end: copies input into a bounded queue served by one
// worker. Overflow drops pictures and forces an IDR wait instead of blocking.
// ---------------------------------------------------------------------------
class Recorder {
public:
  explicit Recorder(std::size_t maxQueuedBytes = 64u << 20) : maxQueued_(maxQueuedBytes) {}
  Recorder(const Recorder&) = delete;
  Recorder& operator=(const Recorder&) = delete;
  ~Recorder() { stop(StopReason::SessionEnded); }

  bool start(RecorderOptions options, RecorderEvents events) {
    stop(StopReason::User);
    std::lock_guard lock(mutex_);
    session_ = std::make_unique<RecordingSession>(std::move(options), std::move(events));
    running_ = true;
    dropping_ = false;
    queue_.clear();
    queued_ = 0;
    worker_ = std::thread([this] { run(); });
    active_.store(true);
    return true;
  }

  // Safe from any thread; returns quickly.
  void stop(StopReason reason) {
    std::thread worker;
    {
      std::lock_guard lock(mutex_);
      if (!running_) return;
      running_ = false;
      stopReason_ = reason;
      worker = std::move(worker_);
    }
    wake_.notify_all();
    if (worker.joinable()) worker.join();
    active_.store(false);
  }

  bool active() const { return active_.load(); }
  // True while the recorder waits for an IDR; callers request one (rate limited).
  bool needsKeyframe() const { return active_.load() && needsKeyframe_.load(); }

  void video(VideoInput input) {
    if (!active_.load()) return;
    {
      std::lock_guard lock(mutex_);
      if (!running_) return;
      if (queued_ + input.accessUnit.size() > maxQueued_) {
        dropping_ = true;  // disk is too slow; resume at the next IDR
        needsKeyframe_.store(true);
        return;
      }
      if (dropping_) { input.discontinuity = true; dropping_ = false; }
      queued_ += input.accessUnit.size();
      queue_.push_back(Item{std::move(input), {}, 0, false});
    }
    wake_.notify_one();
  }

  void audio(std::span<const std::int16_t> stereo, std::uint64_t receivedMicros) {
    if (!active_.load()) return;
    {
      std::lock_guard lock(mutex_);
      if (!running_ || queued_ + stereo.size() * 2 > maxQueued_) return;
      queued_ += stereo.size() * 2;
      Item item;
      item.isAudio = true;
      item.pcm.assign(stereo.begin(), stereo.end());
      item.received = receivedMicros;
      queue_.push_back(std::move(item));
    }
    wake_.notify_one();
  }

private:
  struct Item { VideoInput video; std::vector<std::int16_t> pcm; std::uint64_t received = 0; bool isAudio = false; };
  std::mutex mutex_;
  std::condition_variable wake_;
  std::deque<Item> queue_;
  std::size_t queued_ = 0, maxQueued_;
  std::unique_ptr<RecordingSession> session_;
  std::thread worker_;
  bool running_ = false, dropping_ = false;
  StopReason stopReason_ = StopReason::User;
  std::atomic<bool> active_{false}, needsKeyframe_{false};

  void run() {
    std::unique_lock lock(mutex_);
    auto* session = session_.get();
    needsKeyframe_.store(session->needsKeyframe());
    for (;;) {
      wake_.wait(lock, [&] { return !queue_.empty() || !running_; });
      // Stop drains what was already accepted (bounded by maxQueued_).
      if (queue_.empty()) break;
      auto item = std::move(queue_.front());
      queue_.pop_front();
      queued_ -= item.isAudio ? item.pcm.size() * 2 : item.video.accessUnit.size();
      lock.unlock();
      if (item.isAudio) session->audio(item.pcm, item.received);
      else session->video(item.video);
      needsKeyframe_.store(session->needsKeyframe());
      const bool ended = !session->active();
      lock.lock();
      if (ended) { active_.store(false); break; }
    }
    queue_.clear();
    queued_ = 0;
    const auto reason = stopReason_;
    auto finished = std::move(session_);
    lock.unlock();
    finished->finish(reason);
    finished.reset();
    lock.lock();
    active_.store(false);
  }
};

}  // namespace sanser::recording
