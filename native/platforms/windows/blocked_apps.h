#pragma once
#include "blocked_apps_policy.h"
#include <windows.h>
#include <vector>

namespace sanser::blocked {
// Windows host side of blocked apps (see blocked_apps_policy.h), shared by the
// SNV2 host and the legacy Mac-viewer host in the same process.
//  - windowsMask(): the visible area of blocked apps' top-level windows, in
//    virtual-screen pixels, for DesktopDuplicator::setMaskProvider.
//  - windowsInputAllowed(): false when a remote SendInput batch must be
//    dropped (a key press while a blocked app has focus, or a click/scroll
//    over a blocked window). Callers report a dropped batch as delivered, so
//    no release is retried.
// A process counts as blocked when its executable file name is on the list.
void configureWindows(const List& list);
bool windowsEnabled();
std::vector<Rect> windowsMask();
bool windowsInputAllowed(const INPUT* inputs, UINT count);
std::uint64_t windowsDropped();
}
