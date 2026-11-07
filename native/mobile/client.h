#pragma once
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
// Callback buffers are borrowed for the duration of the call. Callbacks run on
// native workers; callers must dispatch UI work and must not destroy from one.
typedef struct {
  void *context;
  void (*video)(void *,const uint8_t *,size_t,uint32_t,uint32_t,int);
  void (*audio)(void *,const int16_t *,size_t);
  void (*stopped)(void *,unsigned);
} SanserMobileCallbacks;
typedef struct SanserMobileClient SanserMobileClient;
SanserMobileClient *sanser_mobile_create(const char *peer,uint16_t port,const char *token,
  int control,int sharing,const char *viewer,SanserMobileCallbacks callbacks);
void sanser_mobile_video_limit(SanserMobileClient *client,uint32_t bitrate,double baseline_rtt_ms);
int sanser_mobile_start(SanserMobileClient *client);
void sanser_mobile_destroy(SanserMobileClient *client);
int sanser_mobile_running(SanserMobileClient *client);
int sanser_mobile_ready(SanserMobileClient *client);
double sanser_mobile_rtt(SanserMobileClient *client);
typedef struct {
  uint64_t video_packets,video_frames,nack_requests,lost_fragments,reference_drops;
  uint64_t audio_decoded,audio_concealed,audio_late;
} SanserMobileStats;
// Latest one-second native snapshot; numeric counters only, no payload/IDs.
void sanser_mobile_stats(SanserMobileClient *client,SanserMobileStats *stats);
void sanser_mobile_input(SanserMobileClient *client,unsigned kind,unsigned x,unsigned y,unsigned code,int delta,int down);
typedef struct {
  int connected,sampled;
  uint32_t buttons; // Mobile physical layout: A B X Y LB RB Start Select L3 R3 U D L R.
  double lx,ly,rx,ry,lt,rt; // Y up, sticks [-1,1], triggers [0,1].
} SanserMobileGamepad;
int sanser_mobile_gamepad_available(SanserMobileClient *);
// Capture IDs are monotonic per client. Old stop/sample commands never affect
// a replacement. Native snapshots/heartbeat bypass Flutter input serialization.
int sanser_mobile_gamepad_capture(SanserMobileClient *,uint64_t capture,int enabled);
int sanser_mobile_gamepad_sample(SanserMobileClient *,uint64_t capture,const SanserMobileGamepad *);
// Bits: 1 available, 2 matching capture active, 4 locally armed, 8 connected.
unsigned sanser_mobile_gamepad_status(SanserMobileClient *,uint64_t capture);
int sanser_mobile_text_available(SanserMobileClient *client);
// UTF-8, explicit length, <=1024 bytes/256 UTF-16 units. No C-string truncation.
int sanser_mobile_text(SanserMobileClient *client,const uint8_t *utf8,size_t bytes);
int sanser_mobile_clipboard_available(SanserMobileClient *client);
typedef void (*SanserMobileClipboardReply)(void *,unsigned,const uint8_t *,size_t);
// A queued request completes once on a native worker, including stop/timeout.
int sanser_mobile_clipboard(SanserMobileClient *,int write,const uint8_t *utf8,size_t bytes,void *context,SanserMobileClipboardReply reply);
int sanser_mobile_files_enable(SanserMobileClient *,int enabled); // Before start only.
int sanser_mobile_files_available(SanserMobileClient *);
uint32_t sanser_mobile_file_generation(SanserMobileClient *);
int sanser_mobile_file_send(SanserMobileClient *,uint32_t generation,const uint8_t *,size_t);
// Native receive queue: at most 16 bounded messages. Caller supplies 8192 bytes.
size_t sanser_mobile_file_receive(SanserMobileClient *,uint32_t generation,uint8_t *,size_t capacity);
int sanser_mobile_file_overflow(SanserMobileClient *); // Read-and-clear; abort files only.
void sanser_mobile_mute(SanserMobileClient *client,int muted);
void sanser_mobile_keyframe(SanserMobileClient *client);
// One PCM producer and one realtime render consumer; render never locks/allocates.
typedef struct SanserMobilePCM SanserMobilePCM;
SanserMobilePCM *sanser_mobile_pcm_create(void);
void sanser_mobile_pcm_destroy(SanserMobilePCM *pcm);
void sanser_mobile_pcm_push(SanserMobilePCM *pcm,const int16_t *stereo,size_t samples);
void sanser_mobile_pcm_render(SanserMobilePCM *pcm,float *left,float *right,size_t frames);
void sanser_mobile_pcm_reset(SanserMobilePCM *pcm);
#ifdef __cplusplus
}
#endif
