#pragma once

#include "desktop_duplication.h"

#include <cstdint>
#include <filesystem>
#include <iosfwd>
#include <memory>
#include <string>

enum class VideoCodec {
  H264,
  Hevc,
  Av1,
};

struct VideoEncodeOptions {
  VideoCodec codec = VideoCodec::H264;
  std::filesystem::path outputFile = "captures/capture_h264.mp4";
  std::uint32_t fps = 60;
  std::uint32_t bitrate = 28000000;
  bool hardware = true;
};

class MfVideoFileEncoder {
public:
  MfVideoFileEncoder(std::uint32_t width,
                     std::uint32_t height,
                     const VideoEncodeOptions& options);
  ~MfVideoFileEncoder();

  MfVideoFileEncoder(const MfVideoFileEncoder&) = delete;
  MfVideoFileEncoder& operator=(const MfVideoFileEncoder&) = delete;

  void writeFrame(const FrameBgra& frame);
  void finalize();

private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

VideoCodec parseVideoCodec(const std::string& value);
std::string videoCodecName(VideoCodec codec);
void listHardwareVideoEncoders(std::ostream& output);
