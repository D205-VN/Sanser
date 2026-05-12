#include "mf_video_encoder.h"

#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <objbase.h>
#include <wrl/client.h>

#include <algorithm>
#include <cctype>
#include <cstring>
#include <iostream>
#include <stdexcept>
#include <vector>

using Microsoft::WRL::ComPtr;

namespace {

constexpr GUID kSubtypeH264 = {0x34363248, 0x0000, 0x0010, {0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71}};
constexpr GUID kSubtypeHevc = {0x43564548, 0x0000, 0x0010, {0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71}};
constexpr GUID kSubtypeAv1 = {0x31305641, 0x0000, 0x0010, {0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71}};

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
      return kSubtypeAv1;
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

void setVideoTypeCommon(IMFMediaType* type, std::uint32_t width, std::uint32_t height, std::uint32_t fps) {
  checkHr(type->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive), "Set interlace mode");
  checkHr(MFSetAttributeSize(type, MF_MT_FRAME_SIZE, width, height), "Set frame size");
  checkHr(MFSetAttributeRatio(type, MF_MT_FRAME_RATE, std::max<std::uint32_t>(fps, 1), 1), "Set frame rate");
  checkHr(MFSetAttributeRatio(type, MF_MT_PIXEL_ASPECT_RATIO, 1, 1), "Set pixel aspect ratio");
}

std::wstring widen(const std::filesystem::path& path) {
  return path.wstring();
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

void listCodecHardwareEncoders(std::ostream& output, VideoCodec codec) {
  MFT_REGISTER_TYPE_INFO outputInfo{};
  outputInfo.guidMajorType = MFMediaType_Video;
  outputInfo.guidSubtype = codecSubtype(codec);

  IMFActivate** activates = nullptr;
  UINT32 count = 0;
  const HRESULT hr = MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER,
                               MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
                               nullptr,
                               &outputInfo,
                               &activates,
                               &count);
  if (FAILED(hr)) {
    output << videoCodecName(codec) << ": enumerate failed (" << hresultMessage(hr) << ")\n";
    return;
  }

  output << videoCodecName(codec) << ": " << count << " hardware encoder(s)\n";
  for (UINT32 i = 0; i < count; ++i) {
    WCHAR* friendlyName = nullptr;
    UINT32 friendlyNameLength = 0;
    const HRESULT nameHr = activates[i]->GetAllocatedString(MFT_FRIENDLY_NAME_Attribute,
                                                            &friendlyName,
                                                            &friendlyNameLength);
    if (SUCCEEDED(nameHr) && friendlyName) {
      std::wstring name(friendlyName, friendlyNameLength);
      output << "  - ";
      for (wchar_t ch : name) {
        output << static_cast<char>(ch <= 0x7f ? ch : '?');
      }
      output << "\n";
      CoTaskMemFree(friendlyName);
    } else {
      output << "  - encoder " << i << "\n";
    }
  }

  releaseActivates(activates, count);
}

} // namespace

struct MfVideoFileEncoder::Impl {
  ComRuntime com;
  MfRuntime mf;
  ComPtr<IMFSinkWriter> writer;
  DWORD streamIndex = 0;
  std::uint32_t width = 0;
  std::uint32_t height = 0;
  std::uint32_t fps = 60;
  std::uint64_t frameIndex = 0;
  bool finalized = false;
};

MfVideoFileEncoder::MfVideoFileEncoder(std::uint32_t width,
                                       std::uint32_t height,
                                       const VideoEncodeOptions& options)
  : impl_(std::make_unique<Impl>()) {
  impl_->width = width;
  impl_->height = height;
  impl_->fps = std::max<std::uint32_t>(options.fps, 1);

  ComPtr<IMFAttributes> attributes;
  checkHr(MFCreateAttributes(attributes.GetAddressOf(), 3), "MFCreateAttributes");
  checkHr(attributes->SetUINT32(MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, options.hardware ? TRUE : FALSE),
          "Enable hardware transforms");
  checkHr(attributes->SetUINT32(MF_SINK_WRITER_DISABLE_THROTTLING, TRUE), "Disable sink writer throttling");

  const std::wstring outputFile = widen(options.outputFile);
  checkHr(MFCreateSinkWriterFromURL(outputFile.c_str(), nullptr, attributes.Get(), impl_->writer.GetAddressOf()),
          "MFCreateSinkWriterFromURL");

  ComPtr<IMFMediaType> outputType;
  checkHr(MFCreateMediaType(outputType.GetAddressOf()), "MFCreateMediaType output");
  checkHr(outputType->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video), "Set output major type");
  checkHr(outputType->SetGUID(MF_MT_SUBTYPE, codecSubtype(options.codec)), "Set output subtype");
  checkHr(outputType->SetUINT32(MF_MT_AVG_BITRATE, options.bitrate), "Set bitrate");
  setVideoTypeCommon(outputType.Get(), width, height, impl_->fps);

  checkHr(impl_->writer->AddStream(outputType.Get(), &impl_->streamIndex), "Add video stream");

  ComPtr<IMFMediaType> inputType;
  checkHr(MFCreateMediaType(inputType.GetAddressOf()), "MFCreateMediaType input");
  checkHr(inputType->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video), "Set input major type");
  checkHr(inputType->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_RGB32), "Set input subtype");
  checkHr(inputType->SetUINT32(MF_MT_DEFAULT_STRIDE, width * 4), "Set input stride");
  setVideoTypeCommon(inputType.Get(), width, height, impl_->fps);

  checkHr(impl_->writer->SetInputMediaType(impl_->streamIndex, inputType.Get(), nullptr), "Set input media type");
  checkHr(impl_->writer->BeginWriting(), "BeginWriting");
}

MfVideoFileEncoder::~MfVideoFileEncoder() {
  try {
    finalize();
  } catch (...) {
  }
}

void MfVideoFileEncoder::writeFrame(const FrameBgra& frame) {
  if (impl_->finalized) {
    throw std::runtime_error("Encoder is already finalized.");
  }
  if (frame.width != impl_->width || frame.height != impl_->height || frame.stride != impl_->width * 4) {
    throw std::runtime_error("Frame dimensions do not match encoder dimensions.");
  }

  const DWORD bufferSize = static_cast<DWORD>(frame.pixels.size());
  ComPtr<IMFMediaBuffer> buffer;
  checkHr(MFCreateMemoryBuffer(bufferSize, buffer.GetAddressOf()), "MFCreateMemoryBuffer");

  BYTE* destination = nullptr;
  DWORD maxLength = 0;
  DWORD currentLength = 0;
  checkHr(buffer->Lock(&destination, &maxLength, &currentLength), "Lock sample buffer");
  std::memcpy(destination, frame.pixels.data(), std::min<std::size_t>(maxLength, frame.pixels.size()));
  checkHr(buffer->Unlock(), "Unlock sample buffer");
  checkHr(buffer->SetCurrentLength(bufferSize), "Set sample buffer length");

  ComPtr<IMFSample> sample;
  checkHr(MFCreateSample(sample.GetAddressOf()), "MFCreateSample");
  checkHr(sample->AddBuffer(buffer.Get()), "AddBuffer");

  constexpr LONGLONG oneSecond = 10000000;
  const LONGLONG duration = oneSecond / static_cast<LONGLONG>(impl_->fps);
  const LONGLONG sampleTime = static_cast<LONGLONG>(impl_->frameIndex) * duration;
  checkHr(sample->SetSampleTime(sampleTime), "SetSampleTime");
  checkHr(sample->SetSampleDuration(duration), "SetSampleDuration");

  checkHr(impl_->writer->WriteSample(impl_->streamIndex, sample.Get()), "WriteSample");
  ++impl_->frameIndex;
}

void MfVideoFileEncoder::finalize() {
  if (!impl_ || impl_->finalized) return;
  checkHr(impl_->writer->Finalize(), "Finalize");
  impl_->writer.Reset();
  impl_->finalized = true;
}

VideoCodec parseVideoCodec(const std::string& value) {
  std::string lowered = value;
  std::transform(lowered.begin(), lowered.end(), lowered.begin(), [](unsigned char ch) {
    return static_cast<char>(std::tolower(ch));
  });

  if (lowered == "h264" || lowered == "h.264" || lowered == "avc") return VideoCodec::H264;
  if (lowered == "h265" || lowered == "h.265" || lowered == "hevc") return VideoCodec::Hevc;
  if (lowered == "av1") return VideoCodec::Av1;
  throw std::runtime_error("Unsupported codec: " + value);
}

std::string videoCodecName(VideoCodec codec) {
  switch (codec) {
    case VideoCodec::H264:
      return "H.264";
    case VideoCodec::Hevc:
      return "H.265/HEVC";
    case VideoCodec::Av1:
      return "AV1";
  }
  return "H.264";
}

void listHardwareVideoEncoders(std::ostream& output) {
  ComRuntime com;
  MfRuntime mf;

  output << "Media Foundation hardware video encoders:\n";
  listCodecHardwareEncoders(output, VideoCodec::H264);
  listCodecHardwareEncoders(output, VideoCodec::Hevc);
  listCodecHardwareEncoders(output, VideoCodec::Av1);
}
