#pragma once
#ifdef __APPLE__
#include <pthread.h>
#include <pthread/qos.h>
#endif

namespace sanser {
// Media workers do short, latency-sensitive work and wait between packets.
// Do not inherit utility/background timer coalescing from their creator.
// This changes neither process scheduling policy nor other threads' priority.
inline bool prioritizeMediaThread() {
#ifdef __APPLE__
  return pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE,0)==0;
#else
  return true; // Keep the existing Windows/Android scheduling policy.
#endif
}
}
