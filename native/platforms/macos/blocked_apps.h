#pragma once
#import <Cocoa/Cocoa.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#include "blocked_apps_policy.h"
#include "transport.h"
#include <memory>
#include <mutex>
#include <set>

namespace sanser::desktop {
// macOS host side of blocked apps (see blocked_apps_policy.h).
//  - Capture: windows of blocked applications are excluded from the
//    ScreenCaptureKit content filter by window ID. A cheap CoreGraphics window
//    list is polled; when a blocked app opens or shows a window that is not
//    excluded yet, the host loop rebuilds the filter.
//  - Input: remote key presses are dropped while a blocked app is frontmost,
//    remote clicks/scrolls are dropped over a blocked app's window.
// Capture methods: main thread. allow(): the input worker thread.
class API_AVAILABLE(macos(12.3)) MacBlockedApps {
public:
  MacBlockedApps(const blocked::List& list,CGRect bounds);
  bool enabled() const { return !list_.empty(); }
  // Window IDs of blocked apps that are on screen now. Cheap (CoreGraphics).
  std::set<CGWindowID> onScreenWindows() const;
  // True when an on-screen blocked window is neither excluded by the applied
  // filter nor already being excluded. Marks the current windows as tried.
  bool takeStale();
  // Blocked windows in `content`, for the content filter. Any thread.
  static NSArray<SCWindow*>* windowsIn(const blocked::List& list,SCShareableContent* content);
  NSArray<SCWindow*>* windowsIn(SCShareableContent* content) const { return windowsIn(list_,content); }
  // Records which blocked windows an applied filter excludes. Any thread;
  // safe after this object is gone.
  struct Applied { std::mutex mutex; std::set<CGWindowID> ids; void set(NSArray<SCWindow*>* windows); };
  std::shared_ptr<Applied> applied() const { return applied_; }
  const blocked::List& list() const { return list_; }
  // False when the remote event must be dropped.
  bool allow(const Input& event);
  void setBounds(CGRect bounds) { std::lock_guard lock(boundsMutex_); bounds_=bounds; }
  void reset() { gate_.reset(); }
  std::uint64_t dropped() const { return gate_.dropped(); }
private:
  bool blockedPid(pid_t pid) const;
  bool frontBlocked() const;
  bool pointBlocked(CGPoint point) const;
  blocked::List list_;
  blocked::InputGate gate_;
  std::mutex boundsMutex_;
  CGRect bounds_;
  CGPoint point_{},scrollPoint_{};
  Clock::time_point scrollCheckedAt_{};
  bool scrollBlocked_=false;
  std::shared_ptr<Applied> applied_=std::make_shared<Applied>();
  std::set<CGWindowID> tried_;
  pid_t self_=0,shell_=0;
};
}
