// Exercise the production jitter/decode recovery code with a deterministic clock.
#define SANSER_LATENCY_TEST 1
#include "src/main.mm"

namespace {
struct NativeInputPolicyTest {
  static bool acknowledged(const std::string& event) { return NativeInputSender::isBatchable(event); }
};
}

void requireLatency(bool value, const char* message) {
  if (!value) throw std::runtime_error(message);
}
SnvPacket testPacket(std::uint64_t sequence, bool keyframe = false) {
  SnvPacket packet;
  packet.sequence = sequence;
  packet.timestampMicros = sequence * 8333;
  packet.durationMicros = 8333;
  packet.flags = keyframe ? 1 : 0;
  return packet;
}
int main() {
  @autoreleasepool {
    try {
      using namespace std::chrono;
      const auto start = steady_clock::now();
      SnvPacket packet;
      gLatencyPolicy.ultra = true;
      requireLatency(NativeInputPolicyTest::acknowledged("{\"type\":\"input-reset\"}"),
        "recovery reset must itself be acknowledged/retried");
      requireLatency(!NativeInputPolicyTest::acknowledged("{\"type\":\"gamepad-state\",\"connected\":true}"),
        "connected gamepad snapshots must not enter reliable backlog");
      requireLatency(NativeInputPolicyTest::acknowledged("{\"type\":\"gamepad-state\",\"connected\":false}"),
        "gamepad disconnect must remain reliable");
      UdpVideoPacketJitterBuffer jitter;
      jitter.resetForMediaGeneration(1);
      jitter.push(testPacket(1, true), 1, false, start);
      requireLatency(jitter.popReady(packet, start), "contiguous frame must not wait");
      jitter.push(testPacket(3), 1, false, start);
      requireLatency(!jitter.popReady(packet, start + milliseconds(11)), "reordering window must allow recovery");
      jitter.push(testPacket(2), 1, false, start + milliseconds(11));
      requireLatency(jitter.popReady(packet, start + milliseconds(11)) && packet.sequence == 2, "repaired frame order");
      requireLatency(jitter.popReady(packet, start + milliseconds(11)) && packet.sequence == 3, "following frame must release immediately");
      jitter.push(testPacket(5), 1, false, start + milliseconds(20));
      requireLatency(jitter.popReady(packet, start + milliseconds(32)) && packet.sequence == 5, "gaming gap must expire at 12 ms");
      requireLatency(!jitter.push(testPacket(4), 1, false, start + milliseconds(33)), "late repair must not resurrect expired frame");
      jitter.resetForMediaGeneration(2);
      jitter.push(testPacket(1, true), 2, false, start);
      requireLatency(jitter.popReady(packet, start) && packet.sequence == 1, "generation restart must accept a fresh sequence");

      ClientStreamStats stats;
      std::string reason;
      auto first = testPacket(1, true), gap = testPacket(3), delta = testPacket(4), idr = testPacket(5, true);
      stats.observe(first); requireLatency(!stats.shouldDropBeforeDecode(first, reason), "decode initial IDR");
      stats.observe(gap); requireLatency(stats.shouldDropBeforeDecode(gap, reason), "missing reference must block dependent frame");
      stats.observe(delta); requireLatency(stats.shouldDropBeforeDecode(delta, reason), "wait for IDR after deadline");
      stats.observe(idr); requireLatency(!stats.shouldDropBeforeDecode(idr, reason), "IDR must recover decoding");
      requireLatency(gLatencyPolicy.presentAt(10000, 1000000, 90000) == 14000, "future sender timeline cannot add latency");
      requireLatency(gLatencyPolicy.presentAt(10000, 0, 0) == 10000, "zero base render hold");

      CVPixelBufferRef pixels=nullptr;
      requireLatency(CVPixelBufferCreate(nullptr,32,32,kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
        nullptr,&pixels)==kCVReturnSuccess,"allocate queue test image");
      SanserVideoRenderer* renderer=[[SanserVideoRenderer alloc] initForQueueTest];
      DecodedVideoFrameMetadata metadata;
      metadata.durationMicros=8333;
      for(std::uint64_t i=1;i<=100;++i) {
        metadata.sequence=i; metadata.timestampMicros=i*8333; metadata.decodeSubmittedAtMicros=i;
        [renderer submitPixelBuffer:pixels metadata:&metadata];
        requireLatency([renderer testQueueSize]==1 && [renderer testQueuedSequence]==i,"newest decoded frame must replace pending frame");
      }
      metadata.sequence=99; metadata.decodeSubmittedAtMicros=99;
      [renderer submitPixelBuffer:pixels metadata:&metadata];
      requireLatency([renderer testQueuedSequence]==100,"late decode callback cannot replace newest image");
      metadata.sequence=1; metadata.timestampMicros=8333; metadata.decodeSubmittedAtMicros=101;
      [renderer submitPixelBuffer:pixels metadata:&metadata];
      requireLatency([renderer testQueueSize]==1 && [renderer testQueuedSequence]==1,"sender sequence restart must not freeze newest frame queue");
      CVPixelBufferRelease(pixels);

      gLatencyPolicy.ultra = false;
      UdpVideoPacketJitterBuffer balanced;
      balanced.push(testPacket(1), 1, false, start); balanced.popReady(packet, start);
      balanced.push(testPacket(3), 1, false, start);
      requireLatency(!balanced.popReady(packet, start + milliseconds(12)), "balanced profile retains reorder tolerance");
      requireLatency(balanced.popReady(packet, start + milliseconds(70)), "balanced deadline remains bounded");
      std::cout << "Latency: reorder recovery, expired repair, IDR gating, generation reset and pacing passed\n";
      return 0;
    } catch (const std::exception& error) {
      std::cerr << error.what() << '\n'; return 1;
    }
  }
}
