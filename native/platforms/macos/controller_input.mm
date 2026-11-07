#import "controller_input.h"
#import <GameController/GameController.h>
using namespace sanser::desktop;
@implementation SanserControllerInput
- (NSButton*)button{return _button;}
- (instancetype)initWithOwner:(NSWindow*)owner session:(ControllerSession*)session
 source:(std::function<ControllerSample()>)source focused:(std::function<bool()>)focused {
  if((self=[super init])) {
    _owner=owner;_session=session;_source=std::move(source);_focused=std::move(focused);
    auto* accessory=[[NSTitlebarAccessoryViewController alloc] init];accessory.layoutAttribute=NSLayoutAttributeRight;
    accessory.view=[[NSView alloc] initWithFrame:NSMakeRect(0,0,122,28)];
    _button=[NSButton buttonWithTitle:@"Controller" target:self action:@selector(toggleFromButton:)];
    _button.frame=NSMakeRect(4,2,114,24);_button.refusesFirstResponder=YES;_button.bezelStyle=NSBezelStyleRounded;
    _button.accessibilityIdentifier=@"analog-controller";[accessory.view addSubview:_button];[owner addTitlebarAccessoryViewController:accessory];[self refresh];
  }return self;
}
- (ControllerSample)readController {
  if(_source)return _source();
  NSArray<GCController*>* devices=GCController.controllers;
  if(_device && ![devices containsObject:_device]){_device=nil;return {};}
  if(!_device)for(GCController* device in devices)if(device.extendedGamepad){_device=device;break;}
  GCExtendedGamepad* pad=_device.extendedGamepad;if(!pad)return {};
  ControllerSample state;state.connected=state.sampled=1;
  NSArray* buttons=@[pad.buttonA,pad.buttonB,pad.buttonX,pad.buttonY,pad.leftShoulder,pad.rightShoulder,
    pad.buttonMenu,pad.buttonOptions ?: NSNull.null,pad.leftThumbstickButton ?: NSNull.null,pad.rightThumbstickButton ?: NSNull.null,
    pad.dpad.up,pad.dpad.down,pad.dpad.left,pad.dpad.right];
  for(NSUInteger i=0;i<buttons.count;++i)if([buttons[i] isKindOfClass:GCControllerButtonInput.class] && [(GCControllerButtonInput*)buttons[i] isPressed])state.buttons|=1U<<i;
  state.lx=pad.leftThumbstick.xAxis.value;state.ly=pad.leftThumbstick.yAxis.value;
  state.rx=pad.rightThumbstick.xAxis.value;state.ry=pad.rightThumbstick.yAxis.value;
  state.lt=pad.leftTrigger.value;state.rt=pad.rightTrigger.value;return state;
}
- (void)refresh {
  if(!_session)return;
  _session->refresh(_focused && _focused());
  const auto now=std::chrono::steady_clock::now();
  if(_session->active() && now-_polled>=std::chrono::milliseconds(16)) {
    _polled=now;_session->sample([self readController]);_session->refresh(_focused && _focused());
  }
  _button.enabled=_session->available();_button.state=_session->active()?NSControlStateValueOn:NSControlStateValueOff;
  _button.title=_session->active()?@"Controller on":@"Controller";
  const auto status=_session->status();
  _button.toolTip=!_button.enabled ? @"Analog requires a supporting Windows virtual-controller host.":
    !_session->active() ? @"Enable one USB/Bluetooth controller. Start+Select releases capture. No rumble.":
    !(status&8) ? @"Connect a controller through macOS. Start+Select releases capture.":
    !(status&4) ? @"Center both sticks and release all buttons/triggers to arm.":@"Analog active. Start+Select releases capture. No rumble.";
}
- (void)toggleFromButton:(id)sender {if(sender==_button)[self toggle];}
- (void)toggle {if(_session){_session->refresh(_focused && _focused());_session->toggle();[self refresh];}}
- (void)stop {if(_session)_session->stop();_device=nil;[self refresh];}
- (void)detach {if(_session)_session->stop();_session=nullptr;_button.target=nil;_button.enabled=NO;_device=nil;_source={};_focused={};_owner=nil;}
- (void)dealloc {[self detach];}
@end
