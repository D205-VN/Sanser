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
      // Exercise the actual asynchronous UDP sender, demux timestamps and reply
      // parser. A timing receipt must not replace the original pong's T5/T6.
      {
        ScopedFd receiver(socket(AF_INET, SOCK_DGRAM, 0));
        ScopedFd senderFd(socket(AF_INET, SOCK_DGRAM, 0));
        sockaddr_in local{}; local.sin_family = AF_INET;
        local.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
        requireLatency(bind(receiver.get(), reinterpret_cast<sockaddr*>(&local), sizeof(local)) == 0,
          "bind timing loopback");
        socklen_t size = sizeof(local);
        requireLatency(getsockname(receiver.get(), reinterpret_cast<sockaddr*>(&local), &size) == 0,
          "read timing port");
        UdpEndpoint target{}; target.length = sizeof(local);
        std::memcpy(&target.address, &local, sizeof(local));
        NativeInputSender sender;
        const std::string token = "timing-test-session";
        sender.setAuthRequired(true, token);
        sender.setControlAuthenticated(true);
        sender.setPacketAuthEnabled(true);
        sender.setUdpTarget(senderFd.get(), target);
        const auto originalSent = steadyMicros();
        requireLatency(sender.sendJson("{\"type\":\"control-ping\",\"sentSteadyMicros\":" +
          std::to_string(originalSent) + "}"), "enqueue real control ping");
        pollfd ready{receiver.get(), POLLIN, 0};
        requireLatency(poll(&ready, 1, 2000) == 1, "sender must wake for ping");
        char buffer[4096]; const auto bytes = recv(receiver.get(), buffer, sizeof(buffer), 0);
        requireLatency(bytes > 1 && buffer[0] == 0x02, "control uses multiplexed UDP");
        const std::string envelope(buffer+1, bytes-1);
        std::string request;
        std::uint64_t clientSequence = 0;
        requireLatency(unwrapSecureControlEnvelope(envelope, token, "c2h", clientSequence, request),
          "measurement fields covered by packet authentication");
        gExpectedSessionToken = token;
        gHostPacketAuthVerified = true;
        gLastHostSecureSequence = 0;
        const auto probe = jsonUint64Value(request, "timingProbe");
        requireLatency(probe && jsonUint64Value(request, "clientQueuedMicros") >= originalSent,
          "outbound request carries per-attempt probe and enqueue time");
        const auto received = steadyMicros();
        const std::string pong = "{\"type\":\"control-pong\",\"timingProbe\":" + std::to_string(probe) +
          ",\"sentSteadyMicros\":" + std::to_string(originalSent) + "}";
        takeControlRttWindow();
        requireLatency(handleHostControlPayload(makeSecureControlEnvelope(token, "h2c", 1, pong), received) == HostControlEvent::Pong,
          "production pong parser");
        requireLatency(gLiveLatency.controlTimingUpdated == 0, "no fabricated wire RTT before receipt");
        const auto beforeReceipt = steadyMicros();
        const std::string receipt = "{\"type\":\"control-timing\",\"timingProbe\":" + std::to_string(probe) +
          ",\"hostReceivedMicros\":900000000,\"hostDequeuedMicros\":900000000,\"hostSentMicros\":900000000}";
        handleHostControlPayload(receipt, beforeReceipt);
        requireLatency(gLiveLatency.controlTimingUpdated == 0, "unauthenticated receipt rejected");
        handleHostControlPayload(makeSecureControlEnvelope(token, "h2c", 2, receipt), beforeReceipt);
        requireLatency(gLiveLatency.controlTimingUpdated > 0 && gLiveLatency.wireEstimate >= 0,
          "receipt publishes complete timeline");
        const auto window = takeControlRttWindow();
        requireLatency(window.samples == 1, "one control ping sample");
        sender.setBatchEnabled(true);
        requireLatency(sender.sendJson("{\"type\":\"key-down\",\"keyCode\":65}"), "enqueue input for timing");
        requireLatency(poll(&ready, 1, 2000) == 1, "input sender must wake");
        const auto inputBytes = recv(receiver.get(), buffer, sizeof(buffer), 0);
        requireLatency(inputBytes > 1 && buffer[0] == 0x02, "input stays on UDP");
        std::string input;
        requireLatency(unwrapSecureControlEnvelope(std::string(buffer+1, inputBytes-1), token,
          "c2h", clientSequence, input), "authenticated input batch");
        const auto inputProbe = jsonUint64Value(input, "timingProbe");
        requireLatency(inputProbe && inputProbe != probe && jsonStringValue(input, "type") == "input-batch",
          "sampled input has independent attempt ID");
        const auto inputSent = jsonUint64Value(input, "sentSteadyMicros");
        const auto inputQueued = jsonUint64Value(input, "clientQueuedMicros");
        requireLatency(inputQueued > 0 && inputQueued <= inputSent, "batch preserves event enqueue time");
        handleHostControlPayload(makeSecureControlEnvelope(token, "h2c", 3,
          "{\"type\":\"input-ack\",\"sequence\":1,\"timingProbe\":" + std::to_string(inputProbe) +
          ",\"sentSteadyMicros\":" + std::to_string(inputSent) + "}"), steadyMicros());
        handleHostControlPayload(makeSecureControlEnvelope(token, "h2c", 4,
          "{\"type\":\"control-timing\",\"timingProbe\":" + std::to_string(inputProbe) +
          ",\"hostReceivedMicros\":900000000,\"hostDequeuedMicros\":900000000,\"hostSentMicros\":900000000}"), steadyMicros());
        requireLatency(takeControlRttWindow().samples == 0, "input ACK must not pollute ping RTT");
        gLiveLatency.controlTimingUpdated = 0;
        gExpectedSessionToken.clear(); gHostPacketAuthVerified = false; gLastHostSecureSequence = 0;
      }
      // A deliberately stalled video consumer must not prevent socket demux.
      {
        auto bindLoopback = [](int fd) {
          UdpEndpoint endpoint{}; endpoint.length = sizeof(sockaddr_in);
          auto* address = reinterpret_cast<sockaddr_in*>(&endpoint.address);
          address->sin_family = AF_INET; address->sin_addr.s_addr = htonl(INADDR_LOOPBACK);
          requireLatency(bind(fd, reinterpret_cast<sockaddr*>(address), endpoint.length) == 0, "bind receive pump test");
          requireLatency(getsockname(fd, reinterpret_cast<sockaddr*>(address), &endpoint.length) == 0, "pump endpoint");
          return endpoint;
        };
        ScopedFd receiveFd(socket(AF_INET, SOCK_DGRAM, 0));
        ScopedFd sourceFd(socket(AF_INET, SOCK_DGRAM, 0));
        ScopedFd foreignFd(socket(AF_INET, SOCK_DGRAM, 0));
        const auto destination = bindLoopback(receiveFd.get());
        const auto source = bindLoopback(sourceFd.get());
        bindLoopback(foreignFd.get());
        auto send = [&](int fd, const std::vector<std::uint8_t>& bytes) {
          requireLatency(sendto(fd, bytes.data(), bytes.size(), 0,
            reinterpret_cast<const sockaddr*>(&destination.address), destination.length) == static_cast<ssize_t>(bytes.size()),
            "send pump test datagram");
        };
        gControlUdpQueue.clear(); gAudioUdpQueue.clear();
        UdpReceivePump pump(receiveFd.get(), true, &source, 8, 50000);
        for (int i=0; i<40; ++i) send(sourceFd.get(), {0x00, static_cast<std::uint8_t>(i)});
        // Do not pop ANY video until control and audio have both been delivered.
        send(foreignFd.get(), {0x02, 'x'});
        send(sourceFd.get(), {0x01, 0x5a});
        const auto beforePong = steadyMicros();
        send(sourceFd.get(), {0x02, 'p'});
        sanser::ReceivedControl pong;
        requireLatency(gControlUdpQueue.pop_with_timeout(pong, std::chrono::seconds(2)),
          "control must arrive while video processing is completely stopped");
        requireLatency(pong.payload == "p" && pong.receivedMicros >= beforePong && pong.receivedMicros <= steadyMicros(),
          "preserve socket receive timestamp and reject unexpected source");
        std::vector<std::uint8_t> audio;
        requireLatency(gAudioUdpQueue.pop_with_timeout(audio, std::chrono::seconds(2)) && audio == std::vector<std::uint8_t>{0x5a},
          "audio bypasses a full video queue");
        const auto window = pump.takeWindow();
        requireLatency(window.depth <= 8 && window.highWater <= 8 && pump.dropped > 0,
          "slow video consumer cannot grow memory without bound");
        requireLatency(pump.unexpectedPeer == 1, "peer check happens before any control/audio/video routing");
        std::this_thread::sleep_for(std::chrono::milliseconds(65));
        UdpReceivePump::Datagram packet;
        requireLatency(!pump.pop(packet, std::chrono::milliseconds(0)) && pump.expired > 0,
          "expired video is dropped even if no newer packet arrives");
        send(sourceFd.get(), {0x00, 0x7f});
        requireLatency(pump.pop(packet, std::chrono::seconds(2)) && packet.bytes[1] == 0x7f,
          "fresh video resumes after overload");
        pump.stop(); // Must join without needing another network packet.
        pump.stop(); // Idempotent cleanup; socket remains owned by the caller.
        requireLatency(fcntl(receiveFd.get(), F_GETFD) != -1, "receive pump does not close the shared sender socket");
      }
      // Echo shares the actual media fd, bypassing a stopped video consumer.
      {
        auto bindEcho = [](int fd) {
          UdpEndpoint endpoint{}; endpoint.length = sizeof(sockaddr_in);
          auto* address = reinterpret_cast<sockaddr_in*>(&endpoint.address);
          address->sin_family = AF_INET; address->sin_addr.s_addr = htonl(INADDR_LOOPBACK);
          requireLatency(bind(fd, reinterpret_cast<sockaddr*>(address), endpoint.length) == 0, "bind echo");
          requireLatency(getsockname(fd, reinterpret_cast<sockaddr*>(address), &endpoint.length) == 0, "echo address");
          return endpoint;
        };
        ScopedFd media(socket(AF_INET, SOCK_DGRAM, 0)), host(socket(AF_INET, SOCK_DGRAM, 0));
        const auto local = bindEcho(media.get()), peer = bindEcho(host.get());
        const std::string token = "authenticated-same-media-socket-echo-test";
        gLiveLatency.echoUpdated = 0;
        UdpReceivePump pump(media.get(), true, &peer, 2, 50000, token);
        pollfd readable{host.get(), POLLIN, 0};
        requireLatency(poll(&readable, 1, 2000) == 1, "echo sent without a control sender");
        sanser::echo::Packet probe{};
        UdpEndpoint from{}; from.length = sizeof(from.address);
        requireLatency(recvfrom(host.get(), probe.data(), probe.size(), 0,
          reinterpret_cast<sockaddr*>(&from.address), &from.length) == static_cast<ssize_t>(probe.size()), "echo request size");
        requireLatency(sameUdpPeer(from, local), "echo uses the media port, not a second socket");
        sanser::echo::Responder responder(token);
        requireLatency(responder.accept(probe, steadyMicros()), "production echo responder authenticates request");
        for (int i = 0; i < 10; ++i) {
          const std::uint8_t video[] = {0, 1};
          sendto(host.get(), video, sizeof(video), 0, reinterpret_cast<const sockaddr*>(&local.address), local.length);
        }
        auto response = responder.response(0);
        auto invalid = response; invalid.back() ^= 1;
        sendto(host.get(), invalid.data(), invalid.size(), 0, reinterpret_cast<const sockaddr*>(&local.address), local.length);
        std::this_thread::sleep_for(std::chrono::milliseconds(20));
        requireLatency(gLiveLatency.echoUpdated == 0, "bad HMAC cannot publish a measurement");
        sendto(host.get(), response.data(), response.size(), 0, reinterpret_cast<const sockaddr*>(&local.address), local.length);
        const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(2);
        while (!gLiveLatency.echoUpdated && std::chrono::steady_clock::now() < deadline)
          std::this_thread::sleep_for(std::chrono::milliseconds(1));
        requireLatency(gLiveLatency.echoUpdated > 0 && gLiveLatency.echoRtt >= 0, "echo returns while video is never consumed");
        requireLatency(pump.dropped > 0 && pump.takeWindow().depth <= 2, "echo bypasses bounded full video queue");
        requireLatency(pump.takeEchoSample().has_value() && !pump.takeEchoSample(), "bounded latest echo report sample");
        pump.stop();
        gLiveLatency.echoUpdated = 0;
      }
      // Dedicated video UDP still carries unprefixed fragments.
      {
        ScopedFd fd(socket(AF_INET, SOCK_DGRAM, 0));
        sockaddr_in local{}; local.sin_family=AF_INET; local.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
        requireLatency(bind(fd.get(), reinterpret_cast<sockaddr*>(&local), sizeof(local)) == 0, "bind nonmultiplexed test");
        socklen_t size=sizeof(local); getsockname(fd.get(), reinterpret_cast<sockaddr*>(&local), &size);
        UdpReceivePump pump(fd.get(), false, nullptr);
        const char bytes[] = "SNU1";
        requireLatency(sendto(fd.get(), bytes, 4, 0, reinterpret_cast<sockaddr*>(&local), size) == 4, "send unprefixed video");
        UdpReceivePump::Datagram packet;
        requireLatency(pump.pop(packet, std::chrono::seconds(2)) && packet.size == 4 && packet.bytes[0] == 'S',
          "nonmultiplexed video keeps its first byte");
      }
      {
        UdpReceivePump broken(std::numeric_limits<int>::max(), false, nullptr);
        UdpReceivePump::Datagram packet;
        bool propagated = false;
        try { broken.pop(packet, std::chrono::seconds(2)); }
        catch (const std::runtime_error&) { propagated = true; }
        requireLatency(propagated, "socket failure wakes the video consumer instead of hanging");
      }
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
