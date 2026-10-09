#include "control_timing.h"
#include <cmath>
#include <iostream>
#include <stdexcept>

void require(bool ok, const char* message) {
  if (!ok) throw std::runtime_error(message);
}
int main() {
  try {
    // Mac epoch ~1 second, host epoch ~900 seconds. Application RTT=180ms,
    // actual residual=12ms; never subtract timestamps from different machines.
    sanser::ControlTiming t{1000000, 1020000, 900000000, 900120000,
                           900130000, 1162000, 1180000};
    require(t.calculate(), "valid timeline across unrelated clock epochs");
    require(t.appMs == 180 && t.socketMs == 142 && t.residualMs == 12,
            "application queues must not be attributed to the wire");
    require(t.sendQueueMs == 20 && t.hostQueueMs == 120 &&
            t.hostWorkMs == 10 && t.receiveQueueMs == 18, "queue decomposition");
    require(t.appMs == t.sendQueueMs+t.hostQueueMs+t.hostWorkMs+t.residualMs+t.receiveQueueMs,
            "durations reconcile exactly");
    auto bad=t; bad.t4=bad.t2+200000;
    require(!bad.calculate(), "reject impossible host duration, do not clamp");
    bad=t; bad.t5=bad.t1-1; require(!bad.calculate(), "no unsigned underflow");
    bad=t; bad.t6=bad.t0+10000001; require(!bad.calculate(), "reject stale sample");
    sanser::ControlTimingTracker tracker;
    const auto first=tracker.prepare(t.t0,false), retry=tracker.prepare(t.t0,true);
    tracker.sent(first,t.t1); tracker.sent(retry,t.t1+1000);
    require(!tracker.received(first,t.t5,t.t6), "wait for authenticated send receipt");
    const auto complete=tracker.receipt(first,t.t2,t.t3,t.t4);
    require(complete && complete->residualMs == 12, "correct original attempt");
    require(!tracker.receipt(first,t.t2,t.t3,t.t4), "completed sample cannot repeat");
    require(!tracker.receipt(retry,t.t2,t.t3,t.t4), "receipt can precede reply");
    const auto retried=tracker.received(retry,t.t5+1000,t.t6+1000);
    require(retried && retried->input && retried->residualMs == 12,
            "retry uses its own send timestamp");
    const auto lost=tracker.prepare(t.t0,false);
    for (int i=0;i<128;++i) tracker.prepare(t.t0,false);
    tracker.sent(lost,t.t1);
    require(!tracker.receipt(lost,t.t2,t.t3,t.t4) && !tracker.received(lost,t.t5,t.t6),
            "missing receipts are bounded, never fabricated");
    require(!tracker.received(999999,t.t5,t.t6), "unknown probe rejected");
    std::cout << "control timing tests passed\n";
    return 0;
  } catch (const std::exception& error) { std::cerr << error.what() << '\n'; return 1; }
}
