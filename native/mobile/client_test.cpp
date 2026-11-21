#include "client.h"
#include "transport.h"
#include "voice.h"
#include <cmath>
#include <atomic>
#include <iostream>
#include <memory>
#include <random>
#include <stdexcept>
using namespace sanser::desktop;
namespace {
void require(bool ok,const char* text){if(!ok)throw std::runtime_error(text);}
template<class T> bool waitFor(T condition){auto until=Clock::now()+std::chrono::seconds(5);while(Clock::now()<until){if(condition())return true;std::this_thread::sleep_for(std::chrono::milliseconds(5));}return condition();}
struct Results {std::atomic<unsigned> frames{0},keys{0},texts{0},clipboards{0},clipboardStops{0},stopped{0};};
void video(void *context,const uint8_t *bytes,size_t size,uint32_t width,uint32_t height,int key){if(size==5000 && bytes[0]==42 && width==1280 && height==720 && key)static_cast<Results*>(context)->frames++;}
void stopped(void *context,uint32_t){static_cast<Results*>(context)->stopped++;}
void clipboardReply(void* context,unsigned status,const uint8_t* text,size_t count){if(status==0 && std::string(reinterpret_cast<const char*>(text),count)=="clipboard ế😀")static_cast<Results*>(context)->clipboards++;if(status==5)static_cast<Results*>(context)->clipboardStops++;}
std::atomic<size_t> voiceSamples{0};std::atomic<int> voicePeak{0};
std::mutex chatMutex;std::vector<std::string> chats;
void chatInput(void*,const uint8_t *bytes,size_t count){std::lock_guard lock(chatMutex);chats.emplace_back(reinterpret_cast<const char*>(bytes),count);}
void voiceOutput(void*,const int16_t *pcm,size_t count){voiceSamples+=count;for(size_t i=0;i<count;++i)voicePeak=std::max(voicePeak.load(),std::abs(static_cast<int>(pcm[i])));}
struct Delete {void operator()(SanserMobileClient *c)const{sanser_mobile_destroy(c);}};
}
int main(){try{
  static_assert(sizeof(SanserMobileCallbacks)==6*sizeof(void*),"callback ABI changed");
  Results results;
  require(!sanser_mobile_create("127.0.0.1:1",0,"short",1,0,"",{&results,video,nullptr,nullptr}),"short credentials accepted");
  for(bool control:{false,true}){
    std::unique_ptr<Peer> host;std::unique_ptr<SanserMobileClient,Delete> client;
    for(unsigned attempt=0;attempt<20 && !client;++attempt){
      auto port=static_cast<uint16_t>(30000+std::random_device{}()%18000);
      Options options;options.host=true;options.clipboard=true;options.files=true;options.collab=true;options.port=port;options.peer="127.0.0.1:"+std::to_string(port+1);options.token=std::string(48,'m');
      try {host=std::make_unique<Peer>(options);client.reset(sanser_mobile_create(("127.0.0.1:"+std::to_string(port)).c_str(),port+1,options.token.c_str(),control,0,"",{&results,video,nullptr,stopped,nullptr,chatInput}));}catch(...){host.reset();}
    }
    require(host && client,"unable to allocate test ports");
    std::atomic<unsigned> hostChats{0};
    host->onCollab=[&](Bytes payload){if(std::string(payload.begin(),payload.end())==R"({"type":"chat","id":"m1","name":"Phone","text":"Xin chào 😀"})")++hostChats;};
    std::atomic<unsigned> pads{0};std::atomic<bool> padBackend{true};
    host->onGamepadAvailable=[&]{return padBackend.load();};
    host->onInput=[&](Input i){if(i.kind==Input::Key)results.keys++;if(i.kind==Input::Text && i.text=="Tiếng Việt 😀")results.texts++;if(i.kind==Input::Gamepad && i.gamepad.connected)++pads;};
    std::atomic<bool> holdClipboard{false};std::atomic<unsigned> clipboardRequests{0};
    host->onClipboard=[&](bool,std::string,std::function<bool()> current,Peer::ClipboardResult reply){++clipboardRequests;if(!holdClipboard)reply(current()?0:5,"clipboard ế😀");};
    Bytes fileBody{'S','F','T','1',3};fileBody.resize(4125,42);std::atomic<unsigned> hostFiles{0};
    host->onFile=[&](Bytes bytes){require(bytes==fileBody,"mobile file bytes corrupted");++hostFiles;};
    require(sanser_mobile_files_enable(client.get(),1),"file capability must configure before start");
    sanser_mobile_video_limit(client.get(),2000000,8.0);
    PeerStop stop{*host};host->start();require(sanser_mobile_start(client.get()),"client start failed");
    require(waitFor([&]{return host->ready() && sanser_mobile_ready(client.get());}),"authenticated handshake failed");
    require(waitFor([&]{return host->recommendedBitrate()<=2000000;}),"mobile receive ceiling did not reach the desktop encoder budget");
    sanser_mobile_input(client.get(),2,0,0,4,0,1);sanser_mobile_input(client.get(),2,0,0,4,0,0);
    if(control)require(waitFor([&]{return results.keys==2;}),"keyboard did not reach desktop");
    else {std::this_thread::sleep_for(std::chrono::milliseconds(100));require(results.keys==0,"view-only injected keyboard");}
    const std::string edit="Tiếng Việt 😀";
    if(control) {
      require(waitFor([&]{return sanser_mobile_text_available(client.get());}),"C ABI did not expose negotiated Unicode support");
      require(sanser_mobile_text(client.get(),reinterpret_cast<const uint8_t*>(edit.data()),edit.size()),"C ABI rejected valid Unicode");
      require(waitFor([&]{return results.texts==1;}),"C ABI corrupted Unicode on its way to the host");
    } else require(!sanser_mobile_text_available(client.get()) && !sanser_mobile_text(client.get(),reinterpret_cast<const uint8_t*>(edit.data()),edit.size()),"C ABI view-only injected Unicode");
    SanserMobileGamepad pad{};pad.connected=pad.sampled=1;
    if(control){
      require(waitFor([&]{return sanser_mobile_gamepad_available(client.get());}),"mobile analog capability absent");
      require(sanser_mobile_gamepad_capture(client.get(),1,1) && sanser_mobile_gamepad_sample(client.get(),1,&pad),"C ABI analog activation/neutral");
      require(waitFor([&]{return pads>0;}),"neutral did not reach host");
      pad.lx=.5;pad.buttons=1;require(sanser_mobile_gamepad_sample(client.get(),1,&pad),"C ABI analog sample");
      const auto before=pads.load();std::this_thread::sleep_for(std::chrono::milliseconds(350));
      require(pads>=before+5 && (sanser_mobile_gamepad_status(client.get(),1)&4),"held heartbeat failed host lease");
      sanser_mobile_input(client.get(),4,0,0,0,0,0);
      require((sanser_mobile_gamepad_status(client.get(),1)&2)==0 && !sanser_mobile_gamepad_sample(client.get(),1,&pad),"reset left analog captured");
      require(sanser_mobile_gamepad_capture(client.get(),2,1),"second analog capture");
      require(sanser_mobile_gamepad_capture(client.get(),1,0) && (sanser_mobile_gamepad_status(client.get(),2)&2),"stale C stop canceled replacement");
      padBackend=false;require(waitFor([&]{return !sanser_mobile_gamepad_available(client.get()) && (sanser_mobile_gamepad_status(client.get(),2)&2)==0;}),"backend withdrawal kept mobile active");
    }else require(!sanser_mobile_gamepad_available(client.get()) && !sanser_mobile_gamepad_capture(client.get(),1,1) && !sanser_mobile_gamepad_sample(client.get(),1,&pad),"view-only enabled analog");
    require(!sanser_mobile_text(client.get(),nullptr,1) && !sanser_mobile_text(client.get(),reinterpret_cast<const uint8_t*>(edit.data()),1025),"C ABI accepted invalid lengths");
    if(control){
      require(waitFor([&]{return sanser_mobile_clipboard_available(client.get());}),"C ABI clipboard capability absent");
      require(sanser_mobile_clipboard(client.get(),0,nullptr,0,&results,clipboardReply),"C ABI clipboard read rejected");
      require(waitFor([&]{return results.clipboards==1;}),"C ABI did not complete clipboard request");
    }else require(!sanser_mobile_clipboard_available(client.get()) && !sanser_mobile_clipboard(client.get(),0,nullptr,0,&results,clipboardReply),"C ABI view-only read remote clipboard");
    require(!sanser_mobile_files_enable(client.get(),0),"file configuration changed after start");
    if(control){
      require(waitFor([&]{return sanser_mobile_files_available(client.get()) && host->filesAvailable();}),"C ABI file negotiation failed");
      const auto epoch=sanser_mobile_file_generation(client.get());require(epoch!=0,"unmeasured file generation");
      require(!sanser_mobile_file_send(client.get(),epoch==1?2:1,fileBody.data(),fileBody.size()),"stale file generation sent");
      require(sanser_mobile_file_send(client.get(),epoch,fileBody.data(),fileBody.size()),"C ABI file send failed");
      require(waitFor([&]{return hostFiles==1;}),"C ABI file did not reach desktop");
      require(host->fileMessage(fileBody),"desktop file send failed");uint8_t bytes[8192];size_t count=0;
      require(waitFor([&]{if(!count)count=sanser_mobile_file_receive(client.get(),epoch,bytes,sizeof(bytes));return count>0;}),"desktop file did not reach C ABI");
      require(Bytes(bytes,bytes+count)==fileBody,"C ABI file queue corrupted data");
    }else require(!sanser_mobile_files_available(client.get()) && !sanser_mobile_file_send(client.get(),sanser_mobile_file_generation(client.get()),fileBody.data(),fileBody.size()),"view-only received file capability");
    const auto before=results.frames.load();host->video(Frame{Bytes(5000,42),1280,720,true});
    require(waitFor([&]{return results.frames>before;}),"fragmented video did not reach C callback");
    // In-session chat both ways through the C ABI.
    require(waitFor([&]{return sanser_mobile_collab_available(client.get()) && host->collabAvailable();}),"chat capability was not negotiated");
    const std::string chat=R"({"type":"chat","id":"m1","name":"Phone","text":"Xin chào 😀"})";
    require(sanser_mobile_collab_send(client.get(),reinterpret_cast<const uint8_t*>(chat.data()),chat.size()),"mobile chat refused");
    require(!sanser_mobile_collab_send(client.get(),reinterpret_cast<const uint8_t*>(chat.data()),0),"empty chat accepted");
    std::string huge(4097,'x');require(!sanser_mobile_collab_send(client.get(),reinterpret_cast<const uint8_t*>(huge.data()),huge.size()),"oversized chat accepted");
    require(waitFor([&]{return hostChats==1;}),"mobile chat did not reach the host");
    const std::string reply=R"({"type":"chat","id":"h1","name":"Host","text":"Chào"})";
    require(host->collab(Bytes(reply.begin(),reply.end())),"host chat refused");
    require(waitFor([&]{std::lock_guard lock(chatMutex);return !chats.empty() && chats.back()==reply;}),"host chat did not reach the mobile callback");
    // Background: a hidden picture stops fresh video; showing again resumes on a keyframe.
    sanser_mobile_video_hidden(client.get(),1);
    require(waitFor([&]{return host->peerVideoHidden() && !host->acceptsCapture();}),"host kept sending video to a hidden viewer");
    {const auto hidden=results.frames.load();host->video(Frame{Bytes(5000,42),1280,720,true});
     std::this_thread::sleep_for(std::chrono::milliseconds(150));require(results.frames==hidden,"hidden viewer still received video");}
    sanser_mobile_video_hidden(client.get(),0);
    require(waitFor([&]{return !host->peerVideoHidden();}),"host did not resume video for a visible viewer");
    {const auto shown=results.frames.load();require(waitFor([&]{host->video(Frame{Bytes(5000,42),1280,720,true});return results.frames>shown;}),"video did not resume after showing");}
    require(waitFor([&]{return sanser_mobile_rtt(client.get())>=0;}),"unmeasured RTT");
    require(waitFor([&]{SanserMobileStats stats{};sanser_mobile_stats(client.get(),&stats);return stats.video_frames>0 && stats.video_packets>0;}),"mobile numeric receive snapshot did not update without stdout tracing");
    SanserMobileStats empty{1,1,1,1,1,1,1,1};sanser_mobile_stats(nullptr,&empty);
    require(empty.video_frames==0 && empty.audio_decoded==0,"disconnected diagnostics must be empty");
    const auto stoppedBefore=results.stopped.load();
    const auto clipboardStops=results.clipboardStops.load();
    if(control) {
      std::this_thread::sleep_for(std::chrono::milliseconds(120));holdClipboard=true;
      require(sanser_mobile_clipboard(client.get(),0,nullptr,0,&results,clipboardReply),"queue pending C ABI clipboard before close");
      require(waitFor([&]{return clipboardRequests==2;}),"pending clipboard did not reach host");
    }
    client.reset();require(results.stopped==stoppedBefore+1,"stopped callback was not joined safely during destruction");
    if(control)require(results.clipboardStops==clipboardStops+1,"closing C ABI client leaked a pending clipboard callback");
    require(waitFor([&]{return !host->running();}),"client close did not disconnect desktop");host->stop();
  }
  // Cancel while the host has not answered: this uses the same stopped callback
  // path as a failed connection, before any video frame has been received.
  for(unsigned attempt=0;attempt<3;++attempt){
    auto before=results.stopped.load();
    std::unique_ptr<SanserMobileClient,Delete> pending(sanser_mobile_create("127.0.0.1:9",0,std::string(48,'m').c_str(),1,0,"",{&results,video,nullptr,stopped}));
    require(pending && sanser_mobile_start(pending.get()),"pending connection did not start");
    pending.reset();require(results.stopped==before+1,"cancel before handshake lost stopped callback");
  }
  {
    // Group voice: view-only sharing viewer <-> host, through the C ABI only.
    std::unique_ptr<Peer> host;std::unique_ptr<SanserMobileClient,Delete> client;
    std::atomic<unsigned> heard{0};
    for(unsigned attempt=0;attempt<20 && !client;++attempt){
      auto port=static_cast<uint16_t>(30000+std::random_device{}()%18000);
      Options options;options.host=true;options.sharing=true;options.input=false;options.groupVoice=true;options.voiceSlot=1;
      options.port=port;options.peer="127.0.0.1:"+std::to_string(port+1);options.token=std::string(48,'v');
      try {host=std::make_unique<Peer>(options);host->onVoice=[&](sanser::voice::Packet p){if(p.speaker==1)++heard;};
        client.reset(sanser_mobile_create(("127.0.0.1:"+std::to_string(port)).c_str(),port+1,options.token.c_str(),0,1,"viewer-1",{&results,video,nullptr,stopped,voiceOutput}));}catch(...){host.reset();}
    }
    require(host && client,"unable to allocate voice test ports");
    require(sanser_mobile_voice_available(client.get()),"view-only sharing must offer group voice");
    PeerStop stop{*host};host->start();require(sanser_mobile_start(client.get()),"voice client start failed");
    require(waitFor([&]{return host->ready() && sanser_mobile_ready(client.get());}),"voice handshake failed");
    std::vector<int16_t> tone(480);for(size_t i=0;i<tone.size();++i)tone[i]=static_cast<int16_t>(8000*std::sin(i*2*3.14159265*440/48000));
    require(!sanser_mobile_voice_capture(client.get(),tone.data(),tone.size()),"microphone captured before consent");
    host->setVoiceState({true,true,false});
    require(sanser_mobile_voice_state(client.get(),1,1,0),"voice consent rejected");
    sanser::voice::Encoder speaker;
    for(unsigned turn=0;turn<60 && (heard<10 || voiceSamples<48000/10);++turn){
      sanser_mobile_voice_capture(client.get(),tone.data(),tone.size());
      sanser::desktop::AudioSamples block{};for(unsigned i=0;i<audioFrames;++i)block[i*2]=block[i*2+1]=tone[i];
      for(unsigned k=0;k<2;++k) if(auto packet=speaker.push(block)){packet->speaker=0;host->forwardVoice(*packet);}
      std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    require(heard>=10,"mobile microphone did not reach the host with its assigned speaker slot");
    require(voiceSamples>=48000/10 && voicePeak>1000,"host voice was not played through the mobile voice callback");
    require(sanser_mobile_voice_state(client.get(),1,0,0),"mute rejected");
    require(!sanser_mobile_voice_capture(client.get(),tone.data(),tone.size()),"muted microphone still captured");
    std::this_thread::sleep_for(std::chrono::milliseconds(60));const auto muted=heard.load();
    std::this_thread::sleep_for(std::chrono::milliseconds(100));
    require(heard==muted,"muted microphone still reached the host");
    std::unique_ptr<SanserMobileClient,Delete> controlled(sanser_mobile_create("127.0.0.1:9",0,std::string(48,'m').c_str(),1,1,"",{&results,video,nullptr,nullptr}));
    require(controlled && !sanser_mobile_voice_available(controlled.get()) && !sanser_mobile_voice_state(controlled.get(),1,1,0),"controlling session offered group voice");
    client.reset();host->stop();
  }
  std::cout<<"Mobile C ABI: authenticated desktop interoperability, view-only, keyboard, video, group voice, chat, background video hold, RTT and teardown passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
