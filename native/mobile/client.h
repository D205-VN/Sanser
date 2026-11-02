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
