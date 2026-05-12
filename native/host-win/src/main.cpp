#include "bmp_writer.h"
#include "desktop_duplication.h"
#include "mf_video_encoder.h"
#include "mf_video_packet_encoder.h"

#include <chrono>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <limits>
#include <memory>
#include <string>
#include <thread>

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

struct Options {
  std::filesystem::path outputDir = "captures";
  std::filesystem::path outputFile = "captures/capture_h264.mp4";
  std::filesystem::path packetFile;
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
        << "  --adapter N      DXGI adapter index, default 0\n"
        << "  --output N       DXGI output/monitor index, default 0\n"
        << "  --pipe           Write BGRA frames to stdout with SNF1 headers\n"
        << "  --fps N          Target FPS in pipe/encode mode, default 30\n"
        << "  --encode CODEC   Encode captured frames with Media Foundation: h264, hevc, av1\n"
        << "  --encode-pipe CODEC Encode H.264 packets to stdout or --packet-file with SNV1 headers\n"
        << "  --bitrate N      Target encode bitrate, default 28000000\n"
        << "  --output-file P  Encoded output file path\n"
        << "  --packet-file P  SNV1 packet output file; stdout is used when omitted\n"
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

void writeEncodedPacket(std::ostream& output,
                        const EncodedVideoPacket& packet,
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

  output.write(reinterpret_cast<const char*>(&header), sizeof(header));
  output.write(reinterpret_cast<const char*>(packet.payload.data()), packet.payload.size());
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

int runEncodedPipeMode(DesktopDuplicator& duplicator, const Options& options) {
  if (options.codec != VideoCodec::H264) {
    throw std::runtime_error("--encode-pipe currently supports h264 only.");
  }

  std::unique_ptr<std::ofstream> packetFile;
  std::ostream* output = &std::cout;
  if (!options.packetFile.empty()) {
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
            << (options.packetFile.empty() ? " -> stdout" : " -> " + options.packetFile.string())
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

    auto packets = encoder.encodeFrame(frame);
    for (const auto& packet : packets) {
      writeEncodedPacket(*output,
                         packet,
                         duplicator.width(),
                         duplicator.height(),
                         sequence++,
                         encoder.usingHardware());
    }
    output->flush();
    ++capturedFrames;

    if (options.intervalMs > 0) {
      std::this_thread::sleep_for(std::chrono::milliseconds(options.intervalMs));
    } else if (frameDelay.count() > 0) {
      std::this_thread::sleep_until(frameStartedAt + frameDelay);
    }
  }

  auto packets = encoder.finish();
  for (const auto& packet : packets) {
    writeEncodedPacket(*output,
                       packet,
                       duplicator.width(),
                       duplicator.height(),
                       sequence++,
                       encoder.usingHardware());
  }
  output->flush();

  std::cerr << "SNV1 packet stream wrote " << sequence << " packet(s).\n";
  return 0;
}

int main(int argc, char** argv) {
  try {
    const Options options = parseOptions(argc, argv);

    if (options.listEncoders) {
      listHardwareVideoEncoders(std::cout);
      return 0;
    }

    DesktopDuplicator duplicator;
    duplicator.initialize(options.adapterIndex, options.outputIndex);

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
