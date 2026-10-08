// Exercise the production jitter/decode recovery code with a deterministic clock.
#define SANSER_LATENCY_TEST 1
#include "src/main.mm"

namespace {
struct NativeInputPolicyTest {
  static auto retryDelay() { return NativeInputSender::retryDelay(); }
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
      UdpVideoStartup silent(true, start);
      requireLatency(silent.failure(0, start + seconds(19)) == nullptr, "allow host startup before deadline");
      requireLatency(std::string(silent.failure(0, start + seconds(20))).find("No UDP packets") != std::string::npos,
        "successful probing without native traffic must time out");
      UdpVideoStartup foreignPeer(true, start);
      foreignPeer.rawDatagrams = foreignPeer.unexpectedPeer = 100;
      requireLatency(std::string(foreignPeer.failure(0, start + seconds(20))).find("unexpected peer") != std::string::npos,
        "unrelated UDP must neither satisfy nor extend startup deadline");
      UdpVideoStartup controlOnly(true, start);
      controlOnly.rawDatagrams = controlOnly.controlDatagrams = 100;
      requireLatency(std::string(controlOnly.failure(0, start + seconds(20))).find("no video") != std::string::npos,
        "control keepalives must not conceal a missing video stream");
      controlOnly.videoDatagrams = controlOnly.authRejected = 50;
      requireLatency(std::string(controlOnly.failure(0, start + seconds(20))).find("authentication failed") != std::string::npos,
        "credential rejection must be distinguishable from transport silence");
      controlOnly.authRejected = 0;
      requireLatency(std::string(controlOnly.failure(0, start + seconds(20))).find("no complete video") != std::string::npos,
        "fragment loss must be distinguishable from decode failure");
      controlOnly.completed = 1;
      requireLatency(std::string(controlOnly.failure(0, start + seconds(20))).find("no frame could be decoded") != std::string::npos,
        "completed packets do not prove a decoded frame exists");
      requireLatency(controlOnly.failure(1, start + seconds(20)) == nullptr &&
        controlOnly.failure(1, start + hours(1)) == nullptr, "first decoded frame disables startup timeout on idle desktops");
      UdpVideoStartup standalone(false, start);
      requireLatency(standalone.failure(0, start + hours(1)) == nullptr, "standalone listeners may wait for a host");
      SnvPacket packet;
      gLatencyPolicy.ultra = true;
      requireLatency(NativeInputPolicyTest::acknowledged("{\"type\":\"input-reset\"}"),
        "recovery reset must itself be acknowledged/retried");
      requireLatency(!NativeInputPolicyTest::acknowledged("{\"type\":\"gamepad-state\",\"connected\":true}"),
        "connected gamepad snapshots must not enter reliable backlog");
      requireLatency(NativeInputPolicyTest::acknowledged("{\"type\":\"gamepad-state\",\"connected\":false}"),
        "gamepad disconnect must remain reliable");
      gLatencyPolicy.ultra = false;
      requireLatency(!NativeInputPolicyTest::acknowledged("{\"type\":\"gamepad-state\",\"connected\":true}"),
        "Auto must not retransmit obsolete gamepad snapshots");
      gLiveLatency.rtt = 10; gLiveLatency.hostUpdated = steadyMicros();
      requireLatency(NativeInputPolicyTest::retryDelay() == milliseconds(17), "LAN retry follows RTT");
      gLiveLatency.rtt = 100;
      requireLatency(NativeInputPolicyTest::retryDelay() == milliseconds(152), "WAN retry must allow one RTT");
      gLiveLatency.hostUpdated = 0;
      requireLatency(NativeInputPolicyTest::retryDelay() == milliseconds(40), "stale RTT must not drive input retries");
      gLiveLatency.rtt = -1;
      gLatencyPolicy.ultra = true;
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
      ClientStreamStats arrivalStats;
      auto received = testPacket(1, true);
      received.receivedAtMicros = 1000000;
      arrivalStats.observeArrival(received, received.receivedAtMicros);
      arrivalStats.observe(received);
      // Capture cadence changes/idle gaps are not network jitter when arrival
      // follows the sender timestamp, even though duration remains 8333 us.
      received.sequence = 4;
      received.timestampMicros += 50000;
      received.receivedAtMicros += 50000;
      arrivalStats.observeArrival(received, received.receivedAtMicros);
      arrivalStats.observe(received); // delayed dequeue must not sample twice
      requireLatency(arrivalStats.jitterMs == 0, "source cadence or sequence loss must not invent arrival jitter");
      received.sequence = 5;
      received.timestampMicros += 8333;
      received.receivedAtMicros += 8333 + 45000;
      arrivalStats.observeArrival(received, received.receivedAtMicros);
      const auto measuredJitter = arrivalStats.jitterMs;
      arrivalStats.observe(received);
      requireLatency(std::abs(measuredJitter - 45.0 / 16) < 0.001 && arrivalStats.jitterMs == measuredJitter,
        "actual 45 ms arrival variation must be measured once before the jitter buffer");
      arrivalStats.resetSequencing();
      arrivalStats.observeArrival(testPacket(1), 2000000);
      requireLatency(arrivalStats.jitterMs == 0, "new media generation must reset jitter baseline");
      gLiveLatency.jitter = 0;
      std::string reason;
      auto first = testPacket(1, true), gap = testPacket(3), delta = testPacket(4), idr = testPacket(5, true);
      stats.observe(first); requireLatency(!stats.shouldDropBeforeDecode(first, reason), "decode initial IDR");
      stats.observe(gap); requireLatency(stats.shouldDropBeforeDecode(gap, reason), "missing reference must block dependent frame");
      stats.observe(delta); requireLatency(stats.shouldDropBeforeDecode(delta, reason), "wait for IDR after deadline");
      stats.observe(idr); requireLatency(!stats.shouldDropBeforeDecode(idr, reason), "IDR must recover decoding");
      // Reproduce the 2.1.9 regression: intact frames arriving >15 ms late
      // repeatedly entered IDR wait, dropping up to 46/60 frames on real Wi-Fi.
      ClientStreamStats variableArrival;
      std::uint64_t arrival = 1000000;
      for (std::uint64_t sequence = 1; sequence <= 120; ++sequence) {
        auto next = testPacket(sequence, sequence == 1);
        arrival += 8333 + (sequence % 3 == 0 ? 45000 : 0);
        variableArrival.observe(next);
        requireLatency(!variableArrival.shouldDropBeforeDecode(next, reason, arrival),
          "intact references must survive variable arrival time without waiting for IDR");
      }
      requireLatency(variableArrival.keyframeWaitDropped == 0 && variableArrival.latencyDropped == 0,
        "arrival jitter alone must not drop the encoded reference chain");
      requireLatency(gLatencyPolicy.jitterHoldMs(0) == 12 && gLatencyPolicy.jitterHoldMs(20) == 30 &&
        gLatencyPolicy.jitterHoldMs(1000) == 50, "gap recovery must adapt within bounded limits");
      requireLatency(gLatencyPolicy.repairDeadlineMs() == 60, "repair must allow a round trip before expiry");
      requireLatency(gLatencyPolicy.retransmitCacheMs() == 150 &&
        gLatencyPolicy.retransmitCacheMs() > gLatencyPolicy.repairDeadlineMs(), "cache retention must not equal playout deadline");
      requireLatency(gLatencyPolicy.sendBudgetMicros(8333) == 3000 &&
        gLatencyPolicy.sendBudgetMicros(16667) == 4166 && gLatencyPolicy.sendBudgetMicros(50000) == 5000,
        "Ultra software pacing budget must stay within 3-5 ms");
      requireLatency(gLatencyPolicy.jitterHoldMs(0, 10) == 18, "gap deadline must allow feedback round trip");

      UdpVideoReassembler repair;
      UdpVideoStats repairStats;
      UdpPeerLock repairPeer("TEST");
      const auto peer = resolveUdpAddress("127.0.0.1:55000");
      std::vector<std::uint8_t> assembled;
      std::uint64_t packetId = 0, epoch = 0;
      bool grace = false;
      auto fragment = [&](std::uint64_t id, std::uint16_t index) {
        UdpVideoFragmentHeader header;
        header.packetId = id; header.packetSize = 3; header.fragmentCount = 3;
        header.fragmentIndex = index; header.fragmentOffset = index; header.payloadSize = 1;
        std::vector<std::uint8_t> bytes(sizeof(header) + 1, static_cast<std::uint8_t>(index));
        std::memcpy(bytes.data(), &header, sizeof(header));
        return repair.push(bytes, assembled, repairStats, packetId, epoch, grace, peer, repairPeer);
      };
      requireLatency(!fragment(1, 0) && !fragment(1, 2), "missing middle fragment stays incomplete");
      repair.pollRepairs(repairStats, start);
      repair.pollRepairs(repairStats, start + milliseconds(2));
      requireLatency(repairStats.newNackPacketIds.empty(), "short reordering must not produce premature NACK");
      repair.pollRepairs(repairStats, start + milliseconds(3));
      requireLatency(repairStats.newNackPacketIds == std::vector<std::uint64_t>{1}, "missing fragment must request repair after 3 ms");
      requireLatency(fragment(1, 1) && assembled == std::vector<std::uint8_t>({0, 1, 2}), "repair must retain and complete partial frame");
      repairStats.newNackPacketIds.clear();
      requireLatency(!fragment(2, 0) && !fragment(2, 2) && fragment(2, 1), "reordering before deadline completes normally");
      repair.pollRepairs(repairStats, start + milliseconds(10));
      requireLatency(repairStats.newNackPacketIds.empty(), "completed reordering must not request repair");
      requireLatency(!fragment(3, 0), "missing tail stays incomplete on an idle desktop");
      const auto idle = steady_clock::now() + milliseconds(51);
      repair.pollRepairs(repairStats, idle);
      repair.pollRepairs(repairStats, idle + milliseconds(3));
      requireLatency(repairStats.newNackPacketIds == std::vector<std::uint64_t>{3}, "idle missing tail must request repair without another video packet");
      repair.pollRepairs(repairStats, idle + seconds(1));
      requireLatency(repairStats.droppedAssemblies == 1, "partial frames must expire during receive silence");
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
