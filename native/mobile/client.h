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
int sanser_mobile_start(SanserMobileClient *client);
void sanser_mobile_destroy(SanserMobileClient *client);
int sanser_mobile_running(SanserMobileClient *client);
int sanser_mobile_ready(SanserMobileClient *client);
double sanser_mobile_rtt(SanserMobileClient *client);
void sanser_mobile_input(SanserMobileClient *client,unsigned kind,unsigned x,unsigned y,unsigned code,int delta,int down);
void sanser_mobile_mute(SanserMobileClient *client,int muted);
void sanser_mobile_keyframe(SanserMobileClient *client);
#ifdef __cplusplus
}
#endif
