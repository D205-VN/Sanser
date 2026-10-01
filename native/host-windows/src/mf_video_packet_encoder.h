#pragma once

#include "desktop_duplication.h"
#include "mf_video_encoder.h"

#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <vector>

struct VideoPacketEncodeOptions {
  VideoCodec codec = VideoCodec::H264;
  std::string encoderPreference = "auto";
  std::uint32_t fps = 60;
  std::uint32_t bitrate = 28000000;
  std::uint32_t keyframeIntervalSeconds = 1;
  bool hardware = true;
  bool lowLatency = true;
  bool live = false;
  ID3D11Device* gpuDevice = nullptr;
};

struct EncodedVideoPacket {
  std::vector<std::uint8_t> payload;
  std::uint64_t timestampMicros = 0;
  std::uint32_t durationMicros = 0;
  bool keyframe = false;
};

class MfVideoPacketEncoder {
public:
  MfVideoPacketEncoder(std::uint32_t width,
                       std::uint32_t height,
                       const VideoPacketEncodeOptions& options);
  ~MfVideoPacketEncoder();

  MfVideoPacketEncoder(const MfVideoPacketEncoder&) = delete;
  MfVideoPacketEncoder& operator=(const MfVideoPacketEncoder&) = delete;

  std::vector<EncodedVideoPacket> encodeFrame(const FrameBgra& frame,
      std::optional<std::uint64_t> timestampMicros = std::nullopt);
  std::vector<EncodedVideoPacket> drain();
  std::vector<EncodedVideoPacket> finish();

  bool setBitrate(std::uint32_t bitrate);
  bool requestKeyframe();
  std::uint32_t bitrate() const;
  VideoCodec codec() const;
  bool usingHardware() const;
  bool usingGpuInput() const;
  std::uint64_t skippedCaptures() const;
  std::string encoderName() const;
  std::string encoderBackend() const;

private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};
