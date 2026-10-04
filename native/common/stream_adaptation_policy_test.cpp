#include "stream_adaptation_policy.h"

#include <cmath>
#include <cstdlib>
#include <iostream>

static void require(bool condition, const char* message) {
  if (!condition) { std::cerr << message << '\n'; std::exit(1); }
}

int main() {
  sanser::StreamAdaptationPolicy policy;
  double mbps = 25.0;
  unsigned fps = 60;
  // Jitter-only windows supply no loss/failed repair; quality stays intact.
  for (unsigned w = 1; w <= 10; ++w) {
    mbps *= policy.networkBitrateFactor(w, 0, false);
    require(!policy.reducedIn(w), "jitter-only window reduced quality");
  }
  require(mbps == 25 && fps == 60, "jitter-only trace must stay 25/60");
  mbps *= policy.networkBitrateFactor(11, 2, false);
  mbps *= policy.networkBitrateFactor(11, 2, true);
  require(mbps == 25, "duplicate reports must not establish sustained loss");
  mbps *= policy.networkBitrateFactor(12, 2, false);
  mbps *= policy.networkBitrateFactor(12, 5, true, true);
  require(std::abs(mbps - 22) < 0.001 && fps == 60, "first sustained loss step must be 22/60");
  mbps *= policy.networkBitrateFactor(13, 2, true);
  require(std::abs(mbps - 19.36) < 0.001 && fps == 60, "second loss step must preserve FPS");
  require(policy.networkBitrateFactor(15, 5, true) == 1, "missing windows must break loss streak");
  require(policy.networkBitrateFactor(14, 5, true) == 1, "stale window must be ignored");
  require(policy.networkBitrateFactor(16, 5, true) == 0.84, "severe sustained loss step");
  require(policy.reducedIn(16), "prevent recovery in reduction window");
  require(policy.networkBitrateFactor(17, 0, false) == 1, "clear window must not reduce");
  require(policy.networkBitrateFactor(18, 2, true) == 1, "clear window must break loss streak");
  fps = static_cast<unsigned>(std::floor(fps * policy.processingFpsFactor(19, true)));
  fps = static_cast<unsigned>(std::floor(fps * policy.processingFpsFactor(19, true)));
  require(fps == 51, "host and client must not reduce FPS twice in one window");
  require(policy.networkBitrateFactor(19, 0, false) == 1, "processing pressure must not cut bitrate");
  sanser::StreamAdaptationPolicy sender;
  require(sender.networkBitrateFactor(1, 0, false, true) == 1, "isolated slow send must not reduce");
  require(sender.networkBitrateFactor(2, 0, false, true) == 0.88, "sustained slow send reduces bitrate");
}
