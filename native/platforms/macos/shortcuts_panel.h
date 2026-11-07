#pragma once
#import <Cocoa/Cocoa.h>
#include "../../desktop/session_shortcuts.h"

@interface SanserShortcuts : NSObject <NSWindowDelegate,NSTableViewDataSource,NSTableViewDelegate> {
  NSWindow* __weak _owner;
  sanser::desktop::ShortcutSession* _session;
  NSPanel* _panel;
  NSButton* _opener;
  NSButton* _send;
  NSButton* _close;
  NSSegmentedControl* _host;
  NSTableView* _table;
  NSTextField* _notice;
  std::vector<sanser::desktop::SessionShortcut> _actions;
}
@property(nonatomic,readonly) NSPanel* panel;
- (instancetype)initWithOwner:(NSWindow*)owner session:(sanser::desktop::ShortcutSession*)session;
- (void)toggle;
- (void)refresh;
- (void)detach;
@end
