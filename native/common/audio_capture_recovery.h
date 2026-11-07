#pragma once
#include <algorithm>
#include <atomic>
#include <chrono>
#include <exception>

namespace sanser::audio {
// Keep the transport outside this loop: recreating it would reset authenticated
// sequence numbers on the same key. Only the device/capture session is retried.
template<class Capture,class Wait,class Report>
void captureWithRecovery(const std::atomic<bool>& running,Capture capture,Wait wait,Report report) {
  unsigned failures=0;
  while(running.load()) {
    const auto began=std::chrono::steady_clock::now();
    try { capture(); failures=0; }
    catch(const std::exception& error) {
      if(!running.load()) break;
      if(std::chrono::steady_clock::now()-began>=std::chrono::seconds(5)) failures=0;
      failures=std::min(14U,failures+1);
      const auto delay=std::chrono::milliseconds(std::min(2000U,150U*failures));
      report(error,failures,delay);
      wait(delay);
    }
  }
}
}
