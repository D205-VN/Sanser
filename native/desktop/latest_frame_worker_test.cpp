#include "latest_frame_worker.h"
#include <chrono>
#include <iostream>
#include <stdexcept>
#include <vector>
using namespace sanser::desktop;
int main() {
  try {
    std::mutex mutex; std::condition_variable wake; bool blocked=false,release=false; std::vector<unsigned> converted;
    LatestFrameWorker<unsigned> worker([&](unsigned frame) {
      std::unique_lock lock(mutex);
      if(frame==0) { blocked=true; wake.notify_all(); wake.wait(lock,[&] { return release; }); }
      converted.push_back(frame); wake.notify_all();
    });
    // Always release the worker if a test assertion throws.
    struct Release { std::mutex& mutex; std::condition_variable& wake; bool& release; ~Release() { { std::lock_guard lock(mutex); release=true; } wake.notify_all(); } } cleanup{mutex,wake,release};
    worker.submit(0);
    { std::unique_lock lock(mutex); if(!wake.wait_for(lock,std::chrono::seconds(2),[&] { return blocked; })) throw std::runtime_error("conversion did not start"); }
    const auto start=std::chrono::steady_clock::now();
    for(unsigned i=1;i<=100;++i) worker.submit(i);
    if(std::chrono::steady_clock::now()-start>std::chrono::milliseconds(100)) throw std::runtime_error("slow conversion blocked decoder submission");
    { std::lock_guard lock(mutex); release=true; } wake.notify_all();
    { std::unique_lock lock(mutex); if(!wake.wait_for(lock,std::chrono::seconds(2),[&] { return converted.size()==2; }) || converted!=std::vector<unsigned>{0,100}) throw std::runtime_error("decoded picture backlog did not keep only the newest picture"); }
    worker.stop();
    std::cout<<"Latest decoded frame: stalled conversion never blocks the decoder or replays old pictures\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
