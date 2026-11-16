#pragma once
#include <chrono>

namespace sanser::voice {
// Decide whether a captured microphone block is too far behind to send.
//
// Capture timestamps are the first sample's device time, so every block is
// already "old" by the device's input latency (often 20-50 ms on a Mac).
// An absolute 40 ms limit discarded ~10 blocks per second of normal speech,
// and each discard reset the encoder and echo canceller: heard as crackle.
// Only skip a block when the worker has fallen behind newer captures, or the
// timestamp is clearly invalid.
template<class TimePoint>
bool staleCapture(TimePoint at,TimePoint newest,TimePoint now) {
  using namespace std::chrono_literals;
  return newest-at>80ms || now-at>1s;
}
}
