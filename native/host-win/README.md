# Sanser Native Windows Host

This is the first native-capture prototype for the long-term Sunshine/Parsec-style engine.

Current scope:

- Windows Desktop Duplication API
- D3D11 capture from a selected adapter/output
- BMP frame dump for validation
- BGRA frame pipe mode for Node/Electron integration
- Media Foundation H.264 file encode prototype with hardware transforms requested

Not included yet:

- Direct NVIDIA NVENC SDK backend with low-level GPU encoder controls
- Intel QuickSync / AMD AMF backend-specific controls
- Audio capture
- Network transport
- Native client decode/render

## Build

Install Visual Studio Build Tools with C++ Desktop workload and CMake, then run from the repository root:

```powershell
npm run native:host-win:configure
npm run native:host-win:build
```

## Run

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --frames 5 --interval-ms 100 --output-dir native-captures
```

Options:

```text
--frames N       Number of changed frames to write
--interval-ms N  Delay after each written frame
--output-dir DIR BMP output directory
--adapter N      DXGI adapter index
--output N       DXGI output/monitor index
```

If the desktop is idle, the duplicator may wait until the screen changes. Move the mouse or open a window to produce frames.

## Hardware Encoder Probe

List Windows Media Foundation hardware encoders:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --list-encoders
```

This reports hardware MFT encoders exposed by the GPU driver/Windows stack for H.264, H.265/HEVC, and AV1. On NVIDIA systems this may be backed by NVENC, but this prototype does not use the NVIDIA Video Codec SDK directly yet.

## H.264 Encode Test

Write a short H.264 MP4 from native captured frames:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode h264 --frames 300 --fps 60 --interval-ms 0 --bitrate 28000000 --output-file native-captures\capture_h264.mp4
```

Open `native-captures\capture_h264.mp4` after the command finishes. If the desktop is idle, move the mouse or drag a window so Desktop Duplication produces changed frames.

Use `--software-encoder` to disable the hardware-transform request and compare behavior:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode h264 --frames 300 --fps 60 --interval-ms 0 --software-encoder --output-file native-captures\capture_h264_software.mp4
```

## Pipe Mode

Pipe mode writes native captured frames to stdout:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --pipe --fps 60 > frames.snf
```

Each frame is:

```text
SNF1 header
BGRA8 pixel payload
```

Header layout, little-endian:

```cpp
struct PipeFrameHeader {
  char magic[4];              // "SNF1"
  uint32_t headerSize;        // sizeof(PipeFrameHeader)
  uint32_t width;
  uint32_t height;
  uint32_t stride;
  uint32_t pixelFormat;       // 1 = BGRA8
  uint64_t timestampMicros;
  uint32_t payloadSize;
};
```

This is the bridge format for the next step: Electron starts `sanser-native-host --pipe`, reads BGRA frames, and sends them to an encoder or a temporary preview path.
