#include "mf_video_packet_encoder.h"

#include <codecapi.h>
#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mftransform.h>
#include <objbase.h>
#include <strmif.h>
#include <wrl/client.h>

#include <algorithm>
#include <cctype>
#include <chrono>
#include <cmath>
#include <cstring>
#include <iterator>
#include <iostream>
#include <limits>
#include <stdexcept>
#include <string>
#include <thread>
#include <utility>
#include <vector>

using Microsoft::WRL::ComPtr;

namespace {

constexpr GUID kSubtypeH264 = {0x34363248, 0x0000, 0x0010, {0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71}};
constexpr GUID kSubtypeHevc = {0x43564548, 0x0000, 0x0010, {0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71}};

struct FrameNv12 {
  std::uint32_t width = 0;
  std::uint32_t height = 0;
  std::uint32_t strideY = 0;
  std::uint32_t strideUv = 0;
  std::vector<std::uint8_t> pixels;
};

struct LinearScaleCoordinate {
  std::uint32_t first = 0;
  std::uint32_t second = 0;
  std::uint16_t secondWeight = 0; // Fixed point in [0, 256].
};

struct BgraScalePlan {
  std::uint32_t sourceWidth = 0;
  std::uint32_t sourceHeight = 0;
  std::uint32_t targetWidth = 0;
  std::uint32_t targetHeight = 0;
  std::vector<LinearScaleCoordinate> xCoordinates;
  std::vector<LinearScaleCoordinate> yCoordinates;
};

struct RgbSample {
  int r = 0;
  int g = 0;
  int b = 0;
};

void checkHr(HRESULT hr, const char* label) {
  if (FAILED(hr)) {
    throw std::runtime_error(std::string(label) + ": " + hresultMessage(hr));
  }
}

const GUID& codecSubtype(VideoCodec codec) {
  switch (codec) {
    case VideoCodec::H264:
      return kSubtypeH264;
    case VideoCodec::Hevc:
      return kSubtypeHevc;
    case VideoCodec::Av1:
      break;
  }
  throw std::runtime_error("Packet encoder currently supports H.264 and H.265/HEVC only.");
}

class ComRuntime {
public:
  ComRuntime() {
    const HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (SUCCEEDED(hr)) {
      initialized_ = true;
    } else if (hr != RPC_E_CHANGED_MODE) {
      checkHr(hr, "CoInitializeEx");
    }
  }

  ~ComRuntime() {
    if (initialized_) {
      CoUninitialize();
    }
  }

  ComRuntime(const ComRuntime&) = delete;
  ComRuntime& operator=(const ComRuntime&) = delete;

private:
  bool initialized_ = false;
};

class MfRuntime {
public:
  MfRuntime() {
    checkHr(MFStartup(MF_VERSION), "MFStartup");
  }

  ~MfRuntime() {
    MFShutdown();
  }

  MfRuntime(const MfRuntime&) = delete;
  MfRuntime& operator=(const MfRuntime&) = delete;
};

std::uint8_t clampByte(int value) {
  return static_cast<std::uint8_t>(std::clamp(value, 0, 255));
}

std::uint8_t rgbToY(int r, int g, int b) {
  return clampByte(((66 * r + 129 * g + 25 * b + 128) >> 8) + 16);
}

std::uint8_t rgbToU(int r, int g, int b) {
  return clampByte(((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128);
}

std::uint8_t rgbToV(int r, int g, int b) {
  return clampByte(((112 * r - 94 * g - 18 * b + 128) >> 8) + 128);
}

void validateBgraFrame(const FrameBgra& frame) {
  if (frame.width == 0 || frame.height == 0) {
    throw std::runtime_error("BGRA source frame dimensions must be non-zero.");
  }
  if (frame.width > std::numeric_limits<std::size_t>::max() / 4) {
    throw std::runtime_error("BGRA source frame row size overflows addressable memory.");
  }
  const std::size_t minimumRowBytes = static_cast<std::size_t>(frame.width) * 4;
  if (static_cast<std::size_t>(frame.stride) < minimumRowBytes) {
    throw std::runtime_error("BGRA frame stride is smaller than expected.");
  }
  const std::size_t rowsBeforeLast = static_cast<std::size_t>(frame.height - 1);
  if (rowsBeforeLast > 0 &&
      static_cast<std::size_t>(frame.stride) >
        (std::numeric_limits<std::size_t>::max() - minimumRowBytes) / rowsBeforeLast) {
    throw std::runtime_error("BGRA source frame size overflows addressable memory.");
  }
  const std::size_t requiredBytes = rowsBeforeLast * static_cast<std::size_t>(frame.stride) +
                                    minimumRowBytes;
  if (frame.pixels.size() < requiredBytes) {
    throw std::runtime_error("BGRA source frame pixel buffer is truncated.");
  }
}

FrameNv12 allocateNv12Frame(std::uint32_t width, std::uint32_t height) {
  if (width == 0 || height == 0 || (width % 2) != 0 || (height % 2) != 0) {
    throw std::runtime_error("NV12 encoder input requires non-zero even frame dimensions.");
  }
  if (static_cast<std::size_t>(width) >
      std::numeric_limits<std::size_t>::max() / static_cast<std::size_t>(height)) {
    throw std::runtime_error("NV12 luma plane size overflows addressable memory.");
  }

  FrameNv12 result;
  result.width = width;
  result.height = height;
  result.strideY = width;
  result.strideUv = width;
  const std::size_t yBytes = static_cast<std::size_t>(result.strideY) * result.height;
  const std::size_t uvBytes = static_cast<std::size_t>(result.strideUv) * (result.height / 2);
  if (uvBytes > std::numeric_limits<std::size_t>::max() - yBytes) {
    throw std::runtime_error("NV12 frame size overflows addressable memory.");
  }
  const std::size_t totalBytes = yBytes + uvBytes;
  if (totalBytes > std::numeric_limits<DWORD>::max()) {
    throw std::runtime_error("NV12 encoder input exceeds Media Foundation buffer limits.");
  }
  result.pixels.resize(totalBytes);
  return result;
}

FrameNv12 convertBgraToNv12SameSize(const FrameBgra& frame) {
  validateBgraFrame(frame);
  FrameNv12 result = allocateNv12Frame(frame.width, frame.height);
  const std::size_t yBytes = static_cast<std::size_t>(result.strideY) * result.height;

  auto* yPlane = result.pixels.data();
  auto* uvPlane = result.pixels.data() + yBytes;

  for (std::uint32_t y = 0; y < frame.height; ++y) {
    const auto* srcRow = frame.pixels.data() + static_cast<std::size_t>(frame.stride) * y;
    auto* dstRow = yPlane + static_cast<std::size_t>(result.strideY) * y;
    for (std::uint32_t x = 0; x < frame.width; ++x) {
      const auto* pixel = srcRow + static_cast<std::size_t>(x) * 4;
      dstRow[x] = rgbToY(pixel[2], pixel[1], pixel[0]);
    }
  }

  for (std::uint32_t y = 0; y < frame.height; y += 2) {
    auto* uvRow = uvPlane + static_cast<std::size_t>(result.strideUv) * (y / 2);
    for (std::uint32_t x = 0; x < frame.width; x += 2) {
      int r = 0;
      int g = 0;
      int b = 0;
      for (std::uint32_t yy = 0; yy < 2; ++yy) {
        const auto* srcRow = frame.pixels.data() + static_cast<std::size_t>(frame.stride) * (y + yy);
        for (std::uint32_t xx = 0; xx < 2; ++xx) {
          const auto* pixel = srcRow + static_cast<std::size_t>(x + xx) * 4;
          b += pixel[0];
          g += pixel[1];
          r += pixel[2];
        }
      }
      r /= 4;
      g /= 4;
      b /= 4;
      uvRow[x] = rgbToU(r, g, b);
      uvRow[x + 1] = rgbToV(r, g, b);
    }
  }

  return result;
}

std::vector<LinearScaleCoordinate> makeLinearScaleCoordinates(std::uint32_t sourceSize,
                                                              std::uint32_t targetSize) {
  if (sourceSize == 0 || targetSize == 0) {
    throw std::runtime_error("Scale dimensions must be non-zero.");
  }

  std::vector<LinearScaleCoordinate> coordinates(targetSize);
  if (sourceSize == 1) return coordinates;

  const double scale = static_cast<double>(sourceSize) / static_cast<double>(targetSize);
  for (std::uint32_t target = 0; target < targetSize; ++target) {
    const double sourcePosition = (static_cast<double>(target) + 0.5) * scale - 0.5;
    if (sourcePosition <= 0.0) {
      coordinates[target] = {0, 0, 0};
      continue;
    }
    if (sourcePosition >= static_cast<double>(sourceSize - 1)) {
      const std::uint32_t last = sourceSize - 1;
      coordinates[target] = {last, last, 0};
      continue;
    }

    const auto first = static_cast<std::uint32_t>(std::floor(sourcePosition));
    const double fraction = sourcePosition - static_cast<double>(first);
    const auto secondWeight = static_cast<std::uint16_t>(std::clamp<long>(
      std::lround(fraction * 256.0),
      0,
      256));
    coordinates[target] = {first, first + 1, secondWeight};
  }
  return coordinates;
}

void ensureBgraScalePlan(BgraScalePlan& plan,
                         std::uint32_t sourceWidth,
                         std::uint32_t sourceHeight,
                         std::uint32_t targetWidth,
                         std::uint32_t targetHeight) {
  if (plan.sourceWidth == sourceWidth &&
      plan.sourceHeight == sourceHeight &&
      plan.targetWidth == targetWidth &&
      plan.targetHeight == targetHeight) {
    return;
  }

  BgraScalePlan next;
  next.sourceWidth = sourceWidth;
  next.sourceHeight = sourceHeight;
  next.targetWidth = targetWidth;
  next.targetHeight = targetHeight;
  next.xCoordinates = makeLinearScaleCoordinates(sourceWidth, targetWidth);
  next.yCoordinates = makeLinearScaleCoordinates(sourceHeight, targetHeight);
  plan = std::move(next);

  std::cerr << "SNV1_ENCODER_SCALE source=" << sourceWidth << "x" << sourceHeight
            << " target=" << targetWidth << "x" << targetHeight
            << " filter=bilinear-direct-nv12\n";
}

int bilinearChannel(std::uint8_t topLeft,
                    std::uint8_t topRight,
                    std::uint8_t bottomLeft,
                    std::uint8_t bottomRight,
                    std::uint16_t xWeight,
                    std::uint16_t yWeight) {
  constexpr std::uint32_t one = 256;
  const std::uint32_t inverseX = one - xWeight;
  const std::uint32_t inverseY = one - yWeight;
  const std::uint32_t top = static_cast<std::uint32_t>(topLeft) * inverseX +
                            static_cast<std::uint32_t>(topRight) * xWeight;
  const std::uint32_t bottom = static_cast<std::uint32_t>(bottomLeft) * inverseX +
                               static_cast<std::uint32_t>(bottomRight) * xWeight;
  return static_cast<int>((top * inverseY + bottom * yWeight + (one * one / 2)) /
                          (one * one));
}

RgbSample sampleBgraBilinear(const FrameBgra& frame,
                             const LinearScaleCoordinate& x,
                             const LinearScaleCoordinate& y) {
  const auto* topRow = frame.pixels.data() + static_cast<std::size_t>(frame.stride) * y.first;
  const auto* bottomRow = frame.pixels.data() + static_cast<std::size_t>(frame.stride) * y.second;
  const auto* topLeft = topRow + static_cast<std::size_t>(x.first) * 4;
  const auto* topRight = topRow + static_cast<std::size_t>(x.second) * 4;
  const auto* bottomLeft = bottomRow + static_cast<std::size_t>(x.first) * 4;
  const auto* bottomRight = bottomRow + static_cast<std::size_t>(x.second) * 4;

  RgbSample sample;
  sample.b = bilinearChannel(topLeft[0], topRight[0], bottomLeft[0], bottomRight[0],
                             x.secondWeight, y.secondWeight);
  sample.g = bilinearChannel(topLeft[1], topRight[1], bottomLeft[1], bottomRight[1],
                             x.secondWeight, y.secondWeight);
  sample.r = bilinearChannel(topLeft[2], topRight[2], bottomLeft[2], bottomRight[2],
                             x.secondWeight, y.secondWeight);
  return sample;
}

FrameNv12 convertBgraToNv12Scaled(const FrameBgra& frame,
                                  std::uint32_t targetWidth,
                                  std::uint32_t targetHeight,
                                  BgraScalePlan& scalePlan) {
  if (frame.width == targetWidth && frame.height == targetHeight) {
    return convertBgraToNv12SameSize(frame);
  }

  validateBgraFrame(frame);
  FrameNv12 result = allocateNv12Frame(targetWidth, targetHeight);
  ensureBgraScalePlan(scalePlan,
                      frame.width,
                      frame.height,
                      targetWidth,
                      targetHeight);

  const std::size_t yBytes = static_cast<std::size_t>(result.strideY) * result.height;
  auto* yPlane = result.pixels.data();
  auto* uvPlane = result.pixels.data() + yBytes;

  for (std::uint32_t y = 0; y < targetHeight; y += 2) {
    auto* yRow0 = yPlane + static_cast<std::size_t>(result.strideY) * y;
    auto* yRow1 = yPlane + static_cast<std::size_t>(result.strideY) * (y + 1);
    auto* uvRow = uvPlane + static_cast<std::size_t>(result.strideUv) * (y / 2);
    const auto& sourceY0 = scalePlan.yCoordinates[y];
    const auto& sourceY1 = scalePlan.yCoordinates[y + 1];

    for (std::uint32_t x = 0; x < targetWidth; x += 2) {
      const auto& sourceX0 = scalePlan.xCoordinates[x];
      const auto& sourceX1 = scalePlan.xCoordinates[x + 1];
      const RgbSample topLeft = sampleBgraBilinear(frame, sourceX0, sourceY0);
      const RgbSample topRight = sampleBgraBilinear(frame, sourceX1, sourceY0);
      const RgbSample bottomLeft = sampleBgraBilinear(frame, sourceX0, sourceY1);
      const RgbSample bottomRight = sampleBgraBilinear(frame, sourceX1, sourceY1);

      yRow0[x] = rgbToY(topLeft.r, topLeft.g, topLeft.b);
      yRow0[x + 1] = rgbToY(topRight.r, topRight.g, topRight.b);
      yRow1[x] = rgbToY(bottomLeft.r, bottomLeft.g, bottomLeft.b);
      yRow1[x + 1] = rgbToY(bottomRight.r, bottomRight.g, bottomRight.b);

      const int averageR = (topLeft.r + topRight.r + bottomLeft.r + bottomRight.r + 2) / 4;
      const int averageG = (topLeft.g + topRight.g + bottomLeft.g + bottomRight.g + 2) / 4;
      const int averageB = (topLeft.b + topRight.b + bottomLeft.b + bottomRight.b + 2) / 4;
      uvRow[x] = rgbToU(averageR, averageG, averageB);
      uvRow[x + 1] = rgbToV(averageR, averageG, averageB);
    }
  }

  return result;
}

void setVideoTypeCommon(IMFMediaType* type, std::uint32_t width, std::uint32_t height, std::uint32_t fps) {
  checkHr(type->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive), "Set interlace mode");
  checkHr(MFSetAttributeSize(type, MF_MT_FRAME_SIZE, width, height), "Set frame size");
  checkHr(MFSetAttributeRatio(type, MF_MT_FRAME_RATE, std::max<std::uint32_t>(fps, 1), 1), "Set frame rate");
  checkHr(MFSetAttributeRatio(type, MF_MT_PIXEL_ASPECT_RATIO, 1, 1), "Set pixel aspect ratio");
}

void notifyTransform(IMFTransform* transform, MFT_MESSAGE_TYPE message, const char* label) {
  const HRESULT hr = transform->ProcessMessage(message, 0);
  if (FAILED(hr) && hr != E_NOTIMPL) {
    // Hardware encoder MFTs vary here. Some fail startup flush before any input
    // frame even though ProcessInput/ProcessOutput works normally afterward.
    std::cerr << label << " ignored: " << hresultMessage(hr) << "\n";
  }
}

void logCodecApiResult(const char* label, HRESULT hr) {
  if (SUCCEEDED(hr)) {
    std::cerr << "SNV1 encoder option " << label << "=ok\n";
  } else {
    std::cerr << "SNV1 encoder option " << label << " ignored: " << hresultMessage(hr) << "\n";
  }
}

void setCodecApiBool(ICodecAPI* codecApi, const GUID& key, bool enabled, const char* label) {
  VARIANT value{};
  value.vt = VT_BOOL;
  value.boolVal = enabled ? VARIANT_TRUE : VARIANT_FALSE;
  logCodecApiResult(label, codecApi->SetValue(&key, &value));
}

void setCodecApiUInt32(ICodecAPI* codecApi, const GUID& key, std::uint32_t number, const char* label) {
  VARIANT value{};
  value.vt = VT_UI4;
  value.ulVal = number;
  logCodecApiResult(label, codecApi->SetValue(&key, &value));
}

void configureLowLatencyEncoder(IMFTransform* transform, const VideoPacketEncodeOptions& options) {
  if (!options.lowLatency) return;

  ComPtr<ICodecAPI> codecApi;
  const HRESULT queryHr = transform->QueryInterface(IID_PPV_ARGS(codecApi.GetAddressOf()));
  if (FAILED(queryHr) || !codecApi) {
    std::cerr << "SNV1 encoder low-latency CodecAPI unavailable: " << hresultMessage(queryHr) << "\n";
    return;
  }

  constexpr std::uint32_t cbrRateControlMode = 0;
  const std::uint32_t gopFrames = std::max<std::uint32_t>(
    1,
    std::max<std::uint32_t>(options.fps, 1) * std::max<std::uint32_t>(options.keyframeIntervalSeconds, 1));

  setCodecApiBool(codecApi.Get(), CODECAPI_AVLowLatencyMode, true, "low-latency");
  setCodecApiUInt32(codecApi.Get(), CODECAPI_AVEncCommonRateControlMode, cbrRateControlMode, "rate-control-cbr");
  setCodecApiUInt32(codecApi.Get(), CODECAPI_AVEncCommonMeanBitRate, options.bitrate, "mean-bitrate");
  setCodecApiUInt32(codecApi.Get(), CODECAPI_AVEncCommonMaxBitRate, options.bitrate, "max-bitrate");
  setCodecApiUInt32(codecApi.Get(), CODECAPI_AVEncMPVGOPSize, gopFrames, "gop");
  setCodecApiUInt32(codecApi.Get(), CODECAPI_AVEncMPVDefaultBPictureCount, 0, "b-frames");
  setCodecApiUInt32(codecApi.Get(), CODECAPI_AVEncCommonQualityVsSpeed, 100, "speed");

  std::cerr << "SNV1 encoder low-latency requested: cbr=yes gopFrames="
            << gopFrames << " bFrames=0 speed=100\n";
}

void releaseActivates(IMFActivate** activates, UINT32 count) {
  if (!activates) return;
  for (UINT32 i = 0; i < count; ++i) {
    if (activates[i]) {
      activates[i]->Release();
    }
  }
  CoTaskMemFree(activates);
}

std::string asciiFromWide(const WCHAR* value, UINT32 length) {
  std::string output;
  output.reserve(length);
  for (UINT32 i = 0; i < length; ++i) {
    const WCHAR ch = value[i];
    output.push_back(ch >= 0 && ch <= 0x7f ? static_cast<char>(ch) : '?');
  }
  return output;
}

std::string lowerCopy(std::string value) {
  std::transform(value.begin(), value.end(), value.begin(), [](unsigned char ch) {
    return static_cast<char>(std::tolower(ch));
  });
  return value;
}

std::string logValue(std::string value) {
  for (char& ch : value) {
    if (ch == '"' || ch == '\n' || ch == '\r' || ch == '\t') ch = ' ';
  }
  return value.empty() ? "unknown" : value;
}

std::string normalizeEncoderPreference(std::string value) {
  value = lowerCopy(value);
  if (value.empty() || value == "auto" || value == "best") return "auto";
  if (value == "nvidia" || value == "nvenc") return "nvenc";
  if (value == "amd" || value == "amf") return "amf";
  if (value == "intel" || value == "qsv" || value == "quicksync" || value == "quick-sync") return "qsv";
  if (value == "mf" || value == "mediafoundation" || value == "media-foundation") return "mf";
  if (value == "software" || value == "sw") return "software";
  throw std::runtime_error("Unsupported encoder preference: " + value);
}

struct EncoderCandidate {
  ComPtr<IMFActivate> activate;
  std::string name;
  std::string backend;
  UINT32 index = 0;
  int score = 0;
  bool hardware = false;
  bool asynchronous = false;
};

std::string friendlyName(IMFActivate* activate, UINT32 index) {
  WCHAR* friendlyName = nullptr;
  UINT32 friendlyNameLength = 0;
  const HRESULT nameHr = activate->GetAllocatedString(MFT_FRIENDLY_NAME_Attribute,
                                                      &friendlyName,
                                                      &friendlyNameLength);
  if (SUCCEEDED(nameHr) && friendlyName) {
    std::string result = asciiFromWide(friendlyName, friendlyNameLength);
    CoTaskMemFree(friendlyName);
    return result;
  }
  return "Media Foundation encoder " + std::to_string(index);
}

std::string detectBackend(const std::string& name, bool hardware) {
  const std::string lowered = lowerCopy(name);
  if (lowered.find("nvidia") != std::string::npos ||
      lowered.find("nvenc") != std::string::npos) {
    return "NVENC";
  }
  if (lowered.find("amd") != std::string::npos ||
      lowered.find("advanced micro") != std::string::npos ||
      lowered.find("amf") != std::string::npos) {
    return "AMF";
  }
  if (lowered.find("intel") != std::string::npos ||
      lowered.find("quick sync") != std::string::npos ||
      lowered.find("quicksync") != std::string::npos ||
      lowered.find("qsv") != std::string::npos) {
    return "QuickSync";
  }
  return hardware ? "MediaFoundation-HW" : "MediaFoundation-SW";
}

int backendScore(const std::string& backend, bool hardware) {
  if (backend == "NVENC") return 400;
  if (backend == "AMF") return 300;
  if (backend == "QuickSync") return 200;
  if (hardware) return 100;
  return 10;
}

std::vector<EncoderCandidate> enumerateEncoderCandidates(VideoCodec codec,
                                                         UINT32 flags,
                                                         bool hardware,
                                                         bool asynchronous) {
  MFT_REGISTER_TYPE_INFO inputInfo{};
  inputInfo.guidMajorType = MFMediaType_Video;
  inputInfo.guidSubtype = MFVideoFormat_NV12;

  MFT_REGISTER_TYPE_INFO outputInfo{};
  outputInfo.guidMajorType = MFMediaType_Video;
  outputInfo.guidSubtype = codecSubtype(codec);

  IMFActivate** activates = nullptr;
  UINT32 count = 0;
  const HRESULT enumHr = MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER,
                                   flags,
                                   &inputInfo,
                                   &outputInfo,
                                   &activates,
                                   &count);
  if (FAILED(enumHr) || count == 0) {
    releaseActivates(activates, count);
    return {};
  }

  std::vector<EncoderCandidate> candidates;
  candidates.reserve(count);
  for (UINT32 i = 0; i < count; ++i) {
    EncoderCandidate candidate;
    candidate.activate = activates[i];
    candidate.name = friendlyName(activates[i], i);
    candidate.backend = detectBackend(candidate.name, hardware);
    candidate.index = i;
    candidate.hardware = hardware;
    candidate.asynchronous = asynchronous;
    candidate.score = backendScore(candidate.backend, hardware);
    candidates.push_back(std::move(candidate));
  }
  releaseActivates(activates, count);
  return candidates;
}

bool preferenceMatches(const EncoderCandidate& candidate, const std::string& preference) {
  if (preference == "auto") return true;
  if (preference == "nvenc") return candidate.backend == "NVENC";
  if (preference == "amf") return candidate.backend == "AMF";
  if (preference == "qsv") return candidate.backend == "QuickSync";
  if (preference == "mf") return candidate.backend.find("MediaFoundation") == 0;
  if (preference == "software") return !candidate.hardware;
  return true;
}

std::vector<EncoderCandidate> sortCandidates(std::vector<EncoderCandidate> candidates,
                                             const std::string& preference) {
  candidates.erase(std::remove_if(candidates.begin(), candidates.end(), [&](const EncoderCandidate& candidate) {
    return !preferenceMatches(candidate, preference);
  }), candidates.end());

  std::stable_sort(candidates.begin(), candidates.end(), [&](const EncoderCandidate& a, const EncoderCandidate& b) {
    if (preference == "mf") {
      if (a.hardware != b.hardware) return a.hardware && !b.hardware;
      const bool aGeneric = a.backend.find("MediaFoundation") == 0;
      const bool bGeneric = b.backend.find("MediaFoundation") == 0;
      if (aGeneric != bGeneric) return aGeneric && !bGeneric;
    }
    if (a.score != b.score) return a.score > b.score;
    return a.index < b.index;
  });
  return candidates;
}

struct EncoderSelection {
  std::string name = "unknown";
  std::string backend = "unknown";
  bool hardware = false;
  bool asynchronous = false;
};

struct EncoderActivation {
  ComPtr<IMFTransform> transform;
  ComPtr<IMFMediaEventGenerator> eventGenerator;
  bool asynchronous = false;

  explicit operator bool() const {
    return transform.Get() != nullptr;
  }
};

EncoderActivation activateCandidate(const EncoderCandidate& candidate) {
  EncoderActivation activation;
  const HRESULT activateHr = candidate.activate->ActivateObject(
    IID_PPV_ARGS(activation.transform.GetAddressOf()));
  if (FAILED(activateHr)) {
    std::cerr << "SNV1_ENCODER_CANDIDATE_FAILED backend=" << candidate.backend
              << " hardware=" << (candidate.hardware ? "yes" : "no")
              << " name=\"" << logValue(candidate.name) << "\""
              << " error=" << hresultMessage(activateHr)
              << "\n";
    return {};
  }

  ComPtr<IMFAttributes> attributes;
  UINT32 asyncAttribute = FALSE;
  const HRESULT attributesHr = activation.transform->GetAttributes(attributes.GetAddressOf());
  if (SUCCEEDED(attributesHr) && attributes) {
    attributes->GetUINT32(MF_TRANSFORM_ASYNC, &asyncAttribute);
  }
  activation.asynchronous = candidate.asynchronous || asyncAttribute != FALSE;

  if (activation.asynchronous) {
    if (!attributes) {
      std::cerr << "SNV1_ENCODER_CANDIDATE_FAILED backend=" << candidate.backend
                << " hardware=" << (candidate.hardware ? "yes" : "no")
                << " name=\"" << logValue(candidate.name) << "\""
                << " reason=async-attributes-unavailable"
                << " error=" << hresultMessage(attributesHr)
                << "\n";
      return {};
    }
    const HRESULT unlockHr = attributes->SetUINT32(MF_TRANSFORM_ASYNC_UNLOCK, TRUE);
    if (FAILED(unlockHr)) {
      std::cerr << "SNV1_ENCODER_CANDIDATE_FAILED backend=" << candidate.backend
                << " hardware=" << (candidate.hardware ? "yes" : "no")
                << " name=\"" << logValue(candidate.name) << "\""
                << " reason=async-unlock-failed"
                << " error=" << hresultMessage(unlockHr)
                << "\n";
      return {};
    }
    const HRESULT eventsHr = activation.transform->QueryInterface(
      IID_PPV_ARGS(activation.eventGenerator.GetAddressOf()));
    if (FAILED(eventsHr) || !activation.eventGenerator) {
      std::cerr << "SNV1_ENCODER_CANDIDATE_FAILED backend=" << candidate.backend
                << " hardware=" << (candidate.hardware ? "yes" : "no")
                << " name=\"" << logValue(candidate.name) << "\""
                << " reason=async-events-unavailable"
                << " error=" << hresultMessage(eventsHr)
                << "\n";
      return {};
    }
  }
  return activation;
}

EncoderActivation createEncoderTransform(VideoCodec codec,
                                         const VideoPacketEncodeOptions& options,
                                         EncoderSelection& selection) {
  const bool wantsSoftware = !options.hardware ||
                             normalizeEncoderPreference(options.encoderPreference) == "software";
  const std::string preference = wantsSoftware
    ? "software"
    : normalizeEncoderPreference(options.encoderPreference);

  std::vector<EncoderCandidate> candidates;
  if (!wantsSoftware) {
    auto hardwareCandidates = enumerateEncoderCandidates(codec,
                                                         MFT_ENUM_FLAG_HARDWARE |
                                                           MFT_ENUM_FLAG_SORTANDFILTER,
                                                         true,
                                                         true);
    candidates.insert(candidates.end(),
                      std::make_move_iterator(hardwareCandidates.begin()),
                      std::make_move_iterator(hardwareCandidates.end()));
  }

  auto softwareCandidates = enumerateEncoderCandidates(codec,
                                                       MFT_ENUM_FLAG_SYNCMFT |
                                                         MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER,
                                                       false,
                                                       false);
  candidates.insert(candidates.end(),
                    std::make_move_iterator(softwareCandidates.begin()),
                    std::make_move_iterator(softwareCandidates.end()));

  std::vector<EncoderCandidate> preferred = sortCandidates(candidates, preference);
  if (preferred.empty() && preference != "auto" && preference != "software") {
    std::cerr << "SNV1_ENCODER_PREFERENCE_MISS preference=" << preference
              << " fallback=auto\n";
    preferred = sortCandidates(candidates, "auto");
  }

  auto tryCandidates = [&](const std::vector<EncoderCandidate>& list) -> EncoderActivation {
    for (const EncoderCandidate& candidate : list) {
      std::cerr << "SNV1_ENCODER_CANDIDATE backend=" << candidate.backend
                << " hardware=" << (candidate.hardware ? "yes" : "no")
                << " score=" << candidate.score
                << " name=\"" << logValue(candidate.name) << "\"\n";
      auto activation = activateCandidate(candidate);
      if (activation) {
        selection.name = candidate.name;
        selection.backend = candidate.backend;
        selection.hardware = candidate.hardware;
        selection.asynchronous = activation.asynchronous;
        return activation;
      }
    }
    return {};
  };

  if (auto activation = tryCandidates(preferred)) {
    return activation;
  }

  if (preference != "auto" && preference != "software") {
    std::cerr << "SNV1_ENCODER_ACTIVATION_FALLBACK preference=" << preference
              << " fallback=auto\n";
    if (auto activation = tryCandidates(sortCandidates(candidates, "auto"))) {
      return activation;
    }
  }

  throw std::runtime_error("No Media Foundation " + videoCodecName(codec) + " encoder MFT found.");
}

struct OutputPullResult {
  EncodedVideoPacket packet;
  bool producedPacket = false;
  bool needMoreInput = false;
  bool streamChanged = false;
  bool incomplete = false;
};

} // namespace

struct MfVideoPacketEncoder::Impl {
  ComRuntime com;
  MfRuntime mf;
  ComPtr<IMFTransform> transform;
  MFT_OUTPUT_STREAM_INFO outputInfo{};
  VideoPacketEncodeOptions options;
  std::uint32_t width = 0;
  std::uint32_t height = 0;
  std::uint32_t fps = 60;
  std::uint32_t currentBitrate = 0;
  std::uint64_t frameIndex = 0;
  bool usingHardware = false;
  std::string encoderName = "unknown";
  std::string encoderBackend = "unknown";
  bool providesOutputSamples = false;
  std::uint32_t consecutiveStreamChanges = 0;
  BgraScalePlan scalePlan;

  void refreshOutputStreamInfo() {
    MFT_OUTPUT_STREAM_INFO nextInfo{};
    checkHr(transform->GetOutputStreamInfo(0, &nextInfo), "GetOutputStreamInfo");
    outputInfo = nextInfo;
    providesOutputSamples = (outputInfo.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES) != 0;
  }

  void renegotiateOutputType() {
    HRESULT lastHr = MF_E_NO_MORE_TYPES;
    for (DWORD typeIndex = 0; typeIndex < 64; ++typeIndex) {
      ComPtr<IMFMediaType> candidateType;
      const HRESULT availableHr = transform->GetOutputAvailableType(0,
                                                                    typeIndex,
                                                                    candidateType.GetAddressOf());
      if (availableHr == MF_E_NO_MORE_TYPES) break;
      if (FAILED(availableHr)) {
        lastHr = availableHr;
        break;
      }

      GUID majorType{};
      GUID subtype{};
      if (FAILED(candidateType->GetGUID(MF_MT_MAJOR_TYPE, &majorType)) ||
          FAILED(candidateType->GetGUID(MF_MT_SUBTYPE, &subtype)) ||
          !IsEqualGUID(majorType, MFMediaType_Video) ||
          !IsEqualGUID(subtype, codecSubtype(options.codec))) {
        continue;
      }
      UINT32 candidateWidth = 0;
      UINT32 candidateHeight = 0;
      const HRESULT sizeHr = MFGetAttributeSize(candidateType.Get(),
                                                MF_MT_FRAME_SIZE,
                                                &candidateWidth,
                                                &candidateHeight);
      if (SUCCEEDED(sizeHr) && (candidateWidth != width || candidateHeight != height)) {
        continue;
      }

      const HRESULT setHr = transform->SetOutputType(0, candidateType.Get(), 0);
      if (SUCCEEDED(setHr)) {
        refreshOutputStreamInfo();
        std::cerr << "SNV1_ENCODER_STREAM_CHANGE renegotiated=yes"
                  << " typeIndex=" << typeIndex
                  << " codec=" << videoCodecName(options.codec)
                  << " providesSamples=" << (providesOutputSamples ? "yes" : "no")
                  << "\n";
        return;
      }
      lastHr = setHr;
    }

    throw std::runtime_error("Could not renegotiate encoder output type after stream change: " +
                             hresultMessage(lastHr));
  }
};

MfVideoPacketEncoder::MfVideoPacketEncoder(std::uint32_t width,
                                           std::uint32_t height,
                                           const VideoPacketEncodeOptions& options)
  : impl_(std::make_unique<Impl>()) {
  if (width == 0 || height == 0 || (width % 2) != 0 || (height % 2) != 0) {
    throw std::runtime_error("Packet encoder dimensions must be non-zero and even for NV12 input.");
  }
  impl_->width = width;
  impl_->height = height;
  impl_->fps = std::max<std::uint32_t>(options.fps, 1);
  impl_->options = options;
  impl_->currentBitrate = impl_->options.bitrate;

  EncoderSelection encoderSelection;
  impl_->transform = createEncoderTransform(impl_->options.codec, impl_->options, encoderSelection);
  impl_->usingHardware = encoderSelection.hardware;
  impl_->encoderName = encoderSelection.name;
  impl_->encoderBackend = encoderSelection.backend;
  std::cerr << "SNV1_ENCODER_SELECTED preference="
            << normalizeEncoderPreference(impl_->options.encoderPreference)
            << " backend=" << impl_->encoderBackend
            << " hardware=" << (impl_->usingHardware ? "yes" : "no")
            << " name=\"" << logValue(impl_->encoderName) << "\"\n";

  ComPtr<IMFMediaType> outputType;
  checkHr(MFCreateMediaType(outputType.GetAddressOf()), "MFCreateMediaType output");
  checkHr(outputType->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video), "Set output major type");
  checkHr(outputType->SetGUID(MF_MT_SUBTYPE, codecSubtype(impl_->options.codec)), "Set output subtype");
  checkHr(outputType->SetUINT32(MF_MT_AVG_BITRATE, impl_->options.bitrate), "Set output bitrate");
  setVideoTypeCommon(outputType.Get(), width, height, impl_->fps);
  checkHr(impl_->transform->SetOutputType(0, outputType.Get(), 0), "Set encoder output type");

  ComPtr<IMFMediaType> inputType;
  checkHr(MFCreateMediaType(inputType.GetAddressOf()), "MFCreateMediaType input");
  checkHr(inputType->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video), "Set input major type");
  checkHr(inputType->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12), "Set input subtype");
  checkHr(inputType->SetUINT32(MF_MT_DEFAULT_STRIDE, width), "Set input stride");
  setVideoTypeCommon(inputType.Get(), width, height, impl_->fps);
  checkHr(impl_->transform->SetInputType(0, inputType.Get(), 0), "Set encoder input type");

  configureLowLatencyEncoder(impl_->transform.Get(), impl_->options);

  impl_->refreshOutputStreamInfo();

  notifyTransform(impl_->transform.Get(), MFT_MESSAGE_COMMAND_FLUSH, "Encoder flush");
  notifyTransform(impl_->transform.Get(), MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, "Encoder begin streaming");
  notifyTransform(impl_->transform.Get(), MFT_MESSAGE_NOTIFY_START_OF_STREAM, "Encoder start stream");
}

MfVideoPacketEncoder::~MfVideoPacketEncoder() = default;

bool MfVideoPacketEncoder::usingHardware() const {
  return impl_->usingHardware;
}

VideoCodec MfVideoPacketEncoder::codec() const {
  return impl_->options.codec;
}

std::string MfVideoPacketEncoder::encoderName() const {
  return impl_->encoderName;
}

std::string MfVideoPacketEncoder::encoderBackend() const {
  return impl_->encoderBackend;
}

std::uint32_t MfVideoPacketEncoder::bitrate() const {
  return impl_->currentBitrate;
}

bool MfVideoPacketEncoder::setBitrate(std::uint32_t bitrate) {
  bitrate = std::max<std::uint32_t>(bitrate, 1);
  ComPtr<ICodecAPI> codecApi;
  const HRESULT queryHr = impl_->transform->QueryInterface(IID_PPV_ARGS(codecApi.GetAddressOf()));
  if (FAILED(queryHr) || !codecApi) {
    std::cerr << "SNV1_ADAPT bitrate change unavailable: " << hresultMessage(queryHr) << "\n";
    return false;
  }

  VARIANT value{};
  value.vt = VT_UI4;
  value.ulVal = bitrate;
  const HRESULT meanHr = codecApi->SetValue(&CODECAPI_AVEncCommonMeanBitRate, &value);
  const HRESULT maxHr = codecApi->SetValue(&CODECAPI_AVEncCommonMaxBitRate, &value);
  if (FAILED(meanHr) && FAILED(maxHr)) {
    std::cerr << "SNV1_ADAPT bitrate change ignored: mean=" << hresultMessage(meanHr)
              << " max=" << hresultMessage(maxHr) << "\n";
    return false;
  }

  impl_->currentBitrate = bitrate;
  std::cerr << "SNV1_ADAPT bitrate target=" << bitrate << "\n";
  return true;
}

bool MfVideoPacketEncoder::requestKeyframe() {
  ComPtr<ICodecAPI> codecApi;
  const HRESULT queryHr = impl_->transform->QueryInterface(IID_PPV_ARGS(codecApi.GetAddressOf()));
  if (FAILED(queryHr) || !codecApi) {
    std::cerr << "SNV1_KEYFRAME_FORCE unavailable: " << hresultMessage(queryHr) << "\n";
    return false;
  }

  VARIANT value{};
  value.vt = VT_UI4;
  value.ulVal = 1;
  HRESULT forceHr = codecApi->SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &value);
  if (FAILED(forceHr)) {
    VARIANT boolValue{};
    boolValue.vt = VT_BOOL;
    boolValue.boolVal = VARIANT_TRUE;
    forceHr = codecApi->SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &boolValue);
  }

  if (FAILED(forceHr)) {
    std::cerr << "SNV1_KEYFRAME_FORCE ignored: " << hresultMessage(forceHr) << "\n";
    return false;
  }

  std::cerr << "SNV1_KEYFRAME_FORCE api=codecapi requested=yes\n";
  return true;
}

std::vector<EncodedVideoPacket> MfVideoPacketEncoder::encodeFrame(const FrameBgra& frame) {
  const FrameNv12 nv12 = convertBgraToNv12Scaled(frame,
                                                 impl_->width,
                                                 impl_->height,
                                                 impl_->scalePlan);
  if (nv12.pixels.size() > std::numeric_limits<DWORD>::max()) {
    throw std::runtime_error("NV12 encoder input exceeds Media Foundation buffer limits.");
  }
  const DWORD bufferSize = static_cast<DWORD>(nv12.pixels.size());

  ComPtr<IMFMediaBuffer> buffer;
  checkHr(MFCreateMemoryBuffer(bufferSize, buffer.GetAddressOf()), "MFCreateMemoryBuffer");

  BYTE* destination = nullptr;
  DWORD maxLength = 0;
  DWORD currentLength = 0;
  checkHr(buffer->Lock(&destination, &maxLength, &currentLength), "Lock encoder input buffer");
  std::memcpy(destination, nv12.pixels.data(), std::min<std::size_t>(maxLength, nv12.pixels.size()));
  checkHr(buffer->Unlock(), "Unlock encoder input buffer");
  checkHr(buffer->SetCurrentLength(bufferSize), "Set encoder input length");

  ComPtr<IMFSample> sample;
  checkHr(MFCreateSample(sample.GetAddressOf()), "MFCreateSample input");
  checkHr(sample->AddBuffer(buffer.Get()), "Add input buffer");

  constexpr LONGLONG oneSecond = 10000000;
  const LONGLONG duration = oneSecond / static_cast<LONGLONG>(impl_->fps);
  const LONGLONG sampleTime = static_cast<LONGLONG>(impl_->frameIndex) * duration;
  checkHr(sample->SetSampleTime(sampleTime), "Set input sample time");
  checkHr(sample->SetSampleDuration(duration), "Set input sample duration");

  HRESULT inputHr = impl_->transform->ProcessInput(0, sample.Get(), 0);
  std::vector<EncodedVideoPacket> packets;
  if (inputHr == MF_E_NOTACCEPTING) {
    packets = drain();
    inputHr = impl_->transform->ProcessInput(0, sample.Get(), 0);
  }
  checkHr(inputHr, "ProcessInput");
  ++impl_->frameIndex;

  auto morePackets = drain();
  packets.insert(packets.end(),
                 std::make_move_iterator(morePackets.begin()),
                 std::make_move_iterator(morePackets.end()));
  return packets;
}

std::vector<EncodedVideoPacket> MfVideoPacketEncoder::drain() {
  std::vector<EncodedVideoPacket> packets;
  constexpr std::uint32_t maxConsecutiveStreamChanges = 4;

  while (true) {
    ComPtr<IMFSample> sample;
    const bool transformProvidesOutputSamples = impl_->providesOutputSamples;
    if (!transformProvidesOutputSamples) {
      const DWORD outputBufferSize = std::max<DWORD>(
        impl_->outputInfo.cbSize,
        static_cast<DWORD>(std::max<std::uint32_t>(impl_->width * impl_->height, 1024 * 1024)));
      ComPtr<IMFMediaBuffer> outputBuffer;
      checkHr(MFCreateMemoryBuffer(outputBufferSize, outputBuffer.GetAddressOf()), "Create output buffer");
      checkHr(MFCreateSample(sample.GetAddressOf()), "Create output sample");
      checkHr(sample->AddBuffer(outputBuffer.Get()), "Add output buffer");
    }

    MFT_OUTPUT_DATA_BUFFER output{};
    output.dwStreamID = 0;
    output.pSample = sample.Get();
    DWORD status = 0;
    const HRESULT outputHr = impl_->transform->ProcessOutput(0, 1, &output, &status);

    if (output.pEvents) {
      output.pEvents->Release();
      output.pEvents = nullptr;
    }

    if (outputHr == MF_E_TRANSFORM_NEED_MORE_INPUT) {
      if (transformProvidesOutputSamples && output.pSample) {
        output.pSample->Release();
        output.pSample = nullptr;
      }
      break;
    }
    if (outputHr == MF_E_TRANSFORM_STREAM_CHANGE) {
      if (transformProvidesOutputSamples && output.pSample) {
        output.pSample->Release();
        output.pSample = nullptr;
      }
      ++impl_->consecutiveStreamChanges;
      if (impl_->consecutiveStreamChanges > maxConsecutiveStreamChanges) {
        throw std::runtime_error("Encoder reported too many consecutive output stream changes.");
      }
      impl_->renegotiateOutputType();
      continue;
    }
    if (FAILED(outputHr) && transformProvidesOutputSamples && output.pSample) {
      output.pSample->Release();
      output.pSample = nullptr;
    }
    checkHr(outputHr, "ProcessOutput");
    impl_->consecutiveStreamChanges = 0;

    ComPtr<IMFSample> outputSample;
    if (transformProvidesOutputSamples) {
      outputSample.Attach(output.pSample);
      output.pSample = nullptr;
    } else {
      outputSample = sample;
    }
    if (!outputSample) continue;

    ComPtr<IMFMediaBuffer> contiguous;
    checkHr(outputSample->ConvertToContiguousBuffer(contiguous.GetAddressOf()), "Convert output sample");

    BYTE* data = nullptr;
    DWORD maxLength = 0;
    DWORD currentLength = 0;
    checkHr(contiguous->Lock(&data, &maxLength, &currentLength), "Lock output sample");

    EncodedVideoPacket packet;
    try {
      packet.payload.resize(currentLength);
      std::memcpy(packet.payload.data(), data, currentLength);
    } catch (...) {
      contiguous->Unlock();
      throw;
    }

    checkHr(contiguous->Unlock(), "Unlock output sample");

    LONGLONG sampleTime = 0;
    if (SUCCEEDED(outputSample->GetSampleTime(&sampleTime))) {
      packet.timestampMicros = static_cast<std::uint64_t>(sampleTime / 10);
    }
    LONGLONG sampleDuration = 0;
    if (SUCCEEDED(outputSample->GetSampleDuration(&sampleDuration))) {
      packet.durationMicros = static_cast<std::uint32_t>(sampleDuration / 10);
    } else {
      packet.durationMicros = 1000000 / impl_->fps;
    }
    UINT32 cleanPoint = FALSE;
    if (SUCCEEDED(outputSample->GetUINT32(MFSampleExtension_CleanPoint, &cleanPoint))) {
      packet.keyframe = cleanPoint == TRUE;
    }

    if (!packet.payload.empty()) {
      packets.push_back(std::move(packet));
    }
  }

  return packets;
}

std::vector<EncodedVideoPacket> MfVideoPacketEncoder::finish() {
  impl_->transform->ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
  impl_->transform->ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0);
  return drain();
}
