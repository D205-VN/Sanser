#pragma once

// Platform entry points stay independent of the shared wire transport.
namespace sanser::desktop {
#ifdef __APPLE__
bool macHostAvailable();
int runMac(bool host, int argc, char** argv);
#elif defined(_WIN32)
int runWindows(bool host, int argc, char** argv);
#endif
}
