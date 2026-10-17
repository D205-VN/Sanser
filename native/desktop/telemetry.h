#pragma once
#include <cstdio>
#include <sstream>

namespace sanser::desktop {
// One stdio call holds stdout's FILE lock for the whole record. A peer thread
// must not splice numeric fields into another peer's timing line.
inline void writeTelemetry(const std::ostringstream& line) {
  std::fprintf(stdout,"%s\n",line.str().c_str()); std::fflush(stdout);
}
}
