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
#include <cstring>
#include <iterator>
#include <iostream>
#include <stdexcept>
#include <string>
#include <utility>

using Microsoft::WRL::ComPtr;

namespace {

constexpr GUID kSubtypeH264 = {0x34363248, 0x0000, 0x0010, {0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71}};

struct FrameNv12 {
  std::uint32_t width = 0;
  std::uint32_t height = 0;
  std::uint32_t strideY = 0;
  std::uint32_t strideUv = 0;
  std::vector<std::uint8_t> pixels;
};

void checkHr(HRESULT hr, const char* label) {
  if (FAILED(hr)) {
    throw std::runtime_error(std::string(label) + ": " + hresultMessage(hr));
  }
}

const GUID& codecSubtype(VideoCodec codec) {
  if (codec != VideoCodec::H264) {
    throw std::runtime_error("Packet encoder currently supports H.264 only.");
  }
  return kSubtypeH264;
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

FrameNv12 convertBgraToNv12(const FrameBgra& frame) {
  if ((frame.width % 2) != 0 || (frame.height % 2) != 0) {
    throw std::runtime_error("NV12 encoder input requires even frame dimensions.");
  }
  if (frame.stride < frame.width * 4) {
    throw std::runtime_error("BGRA frame stride is smaller than expected.");
  }

  FrameNv12 result;
  result.width = frame.width;
  result.height = frame.height;
  result.strideY = frame.width;
  result.strideUv = frame.width;
  const std::size_t yBytes = static_cast<std::size_t>(result.strideY) * result.height;
  const std::size_t uvBytes = static_cast<std::size_t>(result.strideUv) * (result.height / 2);
  result.pixels.resize(yBytes + uvBytes);

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

ComPtr<IMFTransform> createEncoderTransform(VideoCodec codec, bool hardwareRequested, bool& usingHardware) {
  MFT_REGISTER_TYPE_INFO inputInfo{};
  inputInfo.guidMajorType = MFMediaType_Video;
  inputInfo.guidSubtype = MFVideoFormat_NV12;

  MFT_REGISTER_TYPE_INFO outputInfo{};
  outputInfo.guidMajorType = MFMediaType_Video;
  outputInfo.guidSubtype = codecSubtype(codec);

  auto tryCreate = [&](UINT32 flags, bool hardware) -> ComPtr<IMFTransform> {
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

    ComPtr<IMFTransform> transform;
    const HRESULT activateHr = activates[0]->ActivateObject(IID_PPV_ARGS(transform.GetAddressOf()));
    releaseActivates(activates, count);
    if (FAILED(activateHr)) return {};
    usingHardware = hardware;
    return transform;
  };

  if (hardwareRequested) {
    auto transform = tryCreate(MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER, true);
    if (transform) return transform;
  }

  auto transform = tryCreate(MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_ASYNCMFT |
                             MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER,
                             false);
  if (transform) return transform;

  throw std::runtime_error("No Media Foundation H.264 encoder MFT found.");
}

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
  bool providesOutputSamples = false;
};

MfVideoPacketEncoder::MfVideoPacketEncoder(std::uint32_t width,
                                           std::uint32_t height,
                                           const VideoPacketEncodeOptions& options)
  : impl_(std::make_unique<Impl>()) {
  impl_->width = width;
  impl_->height = height;
  impl_->fps = std::max<std::uint32_t>(options.fps, 1);
  impl_->options = options;
  impl_->options.codec = VideoCodec::H264;
  impl_->currentBitrate = impl_->options.bitrate;

  impl_->transform = createEncoderTransform(impl_->options.codec, options.hardware, impl_->usingHardware);

  ComPtr<IMFAttributes> transformAttributes;
  if (SUCCEEDED(impl_->transform->GetAttributes(transformAttributes.GetAddressOf())) && transformAttributes) {
    transformAttributes->SetUINT32(MF_TRANSFORM_ASYNC_UNLOCK, TRUE);
  }

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

  checkHr(impl_->transform->GetOutputStreamInfo(0, &impl_->outputInfo), "GetOutputStreamInfo");
  impl_->providesOutputSamples = (impl_->outputInfo.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES) != 0;

  notifyTransform(impl_->transform.Get(), MFT_MESSAGE_COMMAND_FLUSH, "Encoder flush");
  notifyTransform(impl_->transform.Get(), MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, "Encoder begin streaming");
  notifyTransform(impl_->transform.Get(), MFT_MESSAGE_NOTIFY_START_OF_STREAM, "Encoder start stream");
}

MfVideoPacketEncoder::~MfVideoPacketEncoder() = default;

bool MfVideoPacketEncoder::usingHardware() const {
  return impl_->usingHardware;
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
  if (frame.width != impl_->width || frame.height != impl_->height) {
    throw std::runtime_error("Frame dimensions do not match packet encoder dimensions.");
  }

  const FrameNv12 nv12 = convertBgraToNv12(frame);
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

  while (true) {
    ComPtr<IMFSample> sample;
    if (!impl_->providesOutputSamples) {
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
      break;
    }
    if (outputHr == MF_E_TRANSFORM_STREAM_CHANGE) {
      continue;
    }
    checkHr(outputHr, "ProcessOutput");

    ComPtr<IMFSample> outputSample;
    if (impl_->providesOutputSamples) {
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
    packet.payload.resize(currentLength);
    std::memcpy(packet.payload.data(), data, currentLength);

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
