#include "audio_capture_recovery.h"
#include <iostream>
#include <stdexcept>
#include <vector>

int main() {
  using namespace std::chrono;
  std::atomic<bool> running{true};
  unsigned captures=0,reports=0,transportSequence=700;
  std::vector<milliseconds> delays;
  sanser::audio::captureWithRecovery(running,[&] {
    ++captures;
    if(captures<=20) throw std::runtime_error("temporary device/driver failure");
    // Simulate recovered capture sending on the original transport, not
    // restarting its authentication nonce at sequence 1.
    ++transportSequence;
    running=false;
  },[&](milliseconds delay) { delays.push_back(delay); },
  [&](const std::exception& error,unsigned attempt,milliseconds delay) {
    if(std::string(error.what())!="temporary device/driver failure" || attempt>14 || delay>seconds(2))
      throw std::runtime_error("Recovery lost failure information or retry bound");
    ++reports;
  });
  if(captures!=21 || reports!=20 || transportSequence!=701 || delays.front()!=milliseconds(150) || delays.back()!=seconds(2))
    throw std::runtime_error("Capture did not resume on its existing transport after repeated errors");
  captures=reports=0;running=true;
  sanser::audio::captureWithRecovery(running,[&] { ++captures; throw std::runtime_error("device removed"); },
    [&](milliseconds) { running=false; },[&](const std::exception&,unsigned,milliseconds) { ++reports; });
  if(captures!=1 || reports!=1) throw std::runtime_error("Cancellation restarted capture");
  running=false;
  sanser::audio::captureWithRecovery(running,[&] { ++captures; },[](milliseconds) {},[](const std::exception&,unsigned,milliseconds) {});
  if(captures!=1) throw std::runtime_error("Disabled capture started a device");
  std::cout<<"Audio capture recovery: repeated driver failures, bounded backoff, existing transport and cancellation passed\n";
}
