#include "broadcast_policy.h"
#include "tx_scheduler.h"
#include <iostream>
#include <stdexcept>
using namespace sanser::desktop;
void check(bool ok,const char* why) { if(!ok) throw std::runtime_error(why); }
int main() {
  try {
    using P=SharingDeliveryPolicy;
    auto now=P::Clock::time_point{}+std::chrono::seconds(100);
    P policy, healthy; P::Sample sample;
    policy.observe(now,sample); healthy.observe(now,sample);
    for(unsigned i=0;i<16 && policy.state()==P::Active;++i) {
      now+=std::chrono::milliseconds(500);
      sample.submitted+=30; sample.replaced+=20; sample.feedbackFresh=true;
      sample.feedbackCongested=true; ++sample.feedbackSerial; ++sample.sentFrame;
      policy.observe(now,sample);
      auto good=sample; good.replaced=0; good.feedbackCongested=false;
      healthy.observe(now,good);
    }
    check(policy.state()==P::Paused && !policy.sendsVideo() && !policy.affectsBitrate(),"sustained pressure isolates this viewer");
    check(healthy.state()==P::Active && healthy.affectsBitrate(),"another viewer must remain independent");
    const auto pausedAt=now;
    sample.feedbackCongested=false;
    // Silence is never treated as restored bandwidth.
    now+=std::chrono::seconds(4); policy.observe(now,sample);
    check(policy.state()==P::Paused,"cooldown retains the paused state");
    now=pausedAt+std::chrono::seconds(5); policy.observe(now,sample);
    check(policy.state()==P::Probing && policy.sendsVideo() && !policy.affectsBitrate(),"recovery probe stays outside the shared bitrate minimum");
    for(unsigned i=0;i<6;++i) {
      now+=std::chrono::milliseconds(500); ++sample.feedbackSerial; sample.deliveredBytes=1000;
      policy.observe(now,sample);
    }
    check(policy.state()==P::Paused && policy.pauses()==2,"received fragments without a completed new frame cannot restore the viewer");
    const auto failedAt=now;
    now+=std::chrono::seconds(5); policy.observe(now,sample);
    check(policy.state()==P::Paused,"a failed probe increases cooldown to ten seconds");
    now=failedAt+std::chrono::seconds(10); policy.observe(now,sample);
    check(policy.state()==P::Probing,"viewer retains a later recovery opportunity");
    sample.completedFrame=sample.sentFrame+1;
    for(unsigned i=0;i<3;++i) {
      now+=std::chrono::milliseconds(500); ++sample.feedbackSerial;
      policy.observe(now,sample);
    }
    check(policy.state()==P::Active && policy.affectsBitrate(),"only proven frame progress and three clean feedback windows restore quality participation");
    P backoff; P::Sample retries; backoff.observe(now,retries);
    for(unsigned i=0;i<18;++i) { now+=std::chrono::milliseconds(500); retries.backoffMs=2000; backoff.observe(now,retries); }
    check(backoff.state()==P::Paused,"persistent maximum IDR backoff isolates a viewer even without queue replacements");
    P gap; P::Sample pressure; gap.observe(now,pressure);
    now+=std::chrono::milliseconds(500); pressure.submitted=30; pressure.referenceDrops=30; gap.observe(now,pressure);
    now+=std::chrono::seconds(20); pressure.submitted+=30; pressure.referenceDrops+=30; gap.observe(now,pressure);
    check(gap.state()==P::Active,"a suspended host clock/sample gap is not continuous pressure");
    TxScheduler tx;
    check(tx.push(TxScheduler::Video,{{1},now,1}),"queue fresh video");
    check(tx.push(TxScheduler::Repair,{{2},now,1}),"queue repair");
    check(tx.push(TxScheduler::Control,{{3},now,0}),"queue control");
    tx.clearRealtimeVideo();
    auto control=tx.pop(true),repair=tx.pop(true);
    check(control && control->bytes==std::vector<std::uint8_t>{3} && repair && repair->repair && !tx.pop(true),"pausing clears fresh video only; priority control and repair survive");
    std::cout<<"Sharing health: sustained pressure, bounded probes, completion proof, cooldown, isolation and preserved control/repair passed\n";
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
