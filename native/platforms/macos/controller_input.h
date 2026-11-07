#pragma once
#import <Cocoa/Cocoa.h>
#include "../../desktop/controller_session.h"
@class GCController;
@interface SanserControllerInput : NSObject {
  __weak NSWindow* _owner;
  NSButton* _button;
  GCController* _device;
  sanser::desktop::ControllerSession* _session;
  std::function<sanser::desktop::ControllerSample()> _source;
  std::function<bool()> _focused;
  std::chrono::steady_clock::time_point _polled;
}
@property(nonatomic,readonly) NSButton* button;
- (instancetype)initWithOwner:(NSWindow*)owner session:(sanser::desktop::ControllerSession*)session
  source:(std::function<sanser::desktop::ControllerSample()>)source focused:(std::function<bool()>)focused;
- (void)refresh;
- (void)toggle;
- (void)stop;
- (void)detach;
@end
