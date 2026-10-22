#include "broadcast.h"
#include "tx_scheduler.h"
#include <array>
#include <cmath>
#include <iostream>
#include <random>
#include <stdexcept>
using namespace sanser::desktop;
namespace v=sanser::voice;
void require(bool ok,const char* why) { if(!ok) throw std::runtime_error(why); }
template<class F> bool waitFor(F f) {
  const auto end=Clock::now()+std::chrono::seconds(4);
  while(Clock::now()<end) { if(f()) return true; std::this_thread::sleep_for(std::chrono::milliseconds(2)); }
  return f();
}
v::Packet packet(v::Encoder& encoder,Clock::time_point start,unsigned speaker=0) {
  for(unsigned block=0;block<4;++block) {
    AudioSamples pcm{};
    for(unsigned i=0;i<audioFrames;++i) {
      const auto value=static_cast<std::int16_t>(12000*std::sin((block*audioFrames+i)*0.06));
      pcm[2*i]=pcm[2*i+1]=value;
    }
    const auto encoded=encoder.push(pcm,start+std::chrono::milliseconds(5*block));
    if(encoded) { auto out=*encoded; out.speaker=speaker; return out; }
  }
  throw std::runtime_error("20ms encoder did not emit a packet");
}
struct Receiver {
  std::mutex mutex;
  std::array<unsigned,4> received{};
  std::unique_ptr<Peer> peer;
  ~Receiver() { if(peer) peer->stop(); }
  void receive(v::Packet p) { std::lock_guard lock(mutex); ++received.at(p.speaker); }
  unsigned count(unsigned speaker) { std::lock_guard lock(mutex); return received.at(speaker); }
  unsigned total() { std::lock_guard lock(mutex); unsigned sum=0; for(auto n:received) sum+=n; return sum; }
};
void connect(Broadcaster& group,Receiver& receiver,const Options& source,unsigned index,bool voice=true) {
  std::random_device random;
  for(unsigned attempt=0;attempt<20 && !receiver.peer;++attempt) {
    const auto port=static_cast<std::uint16_t>(25000+random()%25000);
    auto options=source; options.host=false; options.port=port+1; options.peer="127.0.0.1:"+std::to_string(port);
    options.token=std::string(48,static_cast<char>('a'+index)); options.groupVoice=voice;
    const auto id="viewer"+std::to_string(index);
    try {
      auto peer=std::make_unique<Peer>(options);
      peer->onVoice=[&receiver](v::Packet p) { receiver.receive(std::move(p)); };
      group.add({id,"127.0.0.1:"+std::to_string(port+1),port,options.token,source.bitrate});
      receiver.peer=std::move(peer); receiver.peer->start();
    } catch(...) { group.remove(id); }
  }
  require(receiver.peer && waitFor([&] { return receiver.peer->ready(); }),"authenticated group handshake");
  if(voice) require(waitFor([&] { return receiver.peer->voiceSpeaker()<4; }),"host assigns speaker slot");
}
int main() {
  try {
    require(!v::command("voice-config\t1\t1\t1\textra"),"reject malformed consent IPC");
    require(!v::command("voice-config\t1\t2\t0"),"reject nonboolean consent");
    const auto left=v::command("voice-config\t0\t1\t1"), deaf=v::command("voice-config\t1\t1\t1");
    require(left && !left->microphone && !left->deafened,"leave always closes microphone");
    require(deaf && deaf->joined && !deaf->microphone,"deafen always mutes microphone");
    const auto t=Clock::time_point{}+std::chrono::seconds(100);
    v::Encoder encoder;
    const auto first=packet(encoder,t);
    const auto bytes=v::pack(first);
    require(bytes.size()<417 && v::unpack(bytes)->opus==first.opus,"bounded Opus20 voice wire roundtrip");
    auto corrupt=bytes; corrupt[4]=4;
    require(!v::unpack(corrupt) && !v::unpack(Bytes{'S','V','O','1'}),"reject invalid source and truncated body");
    corrupt=bytes; for(unsigned i=9;i<17;++i) corrupt[i]=255;
    require(!v::unpack(corrupt),"reject sequence overflow");
    v::Mixer mixer;
    require(mixer.receive(first,t) && !mixer.poll(t+std::chrono::milliseconds(19)),"small independent speaker jitter reserve");
    require(mixer.poll(t+std::chrono::milliseconds(20)).has_value(),"decode voice after reserve");
    require(!mixer.receive(first,t+std::chrono::milliseconds(21)),"duplicate speech rejected");
    for(unsigned i=1;i<4;++i) require(mixer.poll(t+std::chrono::milliseconds(20+5*i)).has_value(),"20ms is delivered as four5ms blocks");
    // Sequence2 is lost; sequence3 can repair/conceal it without waiting on video.
    (void)packet(encoder,t+std::chrono::milliseconds(20));
    const auto third=packet(encoder,t+std::chrono::milliseconds(40));
    require(mixer.receive(third,t+std::chrono::milliseconds(39)),"out-of-order next packet accepted");
    require(mixer.poll(t+std::chrono::milliseconds(40)).has_value() && mixer.stats().fecAttempts==1,"attempt Opus FEC for isolated loss");
    for(unsigned i=1;i<8;++i) (void)mixer.poll(t+std::chrono::milliseconds(40+5*i));
    require(mixer.stats().decoded==2,"decode next speech packet after gap");
    mixer.mute(0,true);
    require(!mixer.receive(packet(encoder,t+std::chrono::milliseconds(60)),t),"per-source local mute rejects queued voice");
    mixer.mute(0,false); encoder.reset();
    const auto replacement=packet(encoder,t+std::chrono::milliseconds(80));
    require(mixer.receive(replacement,t+std::chrono::milliseconds(80)) && !mixer.receive(first,t+std::chrono::milliseconds(81)),"new mic epoch rejects delayed prior speech");
    v::Mixer bounded;
    v::Encoder burst;
    for(unsigned i=0;i<40;++i) require(bounded.receive(packet(burst,t+std::chrono::milliseconds(i*20)),t),"bounded burst accepted");
    require(bounded.stats().dropped>=34,"voice queue sheds stale speech rather than accumulating latency");
    require(!bounded.poll(t+std::chrono::milliseconds(100)),"expired packets do not play");
    // A 7ms worker cadence (5ms wait plus codec/device work) must still deliver
    // a 48kHz stream. One poll per wake silently produced only ~34kHz of PCM,
    // with recurring output underruns and stale speech dropped by the mixer.
    v::Mixer scheduled; v::Encoder paced;
    unsigned nextPacket=0,delivered=0;
    for(unsigned ms=0;ms<=2002;ms+=7) {
      while(nextPacket<=ms && nextPacket<2000) {
        require(scheduled.receive(packet(paced,t+std::chrono::milliseconds(nextPacket)),t+std::chrono::milliseconds(nextPacket)),"scheduled speech received");
        nextPacket+=20;
      }
      delivered+=static_cast<unsigned>(scheduled.drain(t+std::chrono::milliseconds(ms)).size());
    }
    require(delivered>=395 && scheduled.stats().dropped==0,"elapsed voice blocks catch up without reducing the sample rate or dropping speech");
    // Independent talkers arrive at different phases. They still share one
    // 48kHz device: adding speakers must not add output blocks or speed up PCM.
    for(unsigned talkers=2;talkers<=3;++talkers) {
      v::Mixer room; std::array<v::Encoder,3> talker;
      std::array<unsigned,3> next{0,6,13}; unsigned blocks=0,maxBurst=0;
      for(unsigned ms=0;ms<=2002;ms+=7) {
        for(unsigned speaker=0;speaker<talkers;++speaker) {
          while(next[speaker]<=ms && next[speaker]<2000) {
            const auto at=t+std::chrono::milliseconds(next[speaker]);
            require(room.receive(packet(talker[speaker],at,speaker),at),"staggered group speech received");
            next[speaker]+=20;
          }
        }
        const auto output=room.drain(t+std::chrono::milliseconds(ms));
        blocks+=static_cast<unsigned>(output.size()); maxBurst=std::max(maxBurst,static_cast<unsigned>(output.size()));
      }
      std::cout<<"Staggered voice talkers="<<talkers<<" output5msBlocks="<<blocks<<" maxWorkerBurst="<<maxBurst<<'\n';
      require(blocks>=395 && blocks<=400 && maxBurst<=2,"two/three talkers must share one playback clock, not multiply PCM output");
      require(room.stats().decoded>=talkers*96 && room.stats().dropped==0,"all staggered talkers decode without stale drops");
    }
    v::Mixer mixed;
    for(unsigned i=0;i<4;++i) { auto p=first; p.speaker=i; require(mixed.receive(p,t),"independent speakers accepted"); }
    const auto sum=mixed.poll(t+std::chrono::milliseconds(20));
    require(sum && mixed.stats().decoded==4,"four independent Opus decoders mixed");
    for(auto sample:*sum) require(sample>-32768 && sample<32767,"mix limits loud talkers without integer wrap or hard clipping");
    TxScheduler schedule;
    for(unsigned i=0;i<30;++i) {
      require(schedule.push(TxScheduler::Audio,{Bytes{3},t}),"system audio enqueue");
      require(schedule.push(TxScheduler::Voice,{Bytes{2},t}),"voice enqueue");
    }
    require(schedule.push(TxScheduler::Video,{Bytes{4},t}) && schedule.push(TxScheduler::Control,{Bytes{1},t}),"video and control enqueue");
    require(schedule.pop(true)->bytes[0]==1,"control outranks voice");
    for(unsigned i=0;i<8;++i) require(schedule.pop(false)->bytes[0]==2,"voice independent queue precedes screen sound");
    for(unsigned i=0;i<8;++i) require(schedule.pop(false)->bytes[0]==3,"bounded sound survives voice saturation");
    require(!schedule.pop(false) && schedule.pop(true)->bytes[0]==4,"video pacing does not block voice");

    Options source; source.host=true; source.input=false; source.audio=false; source.groupVoice=true; source.bitrate=20000000;
    Receiver host;
    Broadcaster group(source);
    group.onVoice=[&](v::Packet p) { host.receive(std::move(p)); };
    std::array<Receiver,3> viewers;
    for(unsigned i=0;i<3;++i) connect(group,viewers[i],source,i);
    std::array<v::Encoder,4> speakers;
    for(unsigned i=0;i<3;++i) viewers[i].peer->voicePacket(packet(speakers[i],Clock::now()));
    std::this_thread::sleep_for(std::chrono::milliseconds(100));
    require(host.total()==0 && viewers[0].total()==0,"microphone is off before explicit consent");
    for(auto& viewer:viewers) viewer.peer->setVoiceState({true,true,false});
    group.setVoiceState({true,true,false});
    std::this_thread::sleep_for(std::chrono::milliseconds(100));
    group.setCapturePaused(true); // Voice must continue even when screen capture pauses.
    for(unsigned turn=0;turn<12;++turn) {
      for(unsigned i=0;i<3;++i) viewers[i].peer->voicePacket(packet(speakers[i],Clock::now()));
      group.voicePacket(packet(speakers[3],Clock::now()));
      std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
    require(waitFor([&] { return host.total()>=30; }),"host hears three viewer microphones");
    for(unsigned i=0;i<3;++i) {
      const auto own=viewers[i].peer->voiceSpeaker();
      require(host.count(own)>=10 && host.count(0)==0,"viewer packets use only their authenticated assigned speaker ID");
      require(waitFor([&] { return viewers[i].total()>=30; }),"every viewer hears host and other viewers");
      require(viewers[i].count(own)==0 && viewers[i].count(0)>0,"SFU never echoes a viewer's microphone to itself");
    }
    viewers[0].peer->setVoiceState({true,false,true});
    std::this_thread::sleep_for(std::chrono::milliseconds(100));
    const auto muted=viewers[0].total(),hostBefore=host.total();
    for(unsigned turn=0;turn<5;++turn) {
      viewers[0].peer->voicePacket(packet(speakers[0],Clock::now()));
      group.voicePacket(packet(speakers[3],Clock::now()));
      std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
    require(viewers[0].total()==muted && host.total()==hostBefore,"deafen gates sending and receiving at transport");
    // Host need not join voice to relay two consenting viewers.
    group.setVoiceState({false,false,false});
    const auto before=viewers[2].total();
    for(unsigned turn=0;turn<5;++turn) {
      viewers[1].peer->voicePacket(packet(speakers[1],Clock::now()));
      std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
    require(waitFor([&] { return viewers[2].total()>=before+4; }),"SFU works while host microphone and listening are off");
    group.remove("viewer0"); viewers[0].peer->stop(); viewers[0].peer.reset();
    connect(group,viewers[0],source,0,false);
    const auto legacyBefore=viewers[0].total();
    for(unsigned turn=0;turn<5;++turn) {
      viewers[1].peer->voicePacket(packet(speakers[1],Clock::now()));
      std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
    require(viewers[0].total()==legacyBefore,"old/nonvoice viewers receive no voice payload");
    group.stop();
    std::cout<<"Voice: consent, Opus20, loss/FEC/PLC, epoch, bounded jitter, mixing, priority, encrypted three-viewer SFU and compatibility passed\n";
  } catch(const std::exception& e) { std::cerr<<e.what()<<'\n'; return 1; }
}
