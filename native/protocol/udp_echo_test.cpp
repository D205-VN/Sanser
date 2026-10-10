#include "../common/udp_echo.h"
#include <iostream>
#include <stdexcept>

static void check(bool condition, const char* message) {
  if (!condition) throw std::runtime_error(message);
}
int main() {
  using namespace sanser::echo;
  try {
    const std::string_view token = "test-session-token-for-echo-measurement";
    Client client(token, 123);
    Responder host(token);
    auto request = client.prepare(1000000);
    check(request.has_value(), "initial probe");
    client.sending(1000010); client.sent(true, 5);
    check(!client.prepare(1100000), "client bounded to 1 Hz");
    check(host.accept(*request, 900000000), "request authenticated across unrelated clock epochs");
    auto reply = host.response(100);
    auto sample = client.receive(reply, 1014010);
    check(sample && sample->rttMs == 14 && sample->hostHoldMs == 0.1 && sample->residualMs == 13.9,
          "same-clock RTT and responder residence");
    check(!client.receive(reply, 1014020), "response replay rejected");
    check(!host.accept(*request, 900300000), "request replay rejected");
    Codec codec(token), wrong("different-session-token");
    check(!wrong.valid(*request, false), "session binding");
    check(!codec.valid(reply, false), "direction binding");
    auto tampered = *request; tampered[8] ^= 1;
    check(!codec.valid(tampered, false), "tampered request");
    check(!codec.valid(std::span(*request).first(kSize - 1), false), "short packet");
    check(!Codec("").valid(*request, false), "no unauthenticated echo service");
    request = client.prepare(2000000); client.sending(2000010); client.sent(true, 0);
    check(!client.receive(codec.encode(true, 999, 2), 2010010), "nonce mismatch");
    check(!client.receive(codec.encode(true, 123, 99), 2010010), "unknown sequence");
    check(!client.receive(codec.encode(true, 123, 2, 20000), 2010010), "impossible responder duration");
    request = client.prepare(3000000); client.sending(3000010); client.sent(true, 0);
    check(!client.receive(codec.encode(true, 123, 3), 6100010), "stale response");
    request = client.prepare(4000000); client.sending(4000010); client.sent(false, 0);
    check(!client.receive(codec.encode(true, 123, 4), 4010010), "failed send cannot produce RTT");
    Responder limiter(token);
    check(limiter.accept(codec.encode(false, 555, 1), 1000000), "first limited request");
    check(!limiter.accept(codec.encode(false, 555, 2), 1010000), "host rate limit");
    check(limiter.accept(codec.encode(false, 555, 2), 1300000), "host recovers after rate limit");
    check(!limiter.accept(codec.encode(false, 556, 3), 1600000), "run nonce pinned");
    std::cout << "authenticated UDP echo tests passed\n";
    return 0;
  } catch (const std::exception& e) { std::cerr << e.what() << '\n'; return 1; }
}
