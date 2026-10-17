#include "decode_worker.h"
#include <iostream>
#include <stdexcept>

using namespace sanser::desktop;
void check(bool ok,const char* message) { if(!ok) throw std::runtime_error(message); }
int main() {
  try {
    InputDeliveryPolicy policy;
    check(policy.repairRetry()==std::chrono::milliseconds(100),"video startup has a conservative retry interval before any input ACK is available");
    check(policy.deadline()>=std::chrono::seconds(2),"first input must allow a WAN ACK rather than expiring after 120 ms");
    policy.acknowledged(std::chrono::milliseconds(350));
    check(policy.repairRetry()==std::chrono::milliseconds(200),"video repair retries follow slow ACK delivery with a finite bound");
    check(policy.retry(true,1)>=std::chrono::milliseconds(350),"retry pacing adapts to observed ACK latency");
    check(policy.deadline()>std::chrono::milliseconds(350) && policy.deadline()<=std::chrono::seconds(5),"input remains bounded without killing a normal slow path");
    policy.acknowledged(std::chrono::seconds(10));
    check(policy.deadline()==std::chrono::seconds(5),"blackholed input retains a finite fail-closed deadline");

    {
      std::mutex mutex; std::condition_variable wake; bool entered=false,release=false;
      unsigned decoded=0; std::atomic<unsigned> requests{0};
      DecodeWorker cold([&](Frame) {
        std::unique_lock lock(mutex);
        if(!entered) { entered=true; wake.notify_all(); wake.wait_for(lock,std::chrono::seconds(3),[&] { return release; }); }
        ++decoded; wake.notify_all();
      },[] {},[&] { ++requests; },true,false,[] { return Clock::time_point{}; });
      cold.submit({{1},640,360,true});
      { std::unique_lock lock(mutex); check(wake.wait_for(lock,std::chrono::seconds(1),[&] { return entered; }),"cold decoder did not enter initialization"); }
      for(unsigned i=0;i<20;++i) cold.submit({{2},640,360,false});
      check(cold.dropped()==0 && requests==0,"cold startup discarded the only IDR/reference chain");
      { std::unique_lock lock(mutex); release=true; wake.notify_all(); check(wake.wait_for(lock,std::chrono::seconds(1),[&] { return decoded==21; }),"cold decoder did not catch up its bounded reference chain"); }
      cold.stop();
      const auto timing=cold.takeTiming();
      check(timing.samples==21,"queue/decode timing counts completed worker calls, not incoming fragments");
      check(cold.takeTiming().samples==0,"periodic timing snapshots must not replay old measurements");
    }

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

    // Below capacity, a fresh IDR must replace an older queued chain too.
    struct Case { bool expired,cold,ultra; int age; };
    for(const auto config : {Case{false,false,true,0},Case{true,false,true,51},Case{true,false,false,101},Case{true,true,true,501}}) {
      const bool expired=config.expired;
      std::mutex gate; std::condition_variable ready; bool busy=false,allow=false;
      std::vector<unsigned> frames; std::atomic<unsigned> idrs{0},cleanups{0}; std::atomic<int> timeMs{0};
      DecodeWorker latest([&](Frame frame) {
        std::unique_lock lock(gate); frames.push_back(frame.data[0]);
        if(frame.data[0]==1) { busy=true; ready.notify_all(); ready.wait_for(lock,std::chrono::seconds(3),[&] { return allow; }); }
        ready.notify_all();
      },[&] { ++cleanups; },[&] { ++idrs; ready.notify_all(); },config.cold,config.ultra,[&] { return Clock::time_point(std::chrono::milliseconds(timeMs.load())); });
      struct Release { std::mutex& gate; std::condition_variable& ready; bool& allow;
        ~Release() { { std::lock_guard lock(gate); allow=true; } ready.notify_all(); }
      } releaseGate{gate,ready,allow};
      latest.submit({{1},640,360,true});
      { std::unique_lock lock(gate); check(ready.wait_for(lock,std::chrono::seconds(2),[&] { return busy; }),"decoder test did not block"); }
      latest.submit({{2},640,360,false}); latest.submit({{3},640,360,false});
      if(expired) timeMs=config.age;
      else latest.submit({{99},640,360,true});
      { std::lock_guard lock(gate); allow=true; } ready.notify_all();
      if(expired) {
        const auto until=Clock::now()+std::chrono::seconds(2);
        while(idrs==0 && Clock::now()<until) std::this_thread::sleep_for(std::chrono::milliseconds(1));
        check(idrs==1 && latest.expired()==2,"stale dependent pictures must expire even below the queue capacity");
        latest.submit({{4},640,360,false});
        check(idrs==1,"waiting frames must not create a keyframe request storm");
        latest.submit({{99},640,360,true});
      }
      { std::unique_lock lock(gate); check(ready.wait_for(lock,std::chrono::seconds(2),[&] { return frames.size()==2; }),"fresh reference failed to resume video"); }
      check(expired || cleanups==0,"fresh IDR must not recreate a healthy hardware decoder");
      latest.stop();
      check(frames==std::vector<unsigned>({1,99}),"stale P-frames must not play before a newer IDR");
    }
    {
      DecodeWorker cleanupFailure([](Frame) {},[] { throw std::runtime_error("synthetic cleanup failure"); },[] {});
      cleanupFailure.stop();
      check(cleanupFailure.failures()>=10 && cleanupFailure.error().find("synthetic cleanup failure")!=std::string::npos,
        "cleanup exception must become a bounded terminal decoder error, not terminate the process");
      DecodeWorker requestFailure([](Frame) {},[] {},[] { throw std::runtime_error("synthetic request failure"); });
      requestFailure.submit({{2},640,360,false}); requestFailure.stop();
      check(requestFailure.failures()>=10,"recovery callback exception must not escape into the UDP reader");
    }
    {
      std::mutex gate; std::condition_variable ready; bool busy=false,allow=false;
      DecodeWorker concurrentFailure([&](Frame) {
        std::unique_lock lock(gate); busy=true; ready.notify_all();
        ready.wait_for(lock,std::chrono::seconds(3),[&] { return allow; });
      },[] {},[] { throw std::runtime_error("synthetic concurrent recovery failure"); });
      concurrentFailure.submit({{1},640,360,true});
      { std::unique_lock lock(gate); check(ready.wait_for(lock,std::chrono::seconds(2),[&] { return busy; }),"concurrent decoder test did not begin"); }
      for(unsigned i=0;i<4;++i) concurrentFailure.submit({{2},640,360,false});
      { std::lock_guard lock(gate); allow=true; } ready.notify_all();
      concurrentFailure.stop();
      check(concurrentFailure.failures()>=10,"a completed in-flight decode must not clear a concurrent terminal recovery error");
    }
    std::cout<<"Decode worker: bounded reference-safe queue, non-blocking socket callback and codec thread ownership passed\n";
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
