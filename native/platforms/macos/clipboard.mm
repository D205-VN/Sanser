#import <Cocoa/Cocoa.h>
#include "clipboard.h"
#include "text_input.h"
namespace sanser::desktop {
void platformClipboard(bool write,std::string text,std::function<bool()> current,Peer::ClipboardResult completed) {
  dispatch_async(dispatch_get_main_queue(),^{
    @autoreleasepool {
      if(!current()) {completed(5,{});return;}
      NSPasteboard* pasteboard=NSPasteboard.generalPasteboard;
      if(write) {
        if(!sanser::clipboardUtf16(text)) {completed(4,{});return;}
        NSString* value=[[NSString alloc] initWithBytes:text.data() length:text.size() encoding:NSUTF8StringEncoding];
        if(!value) {completed(4,{});return;}
        [pasteboard clearContents];completed([pasteboard setString:value forType:NSPasteboardTypeString] ? 0:3,{});
      } else {
        NSString* value=[pasteboard stringForType:NSPasteboardTypeString];
        if(!value) {completed(3,{});return;}
        if(value.length>sanser::maxClipboardBytes) {completed(4,{});return;}
        NSData* bytes=[value dataUsingEncoding:NSUTF8StringEncoding];
        if(!bytes || bytes.length>sanser::maxClipboardBytes) {completed(4,{});return;}
        if(!bytes.length) {completed(0,{});return;}
        std::string result(static_cast<const char*>(bytes.bytes),bytes.length);
        if(!sanser::clipboardUtf16(result)) {completed(4,{});return;}
        completed(0,std::move(result));
      }
    }
  });
}
}
