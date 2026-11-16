#pragma once
#ifdef __APPLE__
#include <pthread.h>
#include <pthread/qos.h>
#elif defined(_WIN32)
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#endif

namespace sanser {
// Media workers do short, latency-sensitive work and wait between packets.
// Do not inherit utility/background timer coalescing from their creator.
// This changes neither process scheduling policy nor other threads' priority.
inline bool prioritizeMediaThread() {
#ifdef __APPLE__
  return pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE,0)==0;
#elif defined(_WIN32)
  // Normal priority let video decode/convert threads starve 5 ms audio work,
  // which was heard as stutter and crackle on Windows only. HIGHEST stays
  // below TIME_CRITICAL and the realtime class; these threads mostly wait.
  return SetThreadPriority(GetCurrentThread(),THREAD_PRIORITY_HIGHEST)!=0;
#else
  return true; // Keep the existing Android scheduling policy.
#endif
}
}
