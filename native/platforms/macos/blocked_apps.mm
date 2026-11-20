#import "blocked_apps.h"
#include <unistd.h>

namespace sanser::desktop {
namespace {
bool matchesApp(const blocked::List& list,NSRunningApplication* app) {
  NSString* bundle=app.bundleIdentifier;
  return bundle && list.matches(bundle.UTF8String);
}
}

MacBlockedApps::MacBlockedApps(const blocked::List& list,CGRect bounds):list_(list),bounds_(bounds),self_(getpid()),shell_(getppid()) {}

bool MacBlockedApps::blockedPid(pid_t pid) const {
  if(pid<=0 || pid==self_) return false;
  NSRunningApplication* app=[NSRunningApplication runningApplicationWithProcessIdentifier:pid];
  return app && matchesApp(list_,app);
}

std::set<CGWindowID> MacBlockedApps::onScreenWindows() const {
  std::set<CGWindowID> ids;
  if(list_.empty()) return ids;
  std::set<pid_t> pids;
  for(NSRunningApplication* app in NSWorkspace.sharedWorkspace.runningApplications)
    if(matchesApp(list_,app)) pids.insert(app.processIdentifier);
  if(pids.empty()) return ids;
  CFArrayRef windows=CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly|kCGWindowListExcludeDesktopElements,kCGNullWindowID);
  if(!windows) return ids;
  for(NSDictionary* window in (__bridge NSArray*)windows) {
    const pid_t owner=[window[(__bridge NSString*)kCGWindowOwnerPID] intValue];
    if(pids.contains(owner)) ids.insert(static_cast<CGWindowID>([window[(__bridge NSString*)kCGWindowNumber] unsignedIntValue]));
  }
  CFRelease(windows);
  return ids;
}

bool MacBlockedApps::takeStale() {
  if(list_.empty()) return false;
  const auto now=onScreenWindows();
  std::set<CGWindowID> excluded; { std::lock_guard lock(applied_->mutex); excluded=applied_->ids; }
  bool stale=false;
  for(const auto id:now) if(!excluded.contains(id) && !tried_.contains(id)) stale=true;
  // A window that ScreenCaptureKit does not list cannot be excluded; trying
  // it once is enough, otherwise the filter would be rebuilt continuously.
  if(stale) tried_=now;
  return stale;
}

NSArray<SCWindow*>* MacBlockedApps::windowsIn(const blocked::List& list,SCShareableContent* content) {
  NSMutableArray<SCWindow*>* out=[NSMutableArray array];
  if(list.empty()) return out;
  for(SCWindow* window in content.windows) {
    NSString* bundle=window.owningApplication.bundleIdentifier;
    if(bundle && list.matches(bundle.UTF8String)) [out addObject:window];
  }
  return out;
}

void MacBlockedApps::Applied::set(NSArray<SCWindow*>* windows) {
  std::lock_guard lock(mutex);
  ids.clear();
  for(SCWindow* window in windows) ids.insert(window.windowID);
}

bool MacBlockedApps::frontBlocked() const {
  NSRunningApplication* front=NSWorkspace.sharedWorkspace.frontmostApplication;
  return front && matchesApp(list_,front);
}

bool MacBlockedApps::pointBlocked(CGPoint point) const {
  CFArrayRef windows=CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly|kCGWindowListExcludeDesktopElements,kCGNullWindowID);
  if(!windows) return false;
  bool blocked=false;
  // Front to back: the first window under the point receives the click.
  for(NSDictionary* window in (__bridge NSArray*)windows) {
    const pid_t owner=[window[(__bridge NSString*)kCGWindowOwnerPID] intValue];
    if(owner==self_) continue;  // privacy overlay, voice panel
    if([window[(__bridge NSString*)kCGWindowAlpha] doubleValue]<0.01) continue;
    NSString* name=window[(__bridge NSString*)kCGWindowName];
    if(owner==shell_ && [name isEqualToString:@"Sanser Annotations"]) continue;  // click-through
    CGRect frame{};
    if(!CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)window[(__bridge NSString*)kCGWindowBounds],&frame)) continue;
    if(!CGRectContainsPoint(frame,point)) continue;
    blocked=blockedPid(owner);
    break;
  }
  CFRelease(windows);
  return blocked;
}

bool MacBlockedApps::allow(const Input& event) {
  if(list_.empty()) return true;
  @autoreleasepool {
  CGRect bounds; { std::lock_guard lock(boundsMutex_); bounds=bounds_; }
  switch(event.kind) {
    case Input::Key: {
      const auto key=blocked::hidKey(event.code,event.down);
      // Only presses need the (cheap) frontmost check; releases always pass.
      return gate_.allowKey(key,key.down && !key.modifier && frontBlocked());
    }
    case Input::Text: return gate_.allowKey(blocked::Key{true,false,false,false},frontBlocked());
    case Input::Move:
    case Input::Button: {
      // Same mapping as MacInputInjector; scrolls use the last position.
      point_=CGPointMake(bounds.origin.x+event.x/65535.0*std::max(0.0,bounds.size.width-1),
                         bounds.origin.y+event.y/65535.0*std::max(0.0,bounds.size.height-1));
      if(event.kind==Input::Move || !event.down) return true;
      return gate_.allowPointer(true,pointBlocked(point_));
    }
    case Input::Scroll: {
      // A wheel burst sends many steps; reuse one window lookup per 100 ms.
      const auto now=Clock::now();
      if(now-scrollCheckedAt_>std::chrono::milliseconds(100) || point_.x!=scrollPoint_.x || point_.y!=scrollPoint_.y) {
        scrollBlocked_=pointBlocked(point_); scrollCheckedAt_=now; scrollPoint_=point_;
      }
      return gate_.allowPointer(true,scrollBlocked_);
    }
    case Input::Reset: gate_.reset(); return true;
    case Input::Gamepad: return !frontBlocked();
    default: return true;
  }
  }
}
}
