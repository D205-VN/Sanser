#pragma once
#include "audio.h"
#include <cstdio>
#include <sstream>

namespace sanser::desktop {
// One stdio call holds stdout's FILE lock for the whole record. A peer thread
// must not splice numeric fields into another peer's timing line.
inline void writeTelemetry(const std::ostringstream& line) {
  std::fprintf(stdout,"%s\n",line.str().c_str()); std::fflush(stdout);
}
// Call from the viewer's existing one-second health tick, never the audio
// device callback. Counters contain no captured PCM or device identity.
inline void writeAudioOutputHealth(const AudioQueue::Stats& stats) {
  std::ostringstream line;
  line<<"SNV2_AUDIO_OUTPUT_HEALTH pcmUnderflows="<<stats.underflows
    <<" pcmCapacityDrops="<<stats.capacityDrops<<" pcmStaleDrops="<<stats.staleDrops
    <<" pcmReprimes="<<stats.reprimes<<" pcmResets="<<stats.resets;
  writeTelemetry(line);
}
}
