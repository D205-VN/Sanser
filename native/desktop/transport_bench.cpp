// Synthetic SNV2 transport load only: no screen capture, codec, render or OS input.
#include "transport.h"
#include <algorithm>
#include <cstring>
#include <iomanip>
#include <iostream>
#include <stdexcept>
using namespace sanser::desktop;
using Samples = std::vector<double>;
double percentile(Samples values,double p) {
  if(values.empty()) return 0;
  std::sort(values.begin(),values.end());
  return values[static_cast<std::size_t>((values.size()-1)*p)];
}
std::int64_t micros() { return std::chrono::duration_cast<std::chrono::microseconds>(Clock::now().time_since_epoch()).count(); }
struct Measurements {
  std::mutex mutex;
  std::vector<std::int64_t> keySent{0},moveSent{0};
  Samples keys,moves,frames;
  unsigned staleMoves=0,lastMove=0,keyOrderErrors=0,lastKey=0;
};
int main(int argc,char** argv) {
  try {
    if(argc!=6) throw std::runtime_error("Usage: transport-bench HOST_PORT CLIENT_PORT HOST_PROXY CLIENT_PROXY SECONDS");
    const auto seconds=std::stoi(argv[5]);
    if(seconds<1 || seconds>60) throw std::runtime_error("Duration must be 1..60 seconds");
    Options h; h.host=true; h.port=static_cast<std::uint16_t>(std::stoi(argv[1])); h.peer="127.0.0.1:"+std::string(argv[3]); h.token=std::string(48,'b');
    Options c=h; c.host=false; c.port=static_cast<std::uint16_t>(std::stoi(argv[2])); c.peer="127.0.0.1:"+std::string(argv[4]);
    Peer host(h),client(c); Measurements m; std::atomic<bool> forceKeyframe{true};
    host.onInput=[&](Input input) {
      if(input.kind!=Input::Key && input.kind!=Input::Move) return;
      const auto id=(static_cast<unsigned>(input.x)<<16)|input.y;
      std::lock_guard lock(m.mutex);
      const auto& timestamps=input.kind==Input::Key ? m.keySent : m.moveSent;
      if(!id || id>=timestamps.size()) return;
      const auto latency=(micros()-timestamps[id])/1000.0;
      if(input.kind==Input::Key) { m.keys.push_back(latency); if(id!=m.lastKey+1) ++m.keyOrderErrors; m.lastKey=id; }
      else { m.moves.push_back(latency); if(id<=m.lastMove) ++m.staleMoves; m.lastMove=std::max(m.lastMove,id); }
    };
    host.onKeyframe=[&] { forceKeyframe=true; };
    client.onFrame=[&](Frame frame) {
      if(frame.data.size()<sizeof(std::int64_t)) return;
      std::int64_t sent=0; std::memcpy(&sent,frame.data.data(),sizeof(sent));
      std::lock_guard lock(m.mutex); m.frames.push_back((micros()-sent)/1000.0);
    };
    PeerStop stopHost{host},stopClient{client}; host.start(); client.start();
    const auto deadline=Clock::now()+std::chrono::seconds(5);
    while((!host.ready() || !client.ready()) && Clock::now()<deadline) std::this_thread::sleep_for(std::chrono::milliseconds(5));
    if(!host.ready() || !client.ready()) throw std::runtime_error("Synthetic peer handshake failed");
    const auto start=Clock::now(),end=start+std::chrono::seconds(seconds);
    auto nextVideo=start,nextMove=start,nextKey=start;
    unsigned sentFrames=0;
    auto sendInput=[&](bool key) {
      unsigned id;
      { std::lock_guard lock(m.mutex); auto& times=key?m.keySent:m.moveSent; id=static_cast<unsigned>(times.size()); times.push_back(micros()); }
      client.input(Input{key?Input::Key:Input::Move,static_cast<std::uint16_t>(id>>16),static_cast<std::uint16_t>(id),static_cast<std::uint16_t>(key?4:0),0,(id%2)!=0});
    };
    while(Clock::now()<end && host.running() && client.running()) {
      const auto now=Clock::now();
      if(now>=nextKey) { sendInput(true); nextKey+=std::chrono::milliseconds(50); }
      if(now>=nextMove) { sendInput(false); nextMove+=std::chrono::microseconds(8333); }
      if(now>=nextVideo) {
        Frame frame{Bytes(41666,0x42),1920,1080,forceKeyframe.exchange(false) || sentFrames%60==0};
        const auto time=micros(); std::memcpy(frame.data.data(),&time,sizeof(time)); host.video(frame); ++sentFrames;
        nextVideo+=std::chrono::microseconds(16667);
      }
      std::this_thread::sleep_until(std::min({nextVideo,nextKey,nextMove}));
    }
    const bool survived=host.running() && client.running();
    const double activeSeconds=std::chrono::duration<double>(Clock::now()-start).count();
    std::this_thread::sleep_for(std::chrono::seconds(2)); client.stop(); host.stop();
    std::lock_guard lock(m.mutex);
    auto print=[&](const char* name,const Samples& samples) { std::cout<<'"'<<name<<"\":{\"received\":"<<samples.size()<<",\"p50_ms\":"<<percentile(samples,.5)<<",\"p95_ms\":"<<percentile(samples,.95)<<",\"max_ms\":"<<percentile(samples,1)<<'}'; };
    std::cout<<std::fixed<<std::setprecision(3)<<"{\"survived\":"<<(survived?"true":"false")<<",\"active_seconds\":"<<activeSeconds<<",\"keys_sent\":"<<m.keySent.size()-1<<",\"moves_sent\":"<<m.moveSent.size()-1<<",\"frames_sent\":"<<sentFrames<<",\"delivered_fps\":"<<m.frames.size()/activeSeconds<<",\"stale_moves\":"<<m.staleMoves<<",\"key_order_errors\":"<<m.keyOrderErrors<<',';
    print("keys",m.keys); std::cout<<','; print("moves",m.moves); std::cout<<','; print("frames",m.frames); std::cout<<"}\n";
    return survived && m.staleMoves==0 && m.keyOrderErrors==0 && m.keys.size()==m.keySent.size()-1 ? 0 : 2;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
