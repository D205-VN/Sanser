#pragma once
#include <chrono>
#include <cstdint>

namespace sanser::desktop {
// A shared encoder cannot produce an IDR for just one viewer. Bound each
// viewer's contribution instead: first recovery is immediate, persistent
// failures back off, and an independent viewer retains its own fast recovery.
// Access is serialized by Delivery::mutex, including observations for stats.
class SharingKeyframeRecovery {
public:
  using Clock = std::chrono::steady_clock;
  void request(Clock::time_point now) {
    if (!hasSignal_ || now-lastSignal_ >= std::chrono::seconds(3)) {
      attempts_=0;
      nextAllowed_=now;
    }
    hasSignal_=true;
    lastSignal_=now;
    pending_=true;
    ++signals_;
  }
  bool take(Clock::time_point now) {
    if (!pending_ || now<nextAllowed_) return false;
    pending_=false;
    nextAllowed_=now+std::chrono::milliseconds(attempts_==0 ? 500 : attempts_==1 ? 1000 : 2000);
    if (attempts_<2) ++attempts_;
    ++requests_;
    return true;
  }
  // A newly published IDR satisfies older demands. Keep the backoff history:
  // enqueueing an IDR is not proof that the viewer received/decoded it.
  void keyframe() { pending_=false; }
  std::uint64_t signals() const { return signals_; }
  std::uint64_t requests() const { return requests_; }
private:
  bool pending_=false, hasSignal_=false;
  unsigned attempts_=0;
  Clock::time_point nextAllowed_{}, lastSignal_{};
  std::uint64_t signals_=0, requests_=0;
};
}
