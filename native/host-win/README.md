# Sanser Native Windows Host

This is the native Windows capture/encode host for the low-latency SNV transport.

Current scope:

- Windows Desktop Duplication API
- D3D11 capture from a selected adapter/output
- Recovery after Desktop Duplication access loss, including display rotation
- BMP frame dump for validation
- BGRA frame pipe mode for Node/Electron integration
- Media Foundation H.264/HEVC file and realtime packet encoding
- TCP and fragmented UDP SNV1 video transport with pacing, NACK retransmit, and adaptive feedback
- WASAPI loopback audio transport
- Authenticated keyboard, mouse, and gamepad control backchannel
- Electron launcher integration

Known limitations:

- Direct NVIDIA NVENC SDK backend with low-level GPU encoder controls
- Intel QuickSync / AMD AMF backend-specific controls
- Fully asynchronous Media Foundation transform pumping across every GPU driver; the
  realtime packet path currently chooses a synchronous MFT for correctness

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
--stream-width N Maximum realtime SNV width while preserving aspect ratio
--stream-height N Maximum realtime SNV height while preserving aspect ratio
```

If the desktop is idle, the duplicator may wait until the screen changes. Move the mouse or open a window to produce frames.

## Hardware Encoder Probe

List Windows Media Foundation hardware encoders:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --list-encoders
```

This reports hardware MFT encoders exposed by the GPU driver/Windows stack for H.264, H.265/HEVC, and AV1. On NVIDIA systems this may be backed by NVENC. The file encoder can use those transforms, while the realtime packet encoder currently falls back to a synchronous Media Foundation encoder until its asynchronous event pump is implemented; this avoids driver-dependent stalls and corrupted output.

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

## H.264 Packet Pipe Test

Write realtime-style H.264 packets with `SNV1` headers:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode-pipe h264 --frames 180 --fps 60 --interval-ms 0 --bitrate 28000000 --packet-file native-captures\capture_h264.snv
```

For a 720p-bounded stream from a larger desktop, add `--stream-width 1280 --stream-height 720`. The host converts BGRA directly into scaled NV12 without allocating an intermediate BGRA frame, and the SNV header reports the fitted output dimensions.

Omit `--packet-file` only when another process is reading stdout:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode-pipe h264 --fps 60 --interval-ms 0
```

Do not run the stdout form directly in a normal terminal unless you are redirecting or piping it; it writes binary data.

Stream packets to the macOS native client over TCP:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode-pipe h264 --frames 180 --fps 60 --interval-ms 0 --bitrate 28000000 --tcp-connect <MAC_IP>:7777
```

Start the Mac receiver first:

```bash
npm run native:client-mac:listen-snv -- 7777 --max-packets 180
```

Inspect a written packet file:

```powershell
npm run native:host-win:inspect-snv -- native-captures\capture_h264.snv
```

If the hardware encoder fails on your GPU/driver stack, retry with the software encoder:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode-pipe h264 --frames 180 --fps 60 --interval-ms 0 --bitrate 12000000 --software-encoder --packet-file native-captures\capture_h264_software.snv
```

Each packet is:

```text
SNV1 header
H.264 sample payload emitted by Media Foundation
```

Header layout, little-endian:

```cpp
struct EncodedPacketHeader {
  char magic[4];              // "SNV1"
  uint32_t headerSize;        // sizeof(EncodedPacketHeader)
  uint32_t codec;             // 1 = H.264
  uint32_t packetFormat;      // 1 = Media Foundation compressed sample bytes
  uint32_t width;
  uint32_t height;
  uint64_t sequence;
  uint64_t timestampMicros;
  uint32_t durationMicros;
  uint32_t flags;             // bit 0 = keyframe, bit 1 = hardware encoder
  uint32_t payloadSize;
};
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
