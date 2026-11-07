#import "shortcuts_panel.h"
#include <algorithm>
using namespace sanser::desktop;
namespace {NSString* label(std::string_view value){return [[NSString alloc] initWithBytes:value.data() length:value.size() encoding:NSUTF8StringEncoding];}}
@implementation SanserShortcuts
- (NSPanel*)panel{return _panel;}
- (instancetype)initWithOwner:(NSWindow*)owner session:(ShortcutSession*)session {
  if((self=[super init])) {
    _owner=owner;_session=session;
    auto* accessory=[[NSTitlebarAccessoryViewController alloc] init];accessory.layoutAttribute=NSLayoutAttributeRight;
    accessory.view=[[NSView alloc] initWithFrame:NSMakeRect(0,0,100,28)];
    _opener=[NSButton buttonWithTitle:@"Shortcuts" target:self action:@selector(toggleFromButton:)];
    _opener.frame=NSMakeRect(4,2,92,24);_opener.refusesFirstResponder=YES;_opener.bezelStyle=NSBezelStyleRounded;
    _opener.toolTip=@"Send a Windows/Mac shortcut to the host. Physical input is released while the panel is open.";
    [accessory.view addSubview:_opener];[owner addTitlebarAccessoryViewController:accessory];[self refresh];
  }return self;
}
- (void)toggleFromButton:(id)sender {if(sender==_opener)[self toggle];}
- (void)closePanel {
  if(_session)_session->close();_actions.clear();
  if(_panel){[_owner removeChildWindow:_panel];_panel.delegate=nil;[_panel orderOut:nil];[_panel close];}
  _panel=nil;_table=nil;_host=nil;_send=nil;_close=nil;_notice=nil;
}
- (void)toggle {
  if(_panel){[self closePanel];return;}
  if(!_owner || !_session || !_session->open())return;
  _panel=[[NSPanel alloc] initWithContentRect:NSMakeRect(0,0,480,520) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskUtilityWindow backing:NSBackingStoreBuffered defer:NO];
  _panel.releasedWhenClosed=NO;_panel.title=@"Host shortcuts";_panel.delegate=self;_panel.hidesOnDeactivate=YES;
  NSView* content=_panel.contentView;
  NSTextField* heading=[NSTextField wrappingLabelWithString:@"Choose the REMOTE computer's OS. Copy/Paste uses its clipboard."];
  heading.frame=NSMakeRect(16,464,448,40);[content addSubview:heading];
  _host=[NSSegmentedControl segmentedControlWithLabels:@[@"Windows host",@"Mac host"] trackingMode:NSSegmentSwitchTrackingSelectOne target:self action:@selector(chooseHost:)];
  _host.selectedSegment=-1;_host.frame=NSMakeRect(16,426,448,28);_host.accessibilityIdentifier=@"shortcut-host";[content addSubview:_host];
  [_host setEnabled:_session->supports(ShortcutHost::Windows) forSegment:0];
  [_host setEnabled:_session->supports(ShortcutHost::Mac) forSegment:1];
  _table=[[NSTableView alloc] initWithFrame:NSMakeRect(0,0,448,334)];
  NSTableColumn* column=[[NSTableColumn alloc] initWithIdentifier:@"action"];column.width=440;[_table addTableColumn:column];
  _table.headerView=nil;_table.rowHeight=30;_table.dataSource=self;_table.delegate=self;
  _table.allowsEmptySelection=YES;_table.accessibilityLabel=@"Remote keyboard shortcuts";
  NSScrollView* scroll=[[NSScrollView alloc] initWithFrame:NSMakeRect(16,86,448,330)];scroll.hasVerticalScroller=YES;scroll.autohidesScrollers=YES;scroll.borderType=NSBezelBorder;scroll.documentView=_table;[content addSubview:scroll];
  _notice=[NSTextField wrappingLabelWithString:@"No OS is assumed. Choose an action, then Send."];_notice.frame=NSMakeRect(16,44,448,34);_notice.font=[NSFont systemFontOfSize:12];[content addSubview:_notice];
  _send=[NSButton buttonWithTitle:@"Send shortcut" target:self action:@selector(sendShortcut:)];_send.frame=NSMakeRect(16,8,300,28);_send.bezelStyle=NSBezelStyleRounded;_send.enabled=NO;_send.accessibilityIdentifier=@"shortcut-send";[content addSubview:_send];
  _close=[NSButton buttonWithTitle:@"Close" target:self action:@selector(closeFromButton:)];_close.frame=NSMakeRect(328,8,136,28);_close.bezelStyle=NSBezelStyleRounded;[content addSubview:_close];
  auto frame=_panel.frame;const auto ownerFrame=_owner.frame;frame.origin=NSMakePoint(NSMidX(ownerFrame)-frame.size.width/2,NSMidY(ownerFrame)-frame.size.height/2);
  if(NSScreen* screen=_owner.screen){frame.origin.x=std::clamp(frame.origin.x,screen.visibleFrame.origin.x,std::max(screen.visibleFrame.origin.x,NSMaxX(screen.visibleFrame)-frame.size.width));frame.origin.y=std::clamp(frame.origin.y,screen.visibleFrame.origin.y,std::max(screen.visibleFrame.origin.y,NSMaxY(screen.visibleFrame)-frame.size.height));}
  [_panel setFrame:frame display:NO];[_owner addChildWindow:_panel ordered:NSWindowAbove];[_panel makeKeyAndOrderFront:nil];
}
- (void)chooseHost:(id)sender {
  if(sender!=_host || !_session || !_session->current() || (_host.selectedSegment!=0 && _host.selectedSegment!=1))return;
  const auto host=_host.selectedSegment==0 ? ShortcutHost::Windows:ShortcutHost::Mac;
  if(!_session->select(host))return;
  _actions=sessionShortcuts(host);[_table reloadData];[_table deselectAll:nil];_send.enabled=NO;
  _notice.stringValue=@"It affects the active window on the host, not this viewer.";
}
- (void)sendShortcut:(id)sender {
  const auto row=_table.selectedRow;
  if(sender!=_send || !_session || !_session->current() || row<0 || static_cast<std::size_t>(row)>=_actions.size())return;
  _notice.stringValue=_session->send(_actions[row].id) ? @"Shortcut queued. The host/application controls the result.":@"Not queued. Check the connection or try again after input settles.";
}
- (void)closeFromButton:(id)sender {if(sender==_close)[self closePanel];}
- (BOOL)windowShouldClose:(NSWindow*)window {if(window==_panel){[self closePanel];return NO;}return YES;}
- (NSInteger)numberOfRowsInTableView:(NSTableView*)tableView {(void)tableView;return static_cast<NSInteger>(_actions.size());}
- (id)tableView:(NSTableView*)tableView objectValueForTableColumn:(NSTableColumn*)column row:(NSInteger)row {
  (void)tableView;(void)column;if(row<0 || static_cast<std::size_t>(row)>=_actions.size())return @"";
  return [NSString stringWithFormat:@"%@     %@",label(_actions[row].label),label(_actions[row].caption)];
}
- (void)tableViewSelectionDidChange:(NSNotification*)notification {
  if(notification.object==_table)_send.enabled=_session && _session->current() && _table.selectedRow>=0 && static_cast<std::size_t>(_table.selectedRow)<_actions.size();
}
- (void)refresh {
  const BOOL available=_session && _session->available();
  if(_opener.enabled!=available)_opener.enabled=available;
  if(_panel && (!_session || !_session->current()))[self closePanel];
}
- (void)detach {
  [self closePanel];_session=nullptr;_opener.enabled=NO;_opener.target=nil;_owner=nil;
}
- (void)dealloc {[self detach];}
@end
