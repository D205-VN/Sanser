#import "shortcuts_panel.h"
#include <iostream>
#include <stdexcept>
using namespace sanser::desktop;
namespace {
void check(bool value,const char* message){if(!value)throw std::runtime_error(message);}
NSButton* button(NSPanel* panel,NSString* title){for(NSView* view in panel.contentView.subviews)if([view isKindOfClass:NSButton.class] && [[(NSButton*)view title] isEqualToString:title])return (NSButton*)view;return nil;}
NSTableView* table(NSPanel* panel){for(NSView* view in panel.contentView.subviews)if([view isKindOfClass:NSScrollView.class])return (NSTableView*)((NSScrollView*)view).documentView;return nil;}
NSSegmentedControl* chooser(NSPanel* panel){for(NSView* view in panel.contentView.subviews)if([view isKindOfClass:NSSegmentedControl.class])return (NSSegmentedControl*)view;return nil;}
void action(NSControl* control){[NSApp sendAction:control.action to:control.target from:control];}
}
int main(){@autoreleasepool{@try{try{
  [NSApplication sharedApplication];bool available=false,focused=true;std::uint32_t epoch=1;unsigned resets=0,submits=0;
  std::vector<std::uint16_t> last;
  ShortcutSession model({[&]{return available;},[&]{return focused;},[&]{return epoch;},[&]{++resets;return true;},[&](auto keys,auto generation){check(generation==epoch,"wrong UI generation");last.assign(keys.begin(),keys.end());++submits;return true;}});
  NSWindow* owner=[[NSWindow alloc] initWithContentRect:NSMakeRect(0,0,800,600) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable backing:NSBackingStoreBuffered defer:NO];owner.releasedWhenClosed=NO;
  SanserShortcuts* ui=[[SanserShortcuts alloc] initWithOwner:owner session:&model];
  [ui toggle];check(!ui.panel && !resets,"unsupported opener reset input");
  available=true;[ui refresh];[ui toggle];check(ui.panel && resets==1 && !model.host(),"modeless panel missing or guessed OS");
  check(table(ui.panel).numberOfRows==0 && !button(ui.panel,@"Send shortcut").enabled,"unknown host exposed action");
  auto choose=chooser(ui.panel);choose.selectedSegment=1;action(choose);
  check(table(ui.panel).numberOfRows==16,"Mac profile row count");
  [table(ui.panel) selectRowIndexes:[NSIndexSet indexSetWithIndex:0] byExtendingSelection:NO];
  auto send=button(ui.panel,@"Send shortcut");check(send.enabled,"chosen action not enabled");action(send);
  check(submits==1 && last==std::vector<std::uint16_t>{227,43},"native Mac action used local OS or wrong HID");
  auto oldClose=button(ui.panel,@"Close"),oldSend=send;auto oldHost=choose;
  action(oldClose);[ui toggle];check(ui.panel && !model.host(),"reopened panel retained old host choice");
  action(oldClose);action(oldSend);action(oldHost);check(ui.panel && submits==1 && !model.host(),"old native controls acted on replacement panel");
  choose=chooser(ui.panel);choose.selectedSegment=0;action(choose);[table(ui.panel) selectRowIndexes:[NSIndexSet indexSetWithIndex:0] byExtendingSelection:NO];
  ++epoch;action(button(ui.panel,@"Send shortcut"));[ui refresh];check(!ui.panel && submits==1,"generation change submitted stale action");
  [ui toggle];focused=false;[ui refresh];check(!ui.panel,"focus loss retained panel");
  focused=true;[ui toggle];available=false;[ui refresh];check(!ui.panel,"permission loss retained panel");
  [ui detach];available=true;[ui toggle];check(!ui.panel,"detached panel reopened");[owner close];
  unsigned closed=0;
  ShortcutSession restricted({[]{return true;},[]{return true;},[]{return 1U;},[]{return true;},[](auto,auto){return true;},
    [](auto host){return host==ShortcutHost::Windows;},[&]{++closed;}});
  NSWindow* legacyOwner=[[NSWindow alloc] initWithContentRect:NSMakeRect(0,0,800,600) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable backing:NSBackingStoreBuffered defer:NO];legacyOwner.releasedWhenClosed=NO;
  SanserShortcuts* legacy=[[SanserShortcuts alloc] initWithOwner:legacyOwner session:&restricted];[legacy toggle];
  check(legacy.panel && [chooser(legacy.panel) isEnabledForSegment:0] && ![chooser(legacy.panel) isEnabledForSegment:1],"Windows-only legacy engine exposed Mac profile");
  check(!restricted.select(ShortcutHost::Mac),"Unsupported host profile bypassed UI");
  action(button(legacy.panel,@"Close"));check(closed==1,"Closing panel did not cancel scoped sender work");
  [legacy detach];check(closed==1,"Repeated detach canceled a later capture");[legacyOwner close];
  std::cout<<"AppKit shortcuts: modeless native controls, explicit host, stale sender/focus/permission/generation fences passed\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}@catch(NSException* e){std::cerr<<"AppKit exception: "<<e.name.UTF8String<<'\n';return 1;}}}
