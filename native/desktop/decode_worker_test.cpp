#include "decode_worker.h"
#include <iostream>
#include <stdexcept>

using namespace sanser::desktop;
void check(bool ok,const char* message) { if(!ok) throw std::runtime_error(message); }
int main() {
  try {
    InputDeliveryPolicy policy;
    check(policy.deadline()>=std::chrono::seconds(2),"first input must allow a WAN ACK rather than expiring after 120 ms");
    policy.acknowledged(std::chrono::milliseconds(350));
    check(policy.retry(true,1)>=std::chrono::milliseconds(350),"retry pacing adapts to observed ACK latency");
    check(policy.deadline()>std::chrono::milliseconds(350) && policy.deadline()<=std::chrono::seconds(5),"input remains bounded without killing a normal slow path");
    policy.acknowledged(std::chrono::seconds(10));
    check(policy.deadline()==std::chrono::seconds(5),"blackholed input retains a finite fail-closed deadline");

    std::mutex mutex; std::condition_variable wake;
    bool entered=false,release=false;
    std::vector<std::uint8_t> decoded;
    std::thread::id owner;
    std::atomic<unsigned> resets{0},requests{0};
    DecodeWorker worker([&](Frame frame) {
      std::unique_lock lock(mutex);
      if(owner==std::thread::id{}) owner=std::this_thread::get_id();
      check(owner==std::this_thread::get_id(),"decoder must remain on its owning thread");
      decoded.push_back(frame.data[0]);
      if(frame.data[0]==1) {
        entered=true; wake.notify_all();
        wake.wait_for(lock,std::chrono::seconds(3),[&] { return release; });
      }
      wake.notify_all();
    },[&] { check(owner==std::thread::id{} || owner==std::this_thread::get_id(),"codec cleanup must run on the decode thread"); ++resets; },[&] { ++requests; });
    worker.submit({{1},640,360,true});
    {
      std::unique_lock lock(mutex);
      check(wake.wait_for(lock,std::chrono::seconds(1),[&] { return entered; }),"decode worker did not begin");
    }
    const auto began=Clock::now();
    for(unsigned i=2;i<12;++i) worker.submit({{static_cast<std::uint8_t>(i)},640,360,false});
    check(Clock::now()-began<std::chrono::milliseconds(100),"socket callback must not wait for a blocked decoder");
    check(worker.dropped()>0 && requests>0,"decode pressure discards dependencies and requests an IDR");
    worker.submit({{99},640,360,true});
    {
      std::unique_lock lock(mutex); release=true; wake.notify_all();
      check(wake.wait_for(lock,std::chrono::seconds(1),[&] { return decoded.size()>=2; }),"fresh IDR did not resume decoding");
      check(decoded==std::vector<std::uint8_t>({1,99}),"overflowed P-frames must not be replayed as a stale backlog");
    }
    worker.stop();
    check(resets>=2,"overflow and shutdown clean up the decoder on its owning thread");
    std::cout<<"Decode worker: bounded reference-safe queue, non-blocking socket callback and codec thread ownership passed\n";
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
